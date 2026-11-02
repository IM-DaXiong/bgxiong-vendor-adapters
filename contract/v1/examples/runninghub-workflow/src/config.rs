//! User-edited constants for the isolated RunningHub AI App specimen.
//! Region is explicit: `.ai` and `.cn` are not fallbacks of each other.
//!
//! 用户手改的标本常量。区域必须写死：`.ai` 与 `.cn` 不是互相兜底关系。

extern crate alloc;
use alloc::format;
use alloc::string::String;

/// Official API origin for the chosen region.
///
/// International: `https://www.runninghub.ai`
/// China: `https://www.runninghub.cn`
pub const PLUGIN_ID: &str = "local.example.specimen";
pub const PLUGIN_VERSION: &str = "0.2.1";

pub const REGION_BASE_URL: &str = "https://www.runninghub.cn";

/// Dedicated vendor API key (32 characters). Keep a low-balance key in this specimen.
pub const API_KEY: &str = "REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY";

/// RunningHub AI App id (`webappId`). Same digits as the console
/// `/call-api/api-detail/{id}?apiType=5` page. Not the V2 workflow runner URL.
pub const WEBAPP_ID: &str = "2084935567606894593";

/// Widget in `nodeInfoList` that receives the workbench prompt.
/// Freeze from a one-time `GET /api/webapp/apiCallDemo` (author machine). Do not guess.
pub const PROMPT_NODE_ID: &str = "6";
pub const PROMPT_FIELD_NAME: &str = "text";

/// Shipped specimen uses `mapped_run` so the video workbench prompt is submitted.
pub const SUBMIT_MODE: &str = "mapped_run";

/// Leave empty until apiCallDemo lists an IMAGE fieldType. Do not guess.
pub const IMAGE_NODE_ID: &str = "";
pub const IMAGE_FIELD_NAME: &str = "image";

/// Duration / fps / resolution exist only as nodeInfoList widgets on the AI App.
/// Official POST `/task/openapi/ai-app/run` body is webappId + apiKey + nodeInfoList.
/// Do not copy Model API fields (`imageUrls`, `resolution: "1k"`) onto this path.
///
/// BINDING_HOWTO: fill DURATION_NODE_ID / FPS_NODE_ID / RESOLUTION_NODE_ID from
/// a one-time GET /api/webapp/apiCallDemo (author machine). Leave `""` if absent.
pub const DURATION_NODE_ID: &str = "";
pub const DURATION_FIELD_NAME: &str = "value";
pub const FPS_NODE_ID: &str = "";
pub const FPS_FIELD_NAME: &str = "value";
pub const RESOLUTION_NODE_ID: &str = "";
pub const RESOLUTION_FIELD_NAME: &str = "value";

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

/// POST `/task/openapi/ai-app/run` (RunningHub AI App, not V2 workflow).
pub fn submit_url() -> String {
    format!(
        "{}/task/openapi/ai-app/run",
        REGION_BASE_URL.trim_end_matches('/')
    )
}

/// Upload is out of this specimen until IMAGE/VIDEO fieldType is frozen.
pub fn upload_url() -> String {
    format!(
        "{}/task/openapi/upload",
        REGION_BASE_URL.trim_end_matches('/')
    )
}

pub fn query_url() -> String {
    format!(
        "{}/task/openapi/outputs",
        REGION_BASE_URL.trim_end_matches('/')
    )
}
