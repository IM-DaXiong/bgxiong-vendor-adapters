use std::collections::BTreeMap;
use serde_json::Value;
use crate::{ContractError, ValidatedOperationInput, SchemaRegistry};
use crate::media::MediaFacts;

fn interactions(context: &ValidatedOperationInput) -> Result<Vec<(&'static str, String, &Value)>, ContractError> {
    // Geometry is an input-schema contract, not a UI control requirement.
    let registry = SchemaRegistry::public()?;
    let definitions = registry.schema("https://bgxiong.local/vendor-adapter/v2/interaction-values.schema.json").unwrap();
    let mut found = Vec::new();
    let mut pending = vec![(String::new(), &context.request()["input"])];
    let id = context.operation()["inputSchema"]["id"].as_str().unwrap();
    while let Some((pointer, value)) = pending.pop() {
        let Ok(field) = context.registry.input_field_schema(id, &pointer) else { continue; };
        if let Some(kind) = ["time_span", "point", "box", "mask"].into_iter().find(|kind| field == definitions["$defs"][*kind]) {
            if !value.is_null() { found.push((kind, pointer, value)); }
        } else if let Some(values) = value.as_object() {
            for (key, child) in values {
                pending.push((format!("{}/{}", pointer, key.replace('~', "~0").replace('/', "~1")), child));
            }
        }
    }
    Ok(found)
}

pub fn validate_shapes(context: &ValidatedOperationInput) -> Result<(), ContractError> {
    let registry = SchemaRegistry::public()?;
    let definitions = registry.schema("https://bgxiong.local/vendor-adapter/v2/interaction-values.schema.json").unwrap();
    for (widget, pointer, value) in interactions(context)? {
        let pointer = pointer.as_str();
        let schema = &definitions["$defs"][widget];
        let validator = jsonschema::draft202012::options().build(schema)
            .map_err(|_| ContractError::new("INVALID_PACKAGE", pointer, "public interaction schema cannot compile"))?;
        if !validator.is_valid(value) { return Err(ContractError::new("INVALID_INPUT", pointer, "interaction value has invalid structure")); }
        for key in ["mediaId", "maskMediaId"] {
            if let Some(id) = value.get(key) {
                if !context.request()["media"].as_array().unwrap().iter().any(|m| &m["id"] == id) {
                    return Err(ContractError::new("INVALID_INPUT", pointer, "interaction references media outside the request"));
                }
            }
        }
    }
    Ok(())
}

pub fn validate_bounds(context: &ValidatedOperationInput, facts: &BTreeMap<String, MediaFacts>) -> Result<(), ContractError> {
    validate_shapes(context)?;
    for (widget, pointer, value) in interactions(context)? {
        let pointer = pointer.as_str();
        let source = facts.get(value["mediaId"].as_str().unwrap()).ok_or_else(|| ContractError::new("RESOURCE_UNAVAILABLE", pointer, "source has not been probed by the host"))?;
        let invalid = || ContractError::new("INVALID_INPUT", pointer, "selection is outside the probed source media");
        if widget == "time_span" {
            let duration = source.duration_ms.ok_or_else(|| ContractError::new("RESOURCE_UNAVAILABLE", pointer, "source duration is unavailable"))?;
            let start = value["startMs"].as_u64().ok_or_else(invalid)?;
            let end = value["endMs"].as_u64().ok_or_else(invalid)?;
            if start >= end || end > duration { return Err(invalid()); }
            // Deliberately no fps: VFR uses source timestamps and millisecond intervals.
            continue;
        }
        let (width, height) = source.width.zip(source.height).filter(|(w,h)| *w > 0 && *h > 0)
            .ok_or_else(|| ContractError::new("RESOURCE_UNAVAILABLE", pointer, "probed source dimensions are unavailable"))?;
        if widget == "mask" {
            let mask = facts.get(value["maskMediaId"].as_str().unwrap()).ok_or_else(invalid)?;
            if mask.kind != "image" || mask.width != Some(width) || mask.height != Some(height) { return Err(invalid()); }
        } else {
            let x = value["x"].as_f64().ok_or_else(invalid)?;
            let y = value["y"].as_f64().ok_or_else(invalid)?;
            if x < 0.0 || y < 0.0 || x >= f64::from(width) || y >= f64::from(height) { return Err(invalid()); }
            if widget == "box" {
                let w = value["width"].as_f64().ok_or_else(invalid)?;
                let h = value["height"].as_f64().ok_or_else(invalid)?;
                if w <= 0.0 || h <= 0.0 || x + w > f64::from(width) || y + h > f64::from(height) { return Err(invalid()); }
            }
        }
    }
    Ok(())
}
