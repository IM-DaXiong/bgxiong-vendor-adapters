//! # AI_AUTHOR_GUIDE
//!
//! This is a **guest crate**, not a built-in client vendor. Vendor URLs, Key,
//! app ids, and node bindings live only here. Host `src-tauri/**` must not
//! name RunningHub.
//!
//! OpenAPI V2 **AI App** path (`/openapi/v2/run/ai-app/{id}`). Not
//! `/run/workflow`. Not the legacy task-envelope AI App runner.
//! Three video `modelId`s share one wasm.
//!
//! ## Models
//! | modelId | APP_ID | LoadImage | never image | prompt |
//! |---|---|---|---|---|
//! | `h3.r2v.1slot` | `2101954317581381633` | `114` | — | `132/prompt` |
//! | `h3.r2v.2slot` | `2101956370017906690` | `137`,`139` | `141` TurboLoRA | `138/value` |
//! | `h3.r2v.3slot` | `2101957517390737410` | `137`,`139`,`143` | `141` TurboLoRA | `138/value` |
//!
//! Unknown / missing `model` is Err. Do not default to the 2-slot graph.
//! Do not guess node ids. Do not share widgets across models.
//! Do not copy LAN Comfy H3 node ids into these tables.
//!
//! Dump authority: `workflows/h3.r2v.*.ui.json` + author apiCallDemo.
//!
//! ## Images
//! Host stages files (`video-reference-media-v1`). Guest uploads by handle
//! only. All three declare `multi_image_to_video` only. Empty refs fold
//! start/end frames; count must be 1..=kind.maxRefs.
//!
//! ## Key / wasm
//! Shipped `API_KEY` must stay placeholder. Never distribute wasm with a
//! live Key.
//!
//! ## Import root
//! Select the folder that contains `Cargo.toml` + `manifest.json` + `wit/` +
//! `vendor-sdk/` + `src/`. Do not select `src/` alone.
//!
//! ## Acceptance
//! L2 import/hello is not generation. Live: cover-import this version, pick
//! a slot, submit must return top-level `taskId`. CI must not claim RunningHub
//! is connected.

extern crate alloc;
use alloc::format;
use alloc::string::String;

pub const PLUGIN_ID: &str = "local.example.runninghub-h3-r2v";
pub const PLUGIN_VERSION: &str = "0.3.2";

pub const REGION_BASE_URL: &str = "https://www.runninghub.cn";

pub const API_KEY: &str = "REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY";

pub const MODEL_ID_1SLOT: &str = "h3.r2v.1slot";
pub const MODEL_ID_2SLOT: &str = "h3.r2v.2slot";
pub const MODEL_ID_3SLOT: &str = "h3.r2v.3slot";

pub const APP_ID_1SLOT: &str = "2101954317581381633";
pub const APP_ID_2SLOT: &str = "2101956370017906690";
pub const APP_ID_3SLOT: &str = "2101957517390737410";

pub const SUBMIT_MODE: &str = "mapped_run";

pub const REF_IMAGE_NODE_IDS_1: [&str; 1] = ["114"];
pub const REF_IMAGE_NODE_IDS_2: [&str; 2] = ["137", "139"];
pub const REF_IMAGE_NODE_IDS_3: [&str; 3] = ["137", "139", "143"];
pub const IMAGE_FIELD_NAME: &str = "image";

pub const ASPECT_FIELD_NAME: &str = "aspect_ratio";
pub const RESOLUTION_FIELD_NAME: &str = "megapixels";
pub const ASPECT_BINDING_ID: &str = "115:aspect_ratio";
pub const RESOLUTION_BINDING_ID: &str = "115:megapixels";

pub const PLACEHOLDER_API_KEY: &str = "REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY";

pub const WORKFLOW_NODE_MISMATCH_PREFIX: &str = "workflow_node_mismatch node=";

/// Per-kind prompt/timing/aspect widgets. LoadImage nodes stay on ModelKind.
#[derive(Debug, Clone, Copy)]
pub struct KindWidgetTable {
    pub prompt_node: &'static str,
    pub prompt_field: &'static str,
    pub duration_node: &'static str,
    pub duration_field: &'static str,
    pub fps_node: &'static str,
    pub fps_field: &'static str,
    pub math_node: &'static str,
    pub math_field: &'static str,
    pub aspect_node: &'static str,
    pub resolution_node: &'static str,
    pub duration_binding_id: &'static str,
    pub fps_binding_id: &'static str,
}

pub const SLOT1_WIDGETS: KindWidgetTable = KindWidgetTable {
    prompt_node: "132",
    prompt_field: "prompt",
    duration_node: "134",
    duration_field: "value",
    fps_node: "131",
    fps_field: "fps",
    math_node: "133",
    math_field: "expression",
    aspect_node: "115",
    resolution_node: "115",
    duration_binding_id: "134:value",
    fps_binding_id: "131:fps",
};

pub const SLOT2_WIDGETS: KindWidgetTable = KindWidgetTable {
    prompt_node: "138",
    prompt_field: "value",
    duration_node: "132",
    duration_field: "value",
    fps_node: "130",
    fps_field: "fps",
    math_node: "131",
    math_field: "expression",
    aspect_node: "115",
    resolution_node: "115",
    duration_binding_id: "132:value",
    fps_binding_id: "130:fps",
};

pub const SLOT3_WIDGETS: KindWidgetTable = KindWidgetTable {
    prompt_node: "138",
    prompt_field: "value",
    duration_node: "132",
    duration_field: "value",
    fps_node: "130",
    fps_field: "fps",
    math_node: "131",
    math_field: "expression",
    aspect_node: "115",
    resolution_node: "115",
    duration_binding_id: "132:value",
    fps_binding_id: "130:fps",
};

pub fn require_live_key() -> Result<(), String> {
    let key = API_KEY.trim();
    if key.is_empty() || key == PLACEHOLDER_API_KEY || key.contains("REPLACE_WITH") {
        return Err(
            "placeholder API key refused; set a dedicated low-balance key in config.rs".into(),
        );
    }
    if key.len() != 32 {
        return Err("API key must be the vendor 32-character key".into());
    }
    Ok(())
}

pub fn submit_url(app_id: &str) -> String {
    format!(
        "{}/openapi/v2/run/ai-app/{}",
        REGION_BASE_URL.trim_end_matches('/'),
        app_id
    )
}

pub fn upload_url() -> String {
    format!(
        "{}/openapi/v2/media/upload/binary",
        REGION_BASE_URL.trim_end_matches('/')
    )
}

pub fn query_url() -> String {
    format!(
        "{}/openapi/v2/query",
        REGION_BASE_URL.trim_end_matches('/')
    )
}
