//! # AI_AUTHOR_GUIDE
//!
//! Guest crate for LAN ComfyUI **Z-Image-Turbo** text-to-image. Not the
//! built-in `image_comfyui` path and not the MiniMax H3 video guest.
//! Host `src-tauri/**` must not name Z-Image or this crate.
//!
//! ## Models
//! | modelId | Pin | Native latent |
//! |---|---|---|
//! | `z.turbo` | `workflows/z.turbo.api.json` | 1920x1088 |
//! | `z.turbo.1080` | `workflows/z.turbo.1080.api.json` | 1920x1088 |
//! | `z.turbo.4k` | `workflows/z.turbo.4k.api.json` | 3840x2160 |
//!
//! Pure t2i. Reference images hard-fail. Do not guess node ids.
//!
//! ## HTTP
//! Probe: `GET {base}/system_stats` (only when `endpointBaseUrl` is set).
//! Submit: `POST {base}/prompt`. Query: `GET {base}/history/{prompt_id}`.
//! No `/upload/image`.
//!
//! ## Import root
//! Select the folder with `Cargo.toml` + `manifest.json` + `wit/` +
//! `vendor-sdk/` + `src/` + `workflows/`. Do not select `src/` alone.

extern crate alloc;
use alloc::format;
use alloc::string::String;

pub const PLUGIN_ID: &str = "local.example.comfyui-lan-z-image";
pub const PLUGIN_VERSION: &str = "0.1.0";
pub const MODEL_ID_TURBO: &str = "z.turbo";
pub const MODEL_ID_1080: &str = "z.turbo.1080";
pub const MODEL_ID_4K: &str = "z.turbo.4k";

/// Shipped example origin (product: not sensitive; may ship in packs).
/// Live machines may override via adapter credential `baseUrl`.
pub const COMFY_BASE_URL: &str = "http://192.168.18.8:8188";

/// SHA-256 of `workflows/z.turbo.api.json`.
pub const WORKFLOW_PIN_TURBO: &str =
    "74bc91eecf411bb2b1f541bd9432b8e184a3ab34661885ffcca4b63cf283831c";
/// SHA-256 of `workflows/z.turbo.1080.api.json`.
pub const WORKFLOW_PIN_1080: &str =
    "74f22c2d80362289cc94fb4fc01396bcc304049562c677070605a20141d1f1ba";
/// SHA-256 of `workflows/z.turbo.4k.api.json`.
pub const WORKFLOW_PIN_4K: &str =
    "952dc46dde09ab3675b2a6abadfcc256307c7638b6a77b5466b4fef861cc56a0";

pub const SUBMIT_MODE: &str = "mapped_run";

pub const PROMPT_NODE_ID: &str = "57:27";
pub const PROMPT_FIELD_NAME: &str = "text";
pub const LATENT_NODE_ID: &str = "57:13";
pub const SAMPLER_NODE_ID: &str = "57:3";
pub const SAVE_IMAGE_NODE_ID: &str = "9";

pub const NATIVE_W_TURBO: u32 = 1920;
pub const NATIVE_H_TURBO: u32 = 1088;
pub const NATIVE_W_1080: u32 = 1920;
pub const NATIVE_H_1080: u32 = 1088;
pub const NATIVE_W_4K: u32 = 3840;
pub const NATIVE_H_4K: u32 = 2160;

pub fn join_url(base: &str, path: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    let p = if path.starts_with('/') {
        path
    } else {
        return format!("{b}/{path}");
    };
    format!("{b}{p}")
}

pub fn size_label(w: u32, h: u32) -> String {
    format!("{w}x{h}")
}
