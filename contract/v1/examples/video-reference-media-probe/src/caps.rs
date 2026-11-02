//! Runtime caps document for the offline video reference-media probe.

extern crate alloc;
use alloc::string::String;
use serde_json::{json, Value};

pub fn runtime_caps_doc() -> Result<Value, String> {
    Ok(json!({
        "schemaVersion": 1,
        "pluginId": "local.example.video-reference-media-probe",
        "pluginVersion": "0.2.0",
        "slots": [
            {
                "slot": "video",
                "modelId": "probe-offline",
                "features": ["video-reference-media-v1"],
                "mediaInputHints": { "images": true, "videos": true, "audios": true }
            },
            {
                "slot": "video",
                "modelId": "probe-offline-reject",
                "features": ["video-reference-media-v1"],
                "mediaInputHints": { "images": true, "videos": true, "audios": true }
            },
            {
                "slot": "keyframe",
                "modelId": "probe-offline-kf",
                "features": ["video-reference-media-v1"],
                "mediaInputHints": { "images": true, "videos": true, "audios": true }
            }
        ]
    }))
}
