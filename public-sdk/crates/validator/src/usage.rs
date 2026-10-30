use std::collections::BTreeSet;
use serde_json::Value;
use crate::{ContractError, SchemaRegistry};

pub fn validate_usage(value: &Value, namespace: &str) -> Result<(), ContractError> {
    if value.is_null() { return Ok(()); }
    SchemaRegistry::validate_public("usage", value)?;
    let protocol = crate::identity::protocol();
    let mut measurements = BTreeSet::new();
    for measurement in value["measurements"].as_array().unwrap() {
        for (field, list) in [("metric", "metrics"), ("unit", "units")] {
            let name = measurement[field].as_str().unwrap();
            if !protocol["usageNames"][list].as_array().unwrap().iter().any(|v| v == name)
                && !name.split_once('/').is_some_and(|(owner, key)| owner == namespace && !key.is_empty() && !key.contains('/')) {
                return Err(ContractError::new("INVALID_INPUT", field, "usage name is neither public nor in the verified publisher namespace"));
            }
        }
        if !measurements.insert((measurement["metric"].as_str().unwrap(), measurement["unit"].as_str().unwrap())) {
            return Err(ContractError::new("INVALID_INPUT", "/usage", "duplicate measurement identity in a usage snapshot"));
        }
    }
    Ok(())
}

/// Provider-reported cumulative snapshots replace each other; never sum snapshots.
#[derive(Default)]
pub struct UsageSnapshot { current: Option<Value> }
impl UsageSnapshot {
    pub fn replace(&mut self, value: &Value, namespace: &str) -> Result<(), ContractError> {
        validate_usage(value, namespace)?;
        if !value.is_null() { self.current = Some(value.clone()); }
        Ok(())
    }
    pub fn current(&self) -> Option<&Value> { self.current.as_ref() }
}
