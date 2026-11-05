use crate::{ContractError, SchemaRegistry, ValidatedOperationInput};
use bgx_adapter_v2_sdk::{Invocation, OperationResultV2, OperationTaskRefV2, Response};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub fn validate_task_input(
    original: &ValidatedOperationInput,
    task: &OperationTaskRefV2,
    operation: &str,
    accepted_vendor_task_id: &str,
) -> Result<ValidatedOperationInput, ContractError> {
    if !matches!(operation, "query" | "cancel") || accepted_vendor_task_id.is_empty() {
        return Err(ContractError::new(
            "INVALID_INPUT",
            "/operation",
            "query/cancel requires the original accepted task identity",
        ));
    }
    let task = crate::input::as_value(task)?;
    original
        .registry
        .validate_dispatch(operation, "input", &task)?;
    for key in [
        "modelId",
        "capabilityKey",
        "operationId",
        "operationVersion",
        "configuration",
    ] {
        if task[key] != original.request[key] {
            return Err(ContractError::new(
                "INVALID_INPUT",
                key,
                "task context differs from frozen submission",
            ));
        }
    }
    if task["vendorTaskId"] != accepted_vendor_task_id {
        return Err(ContractError::new(
            "INVALID_INPUT",
            "/vendorTaskId",
            "cannot query a different remote task",
        ));
    }
    if original.operation["execution"]["mode"] != "asynchronous" {
        return Err(ContractError::new(
            "INVALID_INPUT",
            "/operation",
            "synchronous work has no recoverable asynchronous task",
        ));
    }
    if let Some(id) = original.operation["resumeSchema"]
        .get("id")
        .and_then(Value::as_str)
    {
        original.registry.validate(id, &task["resumeToken"])?;
    } else if !task["resumeToken"].is_null() {
        return Err(ContractError::new(
            "INVALID_INPUT",
            "/resumeToken",
            "resume schema was not declared",
        ));
    }
    Ok(ValidatedOperationInput {
        contract: original.contract.clone(),
        operation: original.operation.clone(),
        request: original.request.clone(),
        registry: crate::input::registry(&original.contract)?,
        transport_operation: operation.to_owned(),
        contract_digest: original.contract_digest.clone(),
    })
}

pub fn validate_output_slots(
    context: &ValidatedOperationInput,
    result: &Value,
) -> Result<(), ContractError> {
    declared_output_slots(context.operation(), result)
}

fn declared_output_slots(operation: &Value, result: &Value) -> Result<(), ContractError> {
    let Some(outputs) = result.get("outputs").and_then(Value::as_array) else {
        return Ok(());
    };
    let slots = operation["outputSlots"].as_array().unwrap();
    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    let mut media_ids = BTreeSet::new();
    for output in outputs {
        let role = output["role"].as_str().unwrap();
        let slot = slots.iter().find(|s| s["role"] == role).ok_or_else(|| {
            ContractError::new(
                "INVALID_INPUT",
                "/outputs/role",
                "output role was not declared",
            )
        })?;
        let kind = if output["kind"] == "text" {
            "text"
        } else {
            output["media"]["kind"].as_str().unwrap()
        };
        if slot["kind"] != kind {
            return Err(ContractError::new(
                "INVALID_INPUT",
                "/outputs",
                "output kind does not match its declared role",
            ));
        }
        if output["kind"] == "media" {
            let media = &output["media"];
            if !crate::media::valid_basename(media["fileName"].as_str().unwrap())
                || media["byteLength"]
                    .as_str()
                    .unwrap()
                    .parse::<u64>()
                    .is_err()
                || !media_ids.insert(media["id"].as_str().unwrap())
            {
                return Err(ContractError::new(
                    "INVALID_INPUT",
                    "/outputs/media",
                    "output media identity, name or length invalid",
                ));
            }
        }
        *counts.entry(role).or_default() += 1;
    }
    let complete = result["kind"] == "completed" || result["status"] == "succeeded";
    for slot in slots {
        let count = counts
            .get(slot["role"].as_str().unwrap())
            .copied()
            .unwrap_or(0);
        if (complete && count < slot["minCount"].as_u64().unwrap())
            || slot["maxCount"].as_u64().is_some_and(|m| count > m)
        {
            return Err(ContractError::new(
                "INVALID_INPUT",
                "/outputs",
                "output count outside declared bounds",
            ));
        }
    }
    Ok(())
}

/// Validate the install-time result witness using the exact declaration rules
/// used for real responses. Dynamic appliedParams and leased media still need
/// the frozen runtime context; this function never claims those are proven.
pub fn validate_declared_result(
    contract: &bgx_adapter_v2_sdk::CapabilityContractV2,
    operation_id: &str,
    operation_version: u64,
    transport: &str,
    result: &OperationResultV2,
) -> Result<(), ContractError> {
    crate::validate_contract(contract)?;
    let declaration = crate::input::as_value(contract)?;
    let operation = declaration["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["id"] == operation_id && op["version"] == operation_version)
        .ok_or_else(|| {
            ContractError::new(
                "INVALID_PACKAGE",
                "/operationId",
                "unknown operation declaration",
            )
        })?;
    let registry = crate::input::registry(&declaration)?;
    declared_result(
        &registry,
        operation,
        transport,
        &crate::input::as_value(result)?,
    )
}

fn declared_result(
    registry: &SchemaRegistry,
    operation: &Value,
    transport: &str,
    value: &Value,
) -> Result<(), ContractError> {
    if !matches!(transport, "submit" | "query") {
        return Err(ContractError::new(
            "INVALID_INPUT",
            "/operation",
            "result declaration requires submit or query",
        ));
    }
    registry.validate_dispatch(transport, "result", value)?;
    if value["kind"] == "accepted" && operation["execution"]["mode"] != "asynchronous" {
        return Err(ContractError::new(
            "INVALID_INPUT",
            "/kind",
            "synchronous operation cannot accept a durable task",
        ));
    }
    if value["kind"] == "completed"
        || value["status"] == "succeeded"
        || value.get("data").is_some_and(|v| !v.is_null())
    {
        registry.validate(
            operation["outputSchema"]["id"].as_str().unwrap(),
            &value["data"],
        )?;
    }
    if let Some(reference) = operation["resumeSchema"].get("id").and_then(Value::as_str) {
        if let Some(token) = value.get("resumeToken") {
            registry.validate(reference, token)?;
        }
    } else if value.get("resumeToken").is_some_and(|v| !v.is_null()) {
        return Err(ContractError::new(
            "INVALID_INPUT",
            "/resumeToken",
            "resume token was not declared",
        ));
    }
    declared_output_slots(operation, value)?;
    let namespace = operation["id"]
        .as_str()
        .unwrap()
        .split_once('/')
        .map(|(owner, _)| owner)
        .ok_or_else(|| {
            ContractError::new(
                "INVALID_PACKAGE",
                "/operationId",
                "operation namespace missing",
            )
        })?;
    crate::usage::validate_usage(&value["usage"], namespace)?;
    if let Some(failure) = value.get("terminalFailure").filter(|v| !v.is_null()) {
        let budget = crate::schema_registry::policy()["limits"]["maxDiagnosticBytes"]
            .as_u64()
            .unwrap();
        if serde_json::to_vec(failure).unwrap().len() as u64 > budget {
            return Err(ContractError::new(
                "INVALID_INPUT",
                "/terminalFailure",
                "diagnostic exceeds public resource budget",
            ));
        }
    }
    Ok(())
}

pub fn validate_result(
    context: &ValidatedOperationInput,
    result: &OperationResultV2,
) -> Result<(), ContractError> {
    let value = crate::input::as_value(result)?;
    declared_result(
        &context.registry,
        context.operation(),
        context.transport_operation(),
        &value,
    )?;
    if let Some(applied) = value.get("appliedParams").filter(|v| !v.is_null()) {
        let mut merged = context.request()["input"].clone();
        for (key, actual) in applied.as_object().unwrap() {
            if context.request()["input"].get(key).is_none() {
                return Err(ContractError::new(
                    "INVALID_INPUT",
                    "/appliedParams",
                    "reported parameter was absent from frozen input",
                ));
            }
            merged[key] = actual.clone();
        }
        context.registry.validate(
            context.operation()["inputSchema"]["id"].as_str().unwrap(),
            &merged,
        )?;
    }
    Ok(())
}

pub fn validate_response(
    invocation: &Invocation,
    response: &Response,
    context: Option<&ValidatedOperationInput>,
) -> Result<(), ContractError> {
    let i = crate::input::as_value(invocation)?;
    let r = crate::input::as_value(response)?;
    let registry = SchemaRegistry::public()?;
    registry.validate(
        "https://bgxiong.local/vendor-adapter/v2/invocation.schema.json",
        &i,
    )?;
    registry.validate(
        "https://bgxiong.local/vendor-adapter/v2/response.schema.json",
        &r,
    )?;
    if i["operation"] != r["operation"] || i["profileId"] != i["bindingId"] {
        return Err(ContractError::new(
            "INVALID_INPUT",
            "/operation",
            "response operation or binding identity differs",
        ));
    }
    if r.get("error").is_some() {
        return Ok(());
    }
    let data: Value = serde_json::from_str(r["dataJson"].as_str().unwrap()).map_err(|_| {
        ContractError::new("INVALID_INPUT", "/dataJson", "response data is not JSON")
    })?;
    registry.validate_dispatch(i["operation"].as_str().unwrap(), "result", &data)?;
    if matches!(i["operation"].as_str(), Some("submit" | "query")) {
        let context = context.ok_or_else(|| {
            ContractError::new(
                "INVALID_INPUT",
                "/context",
                "submit/query requires frozen validation context",
            )
        })?;
        if context.transport_operation() != i["operation"].as_str().unwrap() {
            return Err(ContractError::new(
                "INVALID_INPUT",
                "/context",
                "transport operation differs from frozen context",
            ));
        }
        let result = serde_json::from_value(data).map_err(|_| {
            ContractError::new(
                "INVALID_INPUT",
                "/dataJson",
                "invalid typed operation result",
            )
        })?;
        validate_result(context, &result)?;
    } else if i["operation"] == "probe" {
        let request: Value =
            serde_json::from_str(i["payloadJson"].as_str().unwrap()).map_err(|_| {
                ContractError::new("INVALID_INPUT", "/payloadJson", "invalid probe payload")
            })?;
        if request["kind"] == "package_validation"
            && ["connection", "authentication", "resources"]
                .iter()
                .any(|k| data[*k] != "not_checked")
        {
            return Err(ContractError::new(
                "INVALID_INPUT",
                "/probe",
                "package validation cannot claim connectivity",
            ));
        }
        if ["connection", "authentication", "resources"]
            .iter()
            .any(|k| data[*k] == "failed")
            && data["issues"].as_array().unwrap().is_empty()
        {
            return Err(ContractError::new(
                "INVALID_INPUT",
                "/issues",
                "failed readiness requires structured issues",
            ));
        }
    }
    Ok(())
}

/// Delivered role/index identities cannot be replaced by later partial snapshots.
#[derive(Default)]
pub struct PartialOutputLedger {
    values: BTreeMap<(String, usize), Value>,
}
impl PartialOutputLedger {
    pub fn observe(&mut self, outputs: &[Value]) -> Result<(), ContractError> {
        let mut indices = BTreeMap::<String, usize>::new();
        let mut additions = Vec::new();
        for output in outputs {
            let role = output["role"]
                .as_str()
                .ok_or_else(|| ContractError::new("INVALID_INPUT", "/role", "output role missing"))?
                .to_owned();
            let index = indices.entry(role.clone()).or_default();
            let key = (role, *index);
            *index += 1;
            let identity = |value: &Value| {
                let mut stable = value.clone();
                if stable["kind"] == "media" {
                    if let Some(m) = stable["media"].as_object_mut() {
                        m.remove("handle");
                    }
                }
                stable
            };
            if self
                .values
                .get(&key)
                .is_some_and(|prior| identity(prior) != identity(output))
            {
                return Err(ContractError::new(
                    "INVALID_INPUT",
                    "/outputs",
                    "delivered role/index output was replaced",
                ));
            }
            additions.push((key, output.clone()));
        }
        self.values.extend(additions);
        Ok(())
    }
    pub fn outputs(&self) -> Vec<Value> {
        self.values.values().cloned().collect()
    }
}
