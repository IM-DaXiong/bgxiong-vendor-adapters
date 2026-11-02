//! Vendor-neutral RCD: one text slot.

extern crate alloc;
use alloc::string::String;

use serde_json::{json, Value};

use crate::config::{EXAMPLE_MODEL_ID, PLUGIN_ID, PLUGIN_VERSION};

pub fn runtime_caps_doc() -> Result<Value, String> {
    Ok(json!({
        "schemaVersion": 1,
        "pluginId": PLUGIN_ID,
        "pluginVersion": PLUGIN_VERSION,
        "slots": [
            {
                "slot": "text",
                "modelId": EXAMPLE_MODEL_ID,
            }
        ]
    }))
}
