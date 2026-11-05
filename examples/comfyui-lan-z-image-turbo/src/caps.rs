//! Vendor-neutral RCD: one image slot, ten WxH enum presets with aspectRatio.

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use bgx_vendor_adapter_sdk::{ParamBinding, RcdEnumOption, RcdParam};
use serde_json::{json, Map, Value};

use crate::config::{
    size_label, LATENT_SIZE_BINDING, MODEL_ID_TURBO, PLUGIN_ID, PLUGIN_VERSION, SIZE_PRESETS,
};

fn insert_param(slot: &mut Map<String, Value>, key: &str, p: RcdParam) -> Result<(), String> {
    let v = serde_json::to_value(p).map_err(|e| format!("rcd {key}: {e}"))?;
    slot.insert(key.into(), v);
    Ok(())
}

fn resolution_options() -> Vec<Value> {
    SIZE_PRESETS
        .iter()
        .map(|(id, w, h, ar)| {
            let value = size_label(*w, *h);
            let label = if id.starts_with("2k") {
                format!("2K {ar}")
            } else {
                format!("4K {ar}")
            };
            RcdEnumOption::new(*id, label, json!(value))
                .with_aspect(*ar)
                .to_json()
        })
        .collect()
}

fn image_slot() -> Result<Value, String> {
    let mut slot = Map::new();
    slot.insert("slot".into(), json!("image"));
    slot.insert("modelId".into(), json!(MODEL_ID_TURBO));
    slot.insert("maxReferenceImages".into(), json!(0));
    let binding = ParamBinding::new(LATENT_SIZE_BINDING)
        .map_err(|e| format!("resolution binding: {e}"))?;
    insert_param(
        &mut slot,
        "resolution",
        RcdParam::enum_of(resolution_options(), json!("1920x1088"), binding),
    )?;
    Ok(Value::Object(slot))
}

pub fn runtime_caps_doc() -> Result<Value, String> {
    Ok(json!({
        "schemaVersion": 1,
        "pluginId": PLUGIN_ID,
        "pluginVersion": PLUGIN_VERSION,
        "slots": [image_slot()?]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgx_vendor_adapter_sdk::PromptProjection;

    /// VS29: the prompt projection fields are optional; an image adapter declares none and the
    /// SDK refuses one on a non-video slot.
    #[test]
    fn image_slot_has_no_prompt_projection() {
        let doc = runtime_caps_doc().expect("caps");
        assert!(doc["slots"][0].get("promptProjection").is_none());
        let manifest: Value =
            serde_json::from_str(include_str!("../manifest.json")).expect("manifest json");
        for model in manifest["capabilitySlots"][0]["models"]
            .as_array()
            .expect("manifest models")
        {
            assert!(model.get("promptProjection").is_none());
        }
        let projection =
            PromptProjection::new("x", 1, "x-prompt-writing", vec!["text_to_video".into()])
                .expect("valid descriptor");
        assert!(projection.validate_for_slot("image", &[]).is_err());
    }
}
