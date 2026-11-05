use serde_json::Value;
use crate::schema_registry::SchemaRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError {
    pub code: &'static str,
    pub path: String,
    pub reason: String,
}

impl ContractError {
    pub fn new(code: &'static str, path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self { code, path: path.into(), reason: reason.into() }
    }
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}: {}", self.code, self.path, self.reason)
    }
}
impl std::error::Error for ContractError {}

/// Immutable snapshot; only successful validation can construct one.
pub struct ValidatedOperationInput {
    pub(crate) contract: Value,
    pub(crate) operation: Value,
    pub(crate) request: Value,
    pub(crate) registry: SchemaRegistry,
    pub(crate) transport_operation: String,
    pub(crate) contract_digest: String,
}

impl ValidatedOperationInput {
    pub fn request(&self) -> &Value { &self.request }
    pub fn contract(&self) -> &Value { &self.contract }
    pub fn operation(&self) -> &Value { &self.operation }
    pub fn contract_digest(&self) -> &str { &self.contract_digest }
    pub fn transport_operation(&self) -> &str { &self.transport_operation }
}
