use serde::{Serialize, Deserialize};
use serde_json::Value;
use crate::{ContractError, hash_bytes};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageIdentity {
    pub plugin_id: String,
    pub version: String,
    pub digest: String,
}

pub(crate) fn protocol() -> Value {
    serde_json::from_str(include_str!("../../../protocol/vendor-adapter-protocol.json")).expect("published protocol JSON")
}

impl PackageIdentity {
    pub(crate) fn from_verified(manifest: &Value, inventory: &[u8]) -> Self {
        Self { plugin_id: manifest["pluginId"].as_str().unwrap().into(),
               version: manifest["version"].as_str().unwrap().into(), digest: hash_bytes(inventory) }
    }
}

pub fn validate_operation_namespace(namespace: &str, id: &str) -> Result<(), ContractError> {
    let (owner, name) = id.split_once('/').ok_or_else(|| ContractError::new("INVALID_PACKAGE", "/operations/id", "operation requires a publisher namespace"))?;
    if owner == "std" {
        if !protocol()["standardProfiles"].as_array().unwrap().iter().any(|p| p["operationId"] == id) {
            return Err(ContractError::new("INVALID_PACKAGE", id, "standard operation profile has not been published"));
        }
    } else if owner != namespace || name.is_empty() || name.contains('/') {
        return Err(ContractError::new("UNAUTHORIZED", id, "operation namespace does not match verified publisher"));
    }
    Ok(())
}

pub fn validate_capability_namespace(namespace: &str, key: &str) -> Result<(), ContractError> {
    if protocol()["standardCapabilityKeys"].as_array().unwrap().iter().any(|v| v.as_str() == Some(key)) { return Ok(()); }
    validate_operation_namespace(namespace, key)
}
