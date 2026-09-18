//! # AI_AUTHOR_GUIDE
//!
//! This is a **guest crate**, not a built-in client vendor. Vendor URLs, Key,
//! workflow id, and node bindings live only here. Host `src-tauri/**` must not
//! name RunningHub.
//!
//! ## Why V2 only
//! Sibling crate `runninghub-workflow/` is the AI App teaching specimen
//! (`/task/openapi/ai-app/run` + `/outputs`, empty timing bindings).
//! This crate is the MiniMax H3 **r2v turbo** V2 working example. The graph
//! id below is a Comfy **workflow** id: `POST /task/openapi/ai-app/run`
//! returns `webapp not exists`. Do not add an AI App parse branch.
//!
//! ## Binding table (UI dump `video_minimax_h3_r2v_turbo.json` only)
//! | RCD / host field | node / field | vendor value |
//! |---|---|---|
//! | prompt | `138` / `value` | PrimitiveStringMultiline |
//! | durationSeconds | `132` / `value` | PrimitiveFloat; write user seconds 1..=15 |
//! | fps | `130` / `fps` **and** `131` / `expression` | Closed set 24/25/30. Rewrite both. |
//! | aspect | `115` / `aspect_ratio` | RCD `16:9` → `16:9 (Widescreen)` |
//! | resolution | `115` / `megapixels` | RCD `mp:0.4` → FLOAT `0.4` |
//! | refs 1..=2 | `137` / `139` / `image` | After RH upload; magic-byte filename |
//!
//! Do **not** copy LAN pin `h3.r2v.turbo.api.json`: that pin treats `141` as
//! LoadImage. This uploaded UI graph has `141=MiniMaxH3TurboLoRA` and
//! `142=MiniMaxH3TurboSampler`. Never put `image` on node `141`.
//!
//! ## Change the vendor graph
//! Clone or export the new graph. Update `WORKFLOW_ID` and every binding
//! together. Do **not** guess ids. This id is not a secret; swapping only
//! the Key does **not** promise the same graph on another account.
//!
//! ## Images
//! Decode Base64, sniff png/jpeg/webp/gif, then upload with matching filename
//! and MIME. Unknown magic → explicit Err. Never default to PNG. Never drop
//! refs and silently become T2V. RCD mode is `multi_image_to_video` only
//! (LAN H3 r2v same): never declare I2V / first_last_frame. Empty refs fold
//! `startFrameB64`/`endFrameB64` into 137/139. Need 1..=2 images.
//!
//! ## Key / wasm
//! Put a live Key only in a **writable private copy**. Shipped `API_KEY` must
//! stay `PLACEHOLDER_API_KEY`. Packed wasm can leak the Key; never distribute
//! a wasm that contains a live Key.
//!
//! ## Import root
//! Select the folder that contains `Cargo.toml` + `manifest.json` + `wit/` +
//! `vendor-sdk/` + `src/`. Do not select `src/` alone.
//!
//! ## Acceptance
//! L2 import/hello is not generation. Live proof: video workbench submit →
//! V2 query terminal → host download → restart reclaim. CI must not claim
//! RunningHub is connected.
//!
//! ## After edits
//! `cargo test --manifest-path Cargo.toml` in this crate; repo
//! `cd ui && npm run check:affected`. On failure, give the agent
//! `logs/frontend-*.log` + `logs/bgxiong-ai-story-*.log` tails and the
//! Settings import compiler pane.
//!
//! Region is explicit: `.ai` and `.cn` are not fallbacks of each other.

extern crate alloc;
use alloc::format;
use alloc::string::String;

/// Official API origin for the chosen region.
///
/// International: `https://www.runninghub.ai`
/// China: `https://www.runninghub.cn`
pub const PLUGIN_ID: &str = "local.example.runninghub-h3";
pub const PLUGIN_VERSION: &str = "0.2.3";

pub const REGION_BASE_URL: &str = "https://www.runninghub.cn";

/// Dedicated vendor API key (32 characters). Shipped copy must stay the placeholder.
pub const API_KEY: &str = "REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY";

/// Case-source Comfy workflow id (not an AI App webappId).
pub const WORKFLOW_ID: &str = "2100758868111486978";

/// PrimitiveStringMultiline on the uploaded r2v turbo UI graph.
pub const PROMPT_NODE_ID: &str = "138";
pub const PROMPT_FIELD_NAME: &str = "value";

/// Shipped example uses `mapped_run` so the video workbench prompt is submitted.
pub const SUBMIT_MODE: &str = "mapped_run";

/// LoadImage slots wired to MiniMaxH3ReferenceToVideo ref_image_0 / ref_image_1.
/// Node 141 on this graph is TurboLoRA — never treat it as an image.
pub const REF_IMAGE_NODE_IDS: [&str; 2] = ["137", "139"];
pub const IMAGE_FIELD_NAME: &str = "image";

/// Duration seconds 1–15: PrimitiveFloat `132/value` (not ImpactSwitch 205).
pub const DURATION_NODE_ID: &str = "132";
pub const DURATION_FIELD_NAME: &str = "value";
pub const FPS_NODE_ID: &str = "130";
pub const FPS_FIELD_NAME: &str = "fps";
pub const ASPECT_NODE_ID: &str = "115";
pub const ASPECT_FIELD_NAME: &str = "aspect_ratio";
pub const ASPECT_BINDING_ID: &str = "115:aspect_ratio";
pub const RESOLUTION_NODE_ID: &str = "115";
pub const RESOLUTION_FIELD_NAME: &str = "megapixels";
pub const RESOLUTION_BINDING_ID: &str = "115:megapixels";

/// Math node that consumes duration `132` as `values.a`.
pub const MATH_EXPRESSION_NODE_ID: &str = "131";
pub const MATH_EXPRESSION_FIELD_NAME: &str = "expression";

/// Sentinel equal to the shipped `API_KEY` placeholder. Live calls must not use it.
pub const PLACEHOLDER_API_KEY: &str = "REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY";

/// Refuse empty / placeholder / non-32-char keys before any HTTP submit or query.
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

/// POST `/openapi/v2/run/workflow/{id}` — this graph id is a workflow, not a webapp.
pub fn submit_url() -> String {
    format!(
        "{}/openapi/v2/run/workflow/{}",
        REGION_BASE_URL.trim_end_matches('/'),
        WORKFLOW_ID
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
