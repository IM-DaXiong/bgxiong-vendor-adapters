//! # AI_AUTHOR_GUIDE
//!
//! Guest crate for LAN ComfyUI **Z-Image-Turbo** text-to-image. Not the
//! built-in `image_comfyui` path and not the MiniMax H3 video guest.
//! Host `src-tauri/**` must not name Z-Image or this crate.
//!
//! ## Model
//! | modelId | Pin | Size |
//! |---|---|---|
//! | `z.turbo` | `workflows/z.turbo.api.json` | 10 enum WxH presets (2K/4K x 5 aspects) |
//!
//! Pure t2i. Reference images hard-fail. Do not guess node ids.
//! `size` writes `EmptySD3LatentImage` `57:13` width/height. Default 1920x1088.
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
pub const PLUGIN_VERSION: &str = "0.3.0";
pub const MODEL_ID_TURBO: &str = "z.turbo";

/// Shipped example origin (product: not sensitive; may ship in packs).
/// Live machines may override via adapter credential `baseUrl`.
pub const COMFY_BASE_URL: &str = "http://192.168.18.8:8188";

/// SHA-256 of `workflows/z.turbo.api.json`.
pub const WORKFLOW_PIN_TURBO: &str =
    "74bc91eecf411bb2b1f541bd9432b8e184a3ab34661885ffcca4b63cf283831c";

pub const SUBMIT_MODE: &str = "mapped_run";

pub const PROMPT_NODE_ID: &str = "57:27";
pub const PROMPT_FIELD_NAME: &str = "text";
pub const LATENT_NODE_ID: &str = "57:13";
pub const LATENT_SIZE_BINDING: &str = "57:13.size";
pub const SAMPLER_NODE_ID: &str = "57:3";
pub const SAVE_IMAGE_NODE_ID: &str = "9";

pub const DEFAULT_W: u32 = 1920;
pub const DEFAULT_H: u32 = 1088;

/// Closed size presets: 2K/4K x five UI aspects. Width and height are multiples of 8.
pub const SIZE_PRESETS: &[(&str, u32, u32, &str)] = &[
    ("2k-16-9", 1920, 1088, "16:9"),
    ("2k-9-16", 1088, 1920, "9:16"),
    ("2k-1-1", 1408, 1408, "1:1"),
    ("2k-4-3", 1664, 1248, "4:3"),
    ("2k-3-4", 1248, 1664, "3:4"),
    ("4k-16-9", 3840, 2160, "16:9"),
    ("4k-9-16", 2160, 3840, "9:16"),
    ("4k-1-1", 2880, 2880, "1:1"),
    ("4k-4-3", 3328, 2496, "4:3"),
    ("4k-3-4", 2496, 3328, "3:4"),
];

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

pub fn lookup_size(w: u32, h: u32) -> Option<&'static (&'static str, u32, u32, &'static str)> {
    SIZE_PRESETS.iter().find(|p| p.1 == w && p.2 == h)
}
