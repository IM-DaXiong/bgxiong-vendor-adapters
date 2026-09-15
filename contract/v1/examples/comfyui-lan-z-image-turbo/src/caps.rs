//! Vendor-neutral RCD: one image slot document per modelId.
//! resolution is fixed (native WxH) so workbench null size does not force appliedParams.

extern crate alloc;
use alloc::format;
use alloc::string::String;

use bgx_vendor_adapter_sdk::RcdParam;
use serde_json::{json, Map, Value};

use crate::config::{
    size_label, MODEL_ID_1080, MODEL_ID_4K, MODEL_ID_TURBO, NATIVE_H_1080, NATIVE_H_4K,
    NATIVE_H_TURBO, NATIVE_W_1080, NATIVE_W_4K, NATIVE_W_TURBO, PLUGIN_ID, PLUGIN_VERSION,
};

fn insert_param(slot: &mut Map<String, Value>, key: &str, p: RcdParam) -> Result<(), String> {
    let v = serde_json::to_value(p).map_err(|e| format!("rcd {key}: {e}"))?;
    slot.insert(key.into(), v);
    Ok(())
}

fn image_slot(model_id: &str, w: u32, h: u32) -> Result<Value, String> {
    let mut slot = Map::new();
    slot.insert("slot".into(), json!("image"));
    slot.insert("modelId".into(), json!(model_id));
    insert_param(
        &mut slot,
        "resolution",
        RcdParam::fixed(json!(size_label(w, h))),
    )?;
    Ok(Value::Object(slot))
}

pub fn runtime_caps_doc() -> Result<Value, String> {
    Ok(json!({
        "schemaVersion": 1,
        "pluginId": PLUGIN_ID,
        "pluginVersion": PLUGIN_VERSION,
        "slots": [
            image_slot(MODEL_ID_TURBO, NATIVE_W_TURBO, NATIVE_H_TURBO)?,
            image_slot(MODEL_ID_1080, NATIVE_W_1080, NATIVE_H_1080)?,
            image_slot(MODEL_ID_4K, NATIVE_W_4K, NATIVE_H_4K)?,
        ]
    }))
}
