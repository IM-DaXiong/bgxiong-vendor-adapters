//! Vendor-neutral RCD. Adjustable params exist only when config.rs has a binding.

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec;

use bgx_vendor_adapter_sdk::{ParamBinding, RcdParam};
use serde_json::{json, Map, Value};

use crate::config::{
    ASPECT_BINDING_ID, DURATION_NODE_ID, FPS_NODE_ID, PLUGIN_ID, PLUGIN_VERSION,
    RESOLUTION_BINDING_ID,
};

fn bound(id: &str) -> Option<ParamBinding> {
    ParamBinding::new(id).ok()
}

fn insert_param(slot: &mut Map<String, Value>, key: &str, p: RcdParam) -> Result<(), String> {
    let v = serde_json::to_value(p).map_err(|e| format!("rcd {key}: {e}"))?;
    slot.insert(key.into(), v);
    Ok(())
}

/// Guest-declared windows. Host must not invent these numbers.
pub fn runtime_caps_doc() -> Result<Value, String> {
    let mut slot = Map::new();
    slot.insert("slot".into(), json!("video"));
    slot.insert("modelId".into(), json!("workflow"));
    slot.insert("maxReferenceImages".into(), json!(2));
    slot.insert("supportsFirstLastFrame".into(), json!(false));
    slot.insert(
        "implementedModeIds".into(),
        json!(["multi_image_to_video"]),
    );
    if let Some(b) = bound(DURATION_NODE_ID) {
        insert_param(
            &mut slot,
            "duration",
            RcdParam::range(1.0, 15.0, 1.0, 5.0, b),
        )?;
    }
    if let Some(b) = bound(FPS_NODE_ID) {
        insert_param(
            &mut slot,
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
            &mut slot,
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
            &mut slot,
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
    Ok(json!({
        "schemaVersion": 1,
        "pluginId": PLUGIN_ID,
        "pluginVersion": PLUGIN_VERSION,
        "slots": [Value::Object(slot)]
    }))
}
