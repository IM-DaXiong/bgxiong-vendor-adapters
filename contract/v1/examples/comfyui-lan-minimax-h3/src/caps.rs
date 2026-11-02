//! Vendor-neutral RCD: one video slot document per modelId.

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use bgx_vendor_adapter_sdk::{ParamBinding, PromptProjection, RcdParam};
use serde_json::{json, Map, Value};

use crate::config::{
    ASPECT_BINDING_ID, DURATION_NODE_ID, FPS_NODE_ID, MODEL_ID, MODEL_ID_I2V_TURBO,
    MODEL_ID_R2V_TURBO, PLUGIN_ID, PLUGIN_VERSION, R2V_DURATION_NODE_ID, RESOLUTION_BINDING_ID,
};

/// Prompt projection declared by every model of this plugin: the host's MiniMax H3 profile
/// (`h3` version 1, method package `h3-prompt-writing`). Declared explicitly here, per model,
/// with that model's own implemented modes; the host never infers it from ids or names.
pub const PROMPT_PROJECTION_PROFILE_ID: &str = "h3";
pub const PROMPT_PROJECTION_VERSION: u32 = 1;
pub const PROMPT_PROJECTION_METHOD_NAME: &str = "h3-prompt-writing";

fn prompt_projection(implemented: &[&str]) -> Result<Value, String> {
    let projection = PromptProjection::new(
        PROMPT_PROJECTION_PROFILE_ID,
        PROMPT_PROJECTION_VERSION,
        PROMPT_PROJECTION_METHOD_NAME,
        implemented.iter().map(|m| String::from(*m)).collect(),
    )?;
    projection.validate_for_slot("video", implemented)?;
    Ok(projection.to_json())
}

fn bound(id: &str) -> Option<ParamBinding> {
    ParamBinding::new(id).ok()
}

fn insert_param(slot: &mut Map<String, Value>, key: &str, p: RcdParam) -> Result<(), String> {
    let v = serde_json::to_value(p).map_err(|e| format!("rcd {key}: {e}"))?;
    slot.insert(key.into(), v);
    Ok(())
}

fn timing_params(slot: &mut Map<String, Value>, duration_node: &str) -> Result<(), String> {
    if let Some(b) = bound(duration_node) {
        insert_param(
            slot,
            "duration",
            RcdParam::range(1.0, 15.0, 1.0, 5.0, b),
        )?;
    }
    if let Some(b) = bound(FPS_NODE_ID) {
        insert_param(
            slot,
            "fps",
            RcdParam::enum_of(
                vec![
                    json!({"id": "24", "label": "24", "value": 24}),
                    json!({"id": "25", "label": "25", "value": 25}),
                    json!({"id": "30", "label": "30", "value": 30}),
                ],
                json!(24),
                b,
            ),
        )?;
    }
    if let Some(b) = bound(ASPECT_BINDING_ID) {
        insert_param(
            slot,
            "aspect",
            RcdParam::enum_of(
                vec![
                    json!({"id": "1:1", "label": "1:1", "value": "1:1"}),
                    json!({"id": "16:9", "label": "16:9", "value": "16:9"}),
                    json!({"id": "9:16", "label": "9:16", "value": "9:16"}),
                    json!({"id": "4:3", "label": "4:3", "value": "4:3"}),
                ],
                json!("16:9"),
                b,
            ),
        )?;
    }
    if let Some(b) = bound(RESOLUTION_BINDING_ID) {
        insert_param(
            slot,
            "resolution",
            RcdParam::enum_of(
                vec![
                    json!({"id": "mp:0.25", "label": "0.25 MP", "value": 0.25}),
                    json!({"id": "mp:0.4", "label": "0.4 MP", "value": 0.4}),
                    json!({"id": "mp:0.6", "label": "0.6 MP", "value": 0.6}),
                    json!({"id": "mp:1", "label": "1 MP", "value": 1.0}),
                ],
                json!("mp:0.4"),
                b,
            ),
        )?;
    }
    Ok(())
}

fn video_slot(
    model_id: &str,
    duration_node: &str,
    max_refs: u64,
    implemented: Vec<&str>,
) -> Result<Value, String> {
    let mut slot = Map::new();
    slot.insert("slot".into(), json!("video"));
    slot.insert("modelId".into(), json!(model_id));
    slot.insert("maxReferenceImages".into(), json!(max_refs));
    slot.insert("supportsFirstLastFrame".into(), json!(false));
    slot.insert("promptProjection".into(), prompt_projection(&implemented)?);
    slot.insert(
        "implementedModeIds".into(),
        json!(implemented.into_iter().map(String::from).collect::<Vec<_>>()),
    );
    slot.insert(
        "features".into(),
        json!(["video-reference-media-v1"]),
    );
    timing_params(&mut slot, duration_node)?;
    Ok(Value::Object(slot))
}

pub fn runtime_caps_doc() -> Result<Value, String> {
    Ok(json!({
        "schemaVersion": 1,
        "pluginId": PLUGIN_ID,
        "pluginVersion": PLUGIN_VERSION,
        "slots": [
            video_slot(MODEL_ID, DURATION_NODE_ID, 0, vec!["text_to_video"])?,
            video_slot(MODEL_ID_I2V_TURBO, DURATION_NODE_ID, 1, vec!["image_to_video"])?,
            video_slot(MODEL_ID_R2V_TURBO, R2V_DURATION_NODE_ID, 3, vec!["multi_image_to_video"])?,
        ]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runtime caps and manifest declare the same projection per model, inside each model's
    /// implemented modes (VS29: Comfy H3 keeps its actual slots and modes).
    #[test]
    fn every_model_declares_h3_projection_within_its_modes_and_matches_manifest() {
        let doc = runtime_caps_doc().expect("caps");
        let manifest: Value =
            serde_json::from_str(include_str!("../manifest.json")).expect("manifest json");
        let models = manifest["capabilitySlots"][0]["models"]
            .as_array()
            .expect("manifest models");
        let slots = doc["slots"].as_array().expect("slots");
        assert_eq!(slots.len(), models.len());
        for slot in slots {
            let projection = &slot["promptProjection"];
            assert_eq!(projection["profileId"], PROMPT_PROJECTION_PROFILE_ID);
            assert_eq!(projection["version"], PROMPT_PROJECTION_VERSION);
            assert_eq!(projection["inputModes"], slot["implementedModeIds"]);
            let model = models
                .iter()
                .find(|m| m["modelId"] == slot["modelId"])
                .expect("manifest model");
            assert_eq!(&model["promptProjection"], projection);
        }
    }
}
