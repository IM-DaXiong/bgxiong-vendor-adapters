//! Isolated RunningHub V2 MiniMax H3 r2v turbo video example.
//! Vendor strings stay in this crate. Host `src-tauri/**` must not name RunningHub.

#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
extern crate alloc;

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

pub mod caps;
pub mod config;
pub mod runninghub;

#[cfg(not(target_arch = "wasm32"))]
use bgx_vendor_adapter_sdk::{query_result, submit_accepted, Output, QueryResult};

#[cfg(not(target_arch = "wasm32"))]
use crate::config::{query_url, submit_url};
#[cfg(not(target_arch = "wasm32"))]
use crate::runninghub::{
    outputs_to_json, parse_query, parse_submit_task_id, submit_body_mapped, MappedStatus,
};

#[cfg(any(target_arch = "wasm32", test))]
mod wasm_mem;

#[cfg(target_arch = "wasm32")]
#[path = "guest.rs"]
mod guest;

/// Host-testable operation dispatcher used by the wasm guest.
#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_json(operation: &str, payload_json: &str) -> Result<serde_json::Value, String> {
    match operation {
        "capabilities" => crate::caps::runtime_caps_doc(),
        "probe" => Ok(serde_json::json!({"ok": true, "live": false})),
        "submit" => submit_op(payload_json),
        "query" => query_op(payload_json),
        "cancel" | "upload" => Err(format!("{operation} is out of example scope")),
        other => Err(format!("unknown operation {other}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn submit_op(payload_json: &str) -> Result<serde_json::Value, String> {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|e| format!("payload: {e}"))?;
    let prompt = payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let duration = json_opt_f64(payload.get("durationSeconds"));
    let fps = json_opt_f64(payload.get("fps"));
    let resolution = payload.get("resolution").and_then(|v| v.as_str());
    let aspect = payload
        .get("aspect")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("aspectRatio").and_then(|v| v.as_str()));
    let first = payload
        .get("startFrameB64")
        .and_then(|v| v.as_str())
        .or_else(|| {
            payload
                .get("referenceImagesB64")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        });
    let last = payload.get("endFrameB64").and_then(|v| v.as_str());
    let _url = submit_url();
    let _body = submit_body_mapped(prompt, duration, fps, resolution, aspect, first, last)?;
    Err("live HTTP is only available inside the wasm guest via host-http".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn json_opt_f64(v: Option<&serde_json::Value>) -> Option<f64> {
    let v = v?;
    v.as_f64()
        .or_else(|| v.as_u64().map(|n| n as f64))
        .or_else(|| v.as_i64().map(|n| n as f64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

#[cfg(not(target_arch = "wasm32"))]
fn query_op(payload_json: &str) -> Result<serde_json::Value, String> {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|e| format!("payload: {e}"))?;
    let _id = payload
        .get("vendorTaskId")
        .and_then(|v| v.as_str())
        .ok_or("query requires vendorTaskId")?;
    let _url = query_url();
    Err("live HTTP is only available inside the wasm guest via host-http".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn map_submit_response_json(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let id = parse_submit_task_id(body)?;
    let sdk = submit_accepted(id);
    serde_json::from_str(sdk.data_json.as_deref().unwrap_or("{}"))
        .map_err(|e| format!("sdk: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn map_query_response_json(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let q = parse_query(body)?;
    let status = match q.status {
        MappedStatus::Waiting => "running",
        MappedStatus::Succeeded => "succeeded",
        MappedStatus::Failed => "failed",
    };
    let outputs: Vec<Output> = q
        .outputs
        .into_iter()
        .map(|o| Output::Media {
            media_kind: o.media_kind,
            source: o.source,
            value: o.value,
            mime: o.mime,
        })
        .collect();
    let sdk = query_result(&QueryResult {
        status: status.into(),
        outputs,
        progress_text: q.vendor_message,
        retry_after_ms: None,
    });
    let mut data: serde_json::Value = serde_json::from_str(sdk.data_json.as_deref().unwrap_or("{}"))
        .map_err(|e| format!("sdk: {e}"))?;
    if let Some(code) = q.vendor_code {
        data["vendorCode"] = serde_json::Value::String(code);
    }
    let _ = outputs_to_json;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_key_hard_fails() {
        assert!(crate::config::require_live_key().is_err());
    }

    #[test]
    fn probe_is_offline_ok() {
        let v = dispatch_json("probe", "{}").expect("probe");
        assert_eq!(v["ok"], true);
        assert_eq!(v["live"], false);
    }

    #[test]
    fn capabilities_returns_rcd() {
        let v = dispatch_json("capabilities", "{}").expect("caps");
        assert_eq!(v["schemaVersion"], 1);
        assert_eq!(v["pluginId"], crate::config::PLUGIN_ID);
        assert_eq!(v["pluginVersion"], "0.2.3");
        assert_eq!(v["slots"][0]["slot"], "video");
        assert_eq!(v["slots"][0]["maxReferenceImages"], 2);
        assert_eq!(
            v["slots"][0]["implementedModeIds"],
            serde_json::json!(["multi_image_to_video"])
        );
        assert_eq!(v["slots"][0]["supportsFirstLastFrame"], false);
        assert_eq!(v["slots"][0]["duration"]["min"], 1.0);
        assert_eq!(v["slots"][0]["duration"]["max"], 15.0);
        assert_eq!(v["slots"][0]["fps"]["options"].as_array().map(|a| a.len()), Some(3));
        assert_eq!(
            v["slots"][0]["resolution"]["options"]
                .as_array()
                .map(|a| a.len()),
            Some(4)
        );
        assert_eq!(v["slots"][0]["resolution"]["default"], "mp:0.4");
        assert_eq!(v["slots"][0]["aspect"]["default"], "16:9");
        assert_eq!(v["slots"][0]["resolution"]["bindingId"], "115:megapixels");
        assert_eq!(v["slots"][0]["aspect"]["bindingId"], "115:aspect_ratio");
    }

    #[test]
    fn submit_without_guest_http_fails_explicitly() {
        let err = dispatch_json(
            "submit",
            r#"{"prompt":"cat","durationSeconds":5,"fps":25,"resolution":"mp:0.4","aspectRatio":"16:9","startFrameB64":"openapi/a.png"}"#,
        )
        .unwrap_err();
        assert!(err.contains("live HTTP"));
    }

    #[test]
    fn query_maps_v2_success_to_canonical_value() {
        let data = map_query_response_json(&serde_json::json!({
            "status": "SUCCESS",
            "results": [{ "url": "https://cdn.example.invalid/out.mp4", "outputType": "mp4" }]
        }))
        .expect("map");
        assert_eq!(data["status"], "succeeded");
        assert_eq!(data["outputs"][0]["value"], "https://cdn.example.invalid/out.mp4");
        assert!(data["outputs"][0].get("url").is_none());
    }

    #[test]
    fn http_200_body_failed_is_failed() {
        let data = map_query_response_json(&serde_json::json!({
            "status": "FAILED",
            "failedReason": { "exception_message": "no" }
        }))
        .expect("map");
        assert_eq!(data["status"], "failed");
    }
}
