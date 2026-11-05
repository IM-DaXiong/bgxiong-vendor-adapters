//! Authoritative per-workflow app IDs, node widgets and weight pins; no credentials.
extern crate alloc;
use alloc::{format, string::String};
pub const REGION_BASE_URL: &str = "https://www.runninghub.cn";
pub const MODEL_ID_1SLOT: &str = "h3.r2v.1slot";
pub const MODEL_ID_2SLOT: &str = "h3.r2v.2slot";
pub const MODEL_ID_3SLOT: &str = "h3.r2v.3slot";
pub const MODEL_ID_4SLOT: &str = "h3.r2v.4slot";
pub const MODEL_ID_I2V_TURBO: &str = "h3.i2v.turbo";
pub const MODEL_ID_FL2VA: &str = "h3.fl2va.s20";
pub const MODEL_ID_SOUNDTRACK: &str = "h3.soundtrack.1ref";

pub const APP_ID_1SLOT: &str = "2108667855574224898";
pub const APP_ID_2SLOT: &str = "2101956370017906690";
pub const APP_ID_3SLOT: &str = "2101957517390737410";
pub const APP_ID_4SLOT: &str = "2103015554197053441";
pub const APP_ID_I2V_TURBO: &str = "2101954317581381633";
pub const APP_ID_FL2VA: &str = "2107978021675294722";
pub const APP_ID_SOUNDTRACK: &str = "2107982892470198274";

pub const SUBMIT_MODE: &str = "mapped_run";

pub const REF_IMAGE_NODE_IDS_1REF: [&str; 1] = ["137"];
pub const REF_IMAGE_NODE_IDS_1: [&str; 1] = ["114"];
pub const REF_IMAGE_NODE_IDS_2: [&str; 2] = ["137", "139"];
pub const REF_IMAGE_NODE_IDS_3: [&str; 3] = ["137", "139", "143"];
pub const REF_IMAGE_NODE_IDS_4: [&str; 4] = ["137", "139", "143", "144"];
pub const REF_IMAGE_NODE_IDS_FL2VA: [&str; 1] = ["114"];
pub const REF_IMAGE_NODE_IDS_SOUNDTRACK: [&str; 1] = ["137"];
pub const IMAGE_FIELD_NAME: &str = "image";
pub const AUDIO_NODE_ID_SOUNDTRACK: &str = "142";
pub const AUDIO_FIELD_NAME: &str = "audio";

pub const ASPECT_FIELD_NAME: &str = "aspect_ratio";
pub const RESOLUTION_FIELD_NAME: &str = "megapixels";

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
};

/// Ref2va one-image App. No fps widget. Prompt is node 138, not the i2v prompt node.
pub const REF1_WIDGETS: KindWidgetTable = KindWidgetTable {
    prompt_node: "138",
    prompt_field: "value",
    duration_node: "132",
    duration_field: "value",
    fps_node: "",
    fps_field: "",
    math_node: "",
    math_field: "",
    aspect_node: "115",
    resolution_node: "115",
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
};

pub const SLOT4_WIDGETS: KindWidgetTable = SLOT3_WIDGETS;

/// First-frame fl2va App. No fps node (node 131 is noise_seed on this graph).
pub const FL2VA_WIDGETS: KindWidgetTable = KindWidgetTable {
    prompt_node: "133",
    prompt_field: "prompt",
    duration_node: "135",
    duration_field: "value",
    fps_node: "",
    fps_field: "",
    math_node: "",
    math_field: "",
    aspect_node: "115",
    resolution_node: "115",
};

/// Soundtrack / lip-sync App. Node numbers collide with 2-slot; steps are 20.
pub const SOUNDTRACK_WIDGETS: KindWidgetTable = KindWidgetTable {
    prompt_node: "138",
    prompt_field: "value",
    duration_node: "132",
    duration_field: "value",
    fps_node: "",
    fps_field: "",
    math_node: "",
    math_field: "",
    aspect_node: "115",
    resolution_node: "115",
};

/// nodeId / fieldName / fieldValue. Strings only. No fieldData.
pub type WeightPin = (&'static str, &'static str, &'static str);

const CLIP_H3: &str = "qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors";
const VIDEO_VAE: &str = "minimax_h3_video_vae_fp16.safetensors";
const AUDIO_VAE: &str = "minimax_h3_audio_vae_fp32.safetensors";

pub const SLOT1_WEIGHT_PINS: &[WeightPin] = &[
    ("129", "clip_name", CLIP_H3),
    ("129", "type", "minimax"),
    ("129", "device", "default"),
    (
        "128",
        "unet_name",
        "minimax_h3_fl2va_pruned_int8_convrot.safetensors",
    ),
    ("128", "weight_dtype", "default"),
    ("121", "vae_name", VIDEO_VAE),
    ("122", "vae_name", AUDIO_VAE),
    ("125", "steps", "6"),
    ("125", "scheduler", "simple"),
    ("125", "denoise", "1"),
];

/// One-image ref2va App. CLIP has clip_name only. Steps stay 6. Node 136 is closed.
pub const REF1_WEIGHT_PINS: &[WeightPin] = &[
    ("128", "clip_name", CLIP_H3),
    (
        "127",
        "unet_name",
        "minimax_h3_ref2va_pruned_int8_convrot.safetensors",
    ),
    ("127", "weight_dtype", "default"),
    ("119", "vae_name", VIDEO_VAE),
    ("120", "vae_name", AUDIO_VAE),
    ("124", "steps", "6"),
    ("124", "scheduler", "simple"),
    ("124", "denoise", "1"),
    ("136", "ref_image_size", "match"),
];

pub const SLOT_REF2VA_WEIGHT_PINS: &[WeightPin] = &[
    ("128", "clip_name", CLIP_H3),
    ("128", "type", "minimax"),
    ("128", "device", "default"),
    (
        "127",
        "unet_name",
        "minimax_h3_ref2va_pruned_int8_convrot.safetensors",
    ),
    ("127", "weight_dtype", "default"),
    ("119", "vae_name", VIDEO_VAE),
    ("120", "vae_name", AUDIO_VAE),
    ("124", "steps", "6"),
    ("124", "scheduler", "simple"),
    ("124", "denoise", "1"),
];

/// fl2va demo CLIP has clip_name only. Do not invent type/device.
pub const FL2VA_WEIGHT_PINS: &[WeightPin] = &[
    ("130", "clip_name", CLIP_H3),
    (
        "129",
        "unet_name",
        "minimax_h3_fl2va_pruned_int8_convrot.safetensors",
    ),
    ("129", "weight_dtype", "default"),
    ("121", "vae_name", VIDEO_VAE),
    ("122", "vae_name", AUDIO_VAE),
    ("126", "steps", "20"),
    ("126", "scheduler", "simple"),
    ("126", "denoise", "1"),
];

pub const SOUNDTRACK_WEIGHT_PINS: &[WeightPin] = &[
    ("128", "clip_name", CLIP_H3),
    (
        "127",
        "unet_name",
        "minimax_h3_ref2va_pruned_int8_convrot.safetensors",
    ),
    ("127", "weight_dtype", "default"),
    ("119", "vae_name", VIDEO_VAE),
    ("120", "vae_name", AUDIO_VAE),
    ("124", "steps", "20"),
    ("124", "scheduler", "simple"),
    ("124", "denoise", "1"),
    ("136", "ref_image_size", "match"),
];

pub fn submit_ai_app_url(app: &str) -> String {
    format!("{REGION_BASE_URL}/openapi/v2/run/ai-app/{app}")
}
