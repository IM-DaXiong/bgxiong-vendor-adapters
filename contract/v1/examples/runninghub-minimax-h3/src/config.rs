//! # AI_AUTHOR_GUIDE
//!
//! This is a **guest crate**, not a built-in client vendor. Vendor URLs, Key,
//! workflow id, and node bindings live only here. Host `src-tauri/**` must not
//! name RunningHub.
//!
//! ## Why V2 only
//! Sibling crate `runninghub-workflow/` is the AI App teaching specimen
//! (`/task/openapi/ai-app/run` + `/outputs`, empty timing bindings).
//! This crate is the MiniMax H3 **working** example. The graph id below is a
//! Comfy **workflow** id: `POST /task/openapi/ai-app/run` returns
//! `webapp not exists`. Do not add an AI App parse branch to "make it work".
//!
//! ## Binding table (frozen from an official export of this graph)
//! | RCD / host field | node / field | vendor value |
//! |---|---|---|
//! | prompt | `134` / `prompt` | workbench text |
//! | durationSeconds | `205` / `select` | ImpactSwitch: user 1..=4 → 5s / select=1; 5..=15 → select=seconds-4. `appliedParams.duration` stays the user value. |
//! | fps | `130` / `fps` **and** `132` / `expression` | Closed set 24/25/30. Expression must be rewritten atomically. |
//! | resolution | `115` / `aspect_ratio` | RCD id `16:9` maps to combo `16:9 (Widescreen)` (see `resolution_combo`). |
//! | start frame | `139` / `image` | Upload filename after magic-byte sniff |
//! | end frame | `206` / `image` | Same |
//!
//! ## Change the vendor graph
//! Clone or export the new graph from the vendor. Read real node ids from that
//! dump or the official API response. Do **not** guess ids. Do **not** reuse
//! this table on another workflow. Update `WORKFLOW_ID` and every binding
//! together. This id is not a secret; swapping only the Key does **not**
//! promise the same graph is runnable on another account.
//!
//! ## Images
//! Decode Base64, sniff png/jpeg/webp/gif, then upload with matching filename
//! and MIME. Unknown magic → explicit Err. Never default to PNG. Never drop
//! frames and silently become T2V.
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
pub const PLUGIN_VERSION: &str = "0.1.0";

pub const REGION_BASE_URL: &str = "https://www.runninghub.cn";

/// Dedicated vendor API key (32 characters). Shipped copy must stay the placeholder.
pub const API_KEY: &str = "REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY";

/// Case-source Comfy workflow id (not an AI App webappId).
pub const WORKFLOW_ID: &str = "2084935567606894593";

/// Live graph: CR Prompt Text, not CLIPTextEncode "6".
pub const PROMPT_NODE_ID: &str = "134";
pub const PROMPT_FIELD_NAME: &str = "prompt";

/// Shipped example uses `mapped_run` so the video workbench prompt is submitted.
pub const SUBMIT_MODE: &str = "mapped_run";

/// MiniMaxH3ImageToVideo first_frame / last_frame LoadImage widgets.
pub const IMAGE_NODE_ID: &str = "139";
pub const IMAGE_FIELD_NAME: &str = "image";
pub const LAST_IMAGE_NODE_ID: &str = "206";
pub const LAST_IMAGE_FIELD_NAME: &str = "image";

/// Duration seconds 1–15: ImpactSwitch `205` minimum is 5s.
/// Map `1..4 → 5s / select=1`, `5..15 → select=seconds-4`.
/// Do not write Math `132/values.a` — that input is wired from 205.
pub const DURATION_NODE_ID: &str = "205";
pub const DURATION_FIELD_NAME: &str = "select";
pub const FPS_NODE_ID: &str = "130";
pub const FPS_FIELD_NAME: &str = "fps";
pub const RESOLUTION_NODE_ID: &str = "115";
pub const RESOLUTION_FIELD_NAME: &str = "aspect_ratio";

/// Same Math node as duration: fps must rewrite the frame-count expression.
pub const MATH_EXPRESSION_NODE_ID: &str = "132";
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
