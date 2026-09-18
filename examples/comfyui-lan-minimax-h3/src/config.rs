//! # AI_AUTHOR_GUIDE
//!
//! This is a **guest crate**, not a built-in client vendor. Comfy HTTP, node
//! bindings, and pinned API prompts live only here. Host `src-tauri/**`
//! must not name MiniMax or LAN Comfy branches.
//!
//! The job runs on the **user-configured Comfy HTTP origin** (`endpointBaseUrl`
//! from the adapter credential, else `COMFY_BASE_URL`). This crate does not
//! start local ComfyUI and does not call the cloud sibling crates.
//!
//! ## Models
//! | modelId | Pin | Media |
//! |---|---|---|
//! | `h3.t2v` | `workflows/h3.t2v.api.json` | none; turbo switch `139` stays false |
//! | `h3.i2v.turbo` | `workflows/h3.i2v.turbo.api.json` | required first frame; `139` true |
//! | `h3.r2v.turbo` | `workflows/h3.r2v.turbo.api.json` | required `referenceImagesB64` (1..=3); LoRA always on |
//!
//! Unknown `model` is an error. Do not guess node ids. Do not coerce r2v
//! start frames into `first_frame`.
//!
//! ## HTTP
//! Probe: `GET {base}/system_stats` (only when `endpointBaseUrl` is set).
//! Upload: `POST {base}/upload/image` multipart field `image`.
//! Submit: `POST {base}/prompt`. Query: `GET {base}/history/{prompt_id}`.
//!
//! ## Import root
//! Select the folder with `Cargo.toml` + `manifest.json` + `wit/` +
//! `vendor-sdk/` + `src/` + `workflows/`. Do not select `src/` alone.

extern crate alloc;
use alloc::format;
use alloc::string::String;

pub const PLUGIN_ID: &str = "local.example.comfyui-lan-h3";
pub const PLUGIN_VERSION: &str = "0.2.1";
pub const MODEL_ID: &str = "h3.t2v";
pub const MODEL_ID_I2V_TURBO: &str = "h3.i2v.turbo";
pub const MODEL_ID_R2V_TURBO: &str = "h3.r2v.turbo";

/// Shipped example origin. Live machines set adapter credential `baseUrl`.
pub const COMFY_BASE_URL: &str = "http://127.0.0.1:8188";

/// SHA-256 of `workflows/h3.t2v.api.json` (annotation nodes stripped).
pub const WORKFLOW_PIN: &str = "358db1f8b1c56d53ef41a612b508afeeee0a0444a0e66eccf4b0bc3241dc42c1";
/// SHA-256 of `workflows/h3.i2v.turbo.api.json`.
pub const WORKFLOW_PIN_I2V_TURBO: &str =
    "0e3ab998cb556f76c1eaf179ac328ec0566e78863eff5539b1d978d832d2323e";
/// SHA-256 of `workflows/h3.r2v.turbo.api.json`.
pub const WORKFLOW_PIN_R2V_TURBO: &str =
    "12ed63c6e790e45619aef2e9e4bf089c71d54a75beaa0ac1bbc4899787247752";

pub const SUBMIT_MODE: &str = "mapped_run";

pub const PROMPT_NODE_ID: &str = "131";
pub const PROMPT_FIELD_NAME: &str = "prompt";
pub const DURATION_NODE_ID: &str = "133";
pub const DURATION_FIELD_NAME: &str = "value";
pub const FPS_NODE_ID: &str = "130";
pub const FPS_FIELD_NAME: &str = "fps";
pub const MATH_EXPRESSION_NODE_ID: &str = "132";
pub const MATH_EXPRESSION_FIELD_NAME: &str = "expression";
pub const ASPECT_NODE_ID: &str = "115";
pub const ASPECT_FIELD_NAME: &str = "aspect_ratio";
pub const ASPECT_BINDING_ID: &str = "115:aspect_ratio";
pub const RESOLUTION_NODE_ID: &str = "115";
pub const RESOLUTION_FIELD_NAME: &str = "megapixels";
pub const RESOLUTION_BINDING_ID: &str = "115:megapixels";
pub const SAVE_VIDEO_NODE_ID: &str = "92";
pub const TURBO_SWITCH_NODE_ID: &str = "139";
pub const I2V_LOAD_IMAGE_NODE_ID: &str = "140";

pub const R2V_PROMPT_NODE_ID: &str = "138";
pub const R2V_DURATION_NODE_ID: &str = "132";
pub const R2V_MATH_NODE_ID: &str = "131";
pub const R2V_REF_NODE_ID: &str = "136";
pub const R2V_LORA_NODE_ID: &str = "142";
pub const R2V_LOAD_IMAGE_NODE_IDS: [&str; 3] = ["137", "139", "141"];

pub fn join_url(base: &str, path: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    let p = if path.starts_with('/') {
        path
    } else {
        return format!("{b}/{path}");
    };
    format!("{b}{p}")
}
