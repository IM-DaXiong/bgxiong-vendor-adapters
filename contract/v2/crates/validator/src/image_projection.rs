//! Negotiated, platform-neutral image projection. No geometry guesses or I/O.
use crate::{ContractError, SchemaRegistry, ValidatedOperationInput};
use serde_json::Value;
use std::collections::BTreeSet;

pub const FEATURE: &str = "image-projection-v1";
const OUTPUT: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/image-projection-v1/output.schema.json";
fn invalid(path: &str, reason: &str) -> ContractError {
    ContractError::new("INVALID_INPUT", path, reason)
}
pub fn validate_contract(contract: &Value) -> Result<(), ContractError> {
    if contract["capabilityKey"] != "image.generate" { return Ok(()); }
    let registry = crate::input::registry(contract)?;
    for operation in contract["operations"].as_array().unwrap() {
        let id = operation["inputSchema"]["id"].as_str().unwrap();
        let root = registry.schema(id).unwrap();
        let field = registry.input_field_schema(id, "/outputProjection")?;
        let values = field["enum"].as_array().ok_or_else(|| invalid("/outputProjection", "projection enum is required"))?;
        if field["type"] != "string" || values.is_empty()
            || values.iter().any(|v| v != "flat" && v != "equirectangular360")
            || values.iter().filter_map(Value::as_str).collect::<BTreeSet<_>>().len() != values.len()
            || !values.contains(&field["default"])
            || !root["required"].as_array().is_some_and(|a| a.iter().any(|v| v == "outputProjection"))
            || operation["execution"]["streaming"] != false
        {
            return Err(invalid("/outputProjection", "projection requires a closed enum, valid default, required input and non-streaming operation"));
        }
        // Validate actual result data against the public profile as well as the
        // author's exact output schema. Never replace the declared digest.
        let images = operation["outputSlots"].as_array().unwrap().iter().filter(|s| s["kind"] == "image");
        if !images.clone().any(|s| s["minCount"].as_u64().unwrap_or(0) > 0) {
            return Err(invalid("/outputSlots", "image projection success requires an image output"));
        }
    }
    Ok(())
}

/// Only the verified package negotiation can enable this profile in a host.
pub fn enable(mut context: ValidatedOperationInput, features: &BTreeSet<String>) -> Result<ValidatedOperationInput, ContractError> {
    if features.contains(FEATURE) && context.contract()["capabilityKey"] == "image.generate" {
        validate_contract(context.contract())?;
        let projection = &context.request()["input"]["outputProjection"];
        if projection != "flat" && projection != "equirectangular360" {
            return Err(invalid("/input/outputProjection", "projection choice is required"));
        }
        context.image_projection = true;
    }
    Ok(context)
}

pub fn validate_result(context: &ValidatedOperationInput, result: &Value) -> Result<(), ContractError> {
    if !context.image_projection { return Ok(()); }
    if result["kind"] == "accepted" { return Ok(()); }
    SchemaRegistry::public()?.validate(OUTPUT, &result["data"])?;
    let entries = result["data"]["imageArtifacts"].as_array().unwrap();
    let outputs = result["outputs"].as_array().ok_or_else(|| invalid("/outputs", "outputs are required"))?;
    let images: Vec<_> = outputs.iter().filter(|o| o["kind"] == "media" && o["media"]["kind"] == "image").collect();
    let success = result["kind"] == "completed" || result["status"] == "succeeded";
    if !success {
        if !entries.is_empty() || !images.is_empty() { return Err(invalid("/data/imageArtifacts", "non-success cannot publish images")); }
        return Ok(());
    }
    if images.is_empty() || entries.len() != images.len() { return Err(invalid("/data/imageArtifacts", "each successful image needs exactly one projection")); }
    let mut seen = BTreeSet::new();
    for entry in entries {
        if !seen.insert(entry["mediaId"].as_str().unwrap())
            || entry["projection"] != context.request()["input"]["outputProjection"]
            || !images.iter().any(|o| o["media"]["id"] == entry["mediaId"] && o["role"] == entry["role"])
        { return Err(invalid("/data/imageArtifacts", "projection identity, role or request choice differs")); }
    }
    Ok(())
}

pub fn validate_geometry(context: &ValidatedOperationInput, width: Option<u32>, height: Option<u32>) -> Result<(), ContractError> {
    if context.image_projection && context.request()["input"]["outputProjection"] == "equirectangular360"
        && !height.is_some_and(|h| h > 0 && h.checked_mul(2) == width) {
        return Err(invalid("/outputs/media", "equirectangular image requires decoded width exactly twice height"));
    }
    Ok(())
}
