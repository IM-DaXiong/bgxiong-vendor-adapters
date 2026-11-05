use std::collections::BTreeMap;
use serde::{Serialize, Deserialize};
use serde_json::{Value, json};
use bgx_adapter_v2_sdk::{CapabilityContractV2, OperationSubmitV2, Invocation};
use crate::{ContractError, SchemaRegistry, ValidatedOperationInput, hash_bytes, validate_contract, validate_input};
use crate::identity::{PackageIdentity, validate_capability_namespace, validate_operation_namespace};
use crate::package::VerifiedPackage;

/// Host-owned immutable binding snapshot, never supplied by a guest.
pub struct Binding {
    id: String,
    package: PackageIdentity,
    revision: u64,
    configuration: Value,
    configuration_digest: String,
}
impl Binding {
    pub fn new(id: String, package: &VerifiedPackage, revision: u64, configuration: Value) -> Result<Self, ContractError> {
        if id.is_empty() || !configuration.is_object() { return Err(ContractError::new("INVALID_INPUT", "/binding", "explicit binding and nonsecret configuration required")); }
        let configuration_digest = hash_bytes(&serde_json::to_vec(&configuration).unwrap());
        Ok(Self { id, package: package.identity().clone(), revision, configuration, configuration_digest })
    }
    pub fn id(&self) -> &str { &self.id }
    pub fn configuration(&self) -> &Value { &self.configuration }
    pub fn revision(&self) -> u64 { self.revision }
    pub fn package(&self) -> &PackageIdentity { &self.package }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub struct CapabilityRef {
    pub capability_id: String,
    pub package: PackageIdentity,
    pub binding_id: String,
    pub binding_revision: u64,
    pub configuration_digest: String,
    pub model_id: String,
    pub capability_key: String,
    pub operation_id: String,
    pub operation_version: u64,
    pub input_schema_digest: String,
    pub contract_digest: String,
}

#[derive(Clone)]
struct Entry { reference: CapabilityRef, contract: CapabilityContractV2, readiness: Value, enabled_features: std::collections::BTreeSet<String>, configuration_valid:bool }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub struct CapabilityView {
    pub reference: CapabilityRef,
    pub supported: bool,
    pub environment_ready: bool,
    pub readiness: Value,
}

#[derive(Default, Clone)]
pub struct CapabilityRegistry {
    entries: BTreeMap<String, Entry>,
    operation_schemas: BTreeMap<(String, u64), Value>,
}

impl CapabilityRegistry {
    /// Atomic catalog refresh retains the operation-version schema ledger.
    pub fn refresh(&mut self, package: &VerifiedPackage, binding: &Binding,
                   contracts: &[CapabilityContractV2]) -> Result<Vec<CapabilityRef>, ContractError> {
        let mut candidate = self.clone();
        candidate.remove_binding(binding.id());
        let references = candidate.register(package,binding,contracts)?;
        *self = candidate;
        Ok(references)
    }
    pub fn register(&mut self, package: &VerifiedPackage, binding: &Binding,
                    contracts: &[CapabilityContractV2]) -> Result<Vec<CapabilityRef>, ContractError> {
        if binding.package != *package.identity() { return Err(ContractError::new("INVALID_INPUT", "/binding", "binding package version differs")); }
        let mut candidate_schemas = self.operation_schemas.clone();
        let mut additions = BTreeMap::new();
        for contract in contracts {
            validate_contract(contract)?;
            let c = serde_json::to_value(contract).unwrap();
            let registry = crate::input::registry(&c)?;
            let configuration_valid=registry.validate(c["configurationSchema"]["id"].as_str().unwrap(), &binding.configuration).is_ok();
            validate_capability_namespace(package.namespace(), c["capabilityKey"].as_str().unwrap())?;
            for operation in c["operations"].as_array().unwrap() {
                let id = operation["id"].as_str().unwrap();
                let version = operation["version"].as_u64().unwrap();
                validate_operation_namespace(package.namespace(), id)?;
                let signature = json!([operation["inputSchema"]["digest"], operation["outputSchema"]["digest"],
                    operation["resumeSchema"].get("digest")]);
                let key = (id.to_owned(), version);
                if candidate_schemas.get(&key).is_some_and(|s| s != &signature) {
                    return Err(ContractError::new("INVALID_PACKAGE", id, "operation version already has different schema digests"));
                }
                candidate_schemas.insert(key, signature);
                for model in c["modelIds"].as_array().unwrap() {
                    let identity = json!([package.identity(), binding.id, c["localId"], model, c["capabilityKey"], id, version]);
                    let capability_id = hash_bytes(&serde_json::to_vec(&identity).unwrap());
                    let reference = CapabilityRef { capability_id: capability_id.clone(), package: package.identity().clone(),
                        binding_id: binding.id.clone(), binding_revision: binding.revision,
                        configuration_digest: binding.configuration_digest.clone(), model_id: model.as_str().unwrap().into(),
                        capability_key: c["capabilityKey"].as_str().unwrap().into(), operation_id: id.into(), operation_version: version,
                        input_schema_digest: operation["inputSchema"]["digest"].as_str().unwrap().into(),
                        contract_digest: hash_bytes(&serde_json::to_vec(&c).unwrap()) };
                    if self.entries.contains_key(&capability_id) || additions.contains_key(&capability_id) {
                        return Err(ContractError::new("INVALID_PACKAGE", "/localId", "duplicate capability identity"));
                    }
                    additions.insert(capability_id, Entry { reference, contract: contract.clone(), enabled_features: package.enabled_features().clone(), configuration_valid, readiness: json!({
                        "packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":if configuration_valid{json!([])}else{json!([{"code":"INVALID_CONFIGURATION","message":"Complete the binding configuration using the declared configuration schema"}])}}) });
                }
            }
        }
        let references = additions.values().map(|e| e.reference.clone()).collect();
        self.entries.extend(additions);
        self.operation_schemas = candidate_schemas;
        Ok(references)
    }

    pub fn list(&self, key: Option<&str>) -> Vec<CapabilityView> {
        self.entries.values().filter(|e| key.is_none_or(|key| key == e.reference.capability_key)).map(|e| CapabilityView {
            reference: e.reference.clone(), supported: true,
            environment_ready: e.configuration_valid && ["connection", "authentication", "resources"].iter().all(|key| e.readiness[*key] == "ready"),
            readiness: e.readiness.clone(),
        }).collect()
    }
    pub fn contract_for(&self, reference: &CapabilityRef, binding: &Binding) -> Result<&CapabilityContractV2,ContractError> {
        let entry = self.entries.get(&reference.capability_id).ok_or_else(||ContractError::new("INVALID_INPUT","/capabilityId","unknown capability"))?;
        if entry.reference != *reference || reference.package != binding.package || reference.binding_id != binding.id
            || reference.binding_revision != binding.revision || reference.configuration_digest != binding.configuration_digest {
            return Err(ContractError::new("INVALID_INPUT","/binding","binding or capability changed"));
        }
        Ok(&entry.contract)
    }

    pub fn set_readiness(&mut self, reference: &CapabilityRef, report: Value) -> Result<(), ContractError> {
        SchemaRegistry::public()?.validate_dispatch("probe", "result", &report)?;
        if report["packageValid"] != true { return Err(ContractError::new("INVALID_PACKAGE", "/readiness", "invalid package cannot be ready")); }
        if ["connection", "authentication", "resources"].iter().any(|key| report[*key] == "failed") && report["issues"].as_array().unwrap().is_empty() {
            return Err(ContractError::new("INVALID_INPUT", "/issues", "failed readiness requires structured issues"));
        }
        let entry = self.entries.get_mut(&reference.capability_id).ok_or_else(|| ContractError::new("INVALID_INPUT", "/capabilityId", "unknown capability"))?;
        if &entry.reference != reference { return Err(ContractError::new("INVALID_INPUT", "/capabilityId", "capability reference changed")); }
        if !entry.configuration_valid { return Err(ContractError::new("INVALID_INPUT","/configuration","binding configuration must pass its declared schema before readiness can be updated")); }
        entry.readiness = report;
        Ok(())
    }

    pub fn validate_submission(&self, reference: &CapabilityRef, binding: &Binding,
                               invocation: &Invocation, request: &OperationSubmitV2) -> Result<ValidatedOperationInput, ContractError> {
        SchemaRegistry::validate_public("invocation", &serde_json::to_value(invocation).unwrap())?;
        let entry = self.entries.get(&reference.capability_id).ok_or_else(|| ContractError::new("INVALID_INPUT", "/capabilityId", "unknown capability"))?;
        if entry.reference != *reference || reference.package != binding.package || reference.binding_id != binding.id
            || reference.binding_revision != binding.revision || reference.configuration_digest != binding.configuration_digest {
            return Err(ContractError::new("INVALID_INPUT", "/binding", "binding or capability version has changed"));
        }
        let i = serde_json::to_value(invocation).unwrap();
        let r = serde_json::to_value(request).unwrap();
        let enabled: std::collections::BTreeSet<_> = i["enabledFeatures"].as_array().unwrap().iter().map(|f|f.as_str().unwrap().to_owned()).collect();
        if enabled != entry.enabled_features {
            return Err(ContractError::new("UNSUPPORTED_FEATURE", "/enabledFeatures", "feature was not negotiated for this package and host"));
        }
        if i["profileId"] != binding.id || i["bindingId"] != binding.id || i["operation"] != "submit"
            || i["capabilitySlot"] != serde_json::to_value(&entry.contract).unwrap()["transportSlot"]
            || r["modelId"] != reference.model_id || r["operationId"] != reference.operation_id
            || r["operationVersion"] != reference.operation_version || r["configuration"] != binding.configuration {
            return Err(ContractError::new("INVALID_INPUT", "/binding", "invocation does not match the selected capability and binding"));
        }
        if i["payloadJson"].as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()).as_ref() != Some(&r) {
            return Err(ContractError::new("INVALID_INPUT", "/payloadJson", "invocation payload differs from frozen input"));
        }
        validate_input(&entry.contract, request)
    }

    /// A binding replacement explicitly retires its old catalog entries; no fallback.
    pub fn remove_binding(&mut self, binding_id: &str) {
        self.entries.retain(|_, entry| entry.reference.binding_id != binding_id);
    }
}
