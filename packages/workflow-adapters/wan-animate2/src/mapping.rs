//! Template identity and the resources fixed by it.

/// Certified author canvas this runtime graph corresponds to
/// (`ComfyUI-API/Wan-Animate-2/video_wan_animate2_distilled.json`).
pub const TEMPLATE_ID: &str = "Wan-Animate-2/video_wan_animate2_distilled";
pub const TEMPLATE_SHA256: &str = "e08670cf0a14f4648e97283e4d7c78fe945015d5b11e4550e94b636a1f79324f";

/// Resources are fixed by the template version; changing them is a new template/package version.
pub struct Models {
    pub unet: &'static str,
    pub text_encoder: &'static str,
    pub vae: &'static str,
    pub clip_vision: &'static str,
}
pub const MODELS: Models = Models {
    unet: "wan_animate_2_distill_int8_convrot.safetensors",
    text_encoder: "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
    vae: "Wan2_1_VAE_bf16.safetensors",
    clip_vision: "clip_vision_h.safetensors",
};

/// `ContextWindowsManual` values of the certified canvas (used only when the user enables it).
pub struct ContextWindows {
    pub length: u64,
    pub overlap: u64,
    pub dim: u64,
}
pub const CONTEXT_WINDOWS: ContextWindows = ContextWindows { length: 21, overlap: 8, dim: 2 };