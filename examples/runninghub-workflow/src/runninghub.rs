//! RunningHub AI App submit/query mapper. Isolated specimen — not a client vendor.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::config::{
    DURATION_FIELD_NAME, DURATION_NODE_ID, FPS_FIELD_NAME, FPS_NODE_ID, PROMPT_FIELD_NAME,
    PROMPT_NODE_ID, RESOLUTION_FIELD_NAME, RESOLUTION_NODE_ID, SUBMIT_MODE, WEBAPP_ID,
};
use bgx_vendor_adapter_sdk::AppliedParams;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappedStatus {
    Waiting,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
pub struct MappedQuery {
    pub status: MappedStatus,
    pub outputs: Vec<CanonicalOutput>,
    pub vendor_code: Option<String>,
    pub vendor_message: Option<String>,
    pub terminal_failure: Option<bgx_vendor_adapter_sdk::TerminalFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalOutput {
    pub media_kind: String,
    pub source: String,
    pub value: String,
    pub mime: Option<String>,
}

fn push_node(list: &mut Vec<Value>, node_id: &str, field_name: &str, field_value: Value) {
    if node_id.trim().is_empty() {
        return;
    }
    list.push(json!({
        "nodeId": node_id,
        "fieldName": field_name,
        "fieldValue": field_value
    }));
}

pub fn node_info_list(prompt: &str) -> Value {
    let mut list = Vec::new();
    if !prompt.is_empty() {
        push_node(&mut list, PROMPT_NODE_ID, PROMPT_FIELD_NAME, json!(prompt));
    }
    Value::Array(list)
}

/// Map only widgets that have a configured node id. Unconfigured fields stay on the vendor app.
pub fn mapped_node_info(
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
) -> Result<(Value, AppliedParams), String> {
    if SUBMIT_MODE != "mapped_run" {
        return Ok((json!([]), AppliedParams::default()));
    }
    if prompt.is_empty() {
        return Err("submit requires prompt".into());
    }
    if PROMPT_NODE_ID.trim().is_empty() {
        return Err("mapped_run requires PROMPT_NODE_ID".into());
    }
    let mut list = Vec::new();
    let mut applied = AppliedParams::default();
    push_node(&mut list, PROMPT_NODE_ID, PROMPT_FIELD_NAME, json!(prompt));
    if !DURATION_NODE_ID.trim().is_empty() {
        let Some(d) = duration else {
            return Err("duration node is configured; durationSeconds is required".into());
        };
        push_node(&mut list, DURATION_NODE_ID, DURATION_FIELD_NAME, json!(d));
        applied.duration = Some(json!(d));
    }
    if !FPS_NODE_ID.trim().is_empty() {
        if let Some(f) = fps {
            push_node(&mut list, FPS_NODE_ID, FPS_FIELD_NAME, json!(f));
            applied.fps = Some(json!(f));
        }
    }
    if !RESOLUTION_NODE_ID.trim().is_empty() {
        let Some(r) = resolution.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("resolution node is configured; resolution is required".into());
        };
        push_node(
            &mut list,
            RESOLUTION_NODE_ID,
            RESOLUTION_FIELD_NAME,
            json!(r),
        );
        applied.resolution = Some(json!(r));
    }
    Ok((Value::Array(list), applied))
}

pub fn submit_body(prompt: &str) -> Result<Value, String> {
    Ok(submit_body_mapped(prompt, None, None, None)?.0)
}

pub fn submit_body_mapped(
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
) -> Result<(Value, AppliedParams), String> {
    let (node_info, applied) = mapped_node_info(prompt, duration, fps, resolution)?;
    Ok((
        json!({
            "webappId": WEBAPP_ID,
            "apiKey": "",
            "nodeInfoList": node_info
        }),
        applied,
    ))
}

pub fn query_body(task_id: &str) -> Value {
    json!({ "apiKey": "", "taskId": task_id })
}

fn json_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().map(|n| n as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn task_id_from_value(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(s.to_string());
    }
    json_i64(v).map(|n| n.to_string())
}

fn prompt_tips_has_node_errors(tips: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(tips) else {
        return false;
    };
    match v.get("node_errors") {
        Some(Value::Object(m)) => !m.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    }
}

fn submit_data(body: &Value) -> Option<&Value> {
    body.get("data")
}

pub fn parse_submit_task_id(body: &Value) -> Result<String, String> {
    let code = body.get("code").and_then(json_i64).unwrap_or(-1);
    if code != 0 {
        return Err(format!("submit rejected code={code}"));
    }
    if body.get("taskId").is_some() && body.get("data").is_none() {
        return Err("submit must use AI App envelope data.taskId".into());
    }
    let data = submit_data(body).ok_or("submit response missing data")?;
    let tips = data
        .get("promptTips")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if prompt_tips_has_node_errors(tips) {
        return Err("submit promptTips.node_errors is not empty".into());
    }
    data.get("taskId")
        .and_then(task_id_from_value)
        .ok_or_else(|| "submit response missing data.taskId".into())
}

pub fn mime_for_output_type(ext: &str) -> Result<(String, String), String> {
    match ext.trim().to_ascii_lowercase().as_str() {
        "png" => Ok(("image".into(), "image/png".into())),
        "jpg" | "jpeg" => Ok(("image".into(), "image/jpeg".into())),
        "webp" => Ok(("image".into(), "image/webp".into())),
        "gif" => Ok(("image".into(), "image/gif".into())),
        "mp4" => Ok(("video".into(), "video/mp4".into())),
        "webm" => Ok(("video".into(), "video/webm".into())),
        "mov" => Ok(("video".into(), "video/quicktime".into())),
        other => Err(format!("unsupported outputType {other}")),
    }
}

fn ext_from_url(url: &str) -> &str {
    let path = url.split('?').next().unwrap_or(url);
    path.rsplit('.').next().unwrap_or("")
}

pub fn map_file_outputs(data: &Value) -> Result<Vec<CanonicalOutput>, String> {
    let Some(arr) = data.as_array() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in arr {
        let url = item
            .get("fileUrl")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(url) = url else {
            continue;
        };
        let ext = item
            .get("fileType")
            .or_else(|| item.get("outputType"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| ext_from_url(url));
        let (media_kind, mime) = mime_for_output_type(ext)?;
        out.push(CanonicalOutput {
            media_kind,
            source: "url".into(),
            value: url.to_string(),
            mime: Some(mime),
        });
    }
    Ok(out)
}

pub fn parse_query(body: &Value) -> Result<MappedQuery, String> {
    parse_query_for_slot(body, "video")
}

pub fn parse_query_for_slot(body: &Value, slot: &str) -> Result<MappedQuery, String> {
    if body.get("status").is_some() && body.get("code").is_none() {
        return Err("V2 workflow query envelope is not accepted".into());
    }
    let code = body
        .get("code")
        .and_then(json_i64)
        .ok_or("query missing code")?;
    let mapped = match code {
        804 | 813 => MappedStatus::Waiting,
        0 => MappedStatus::Succeeded,
        805 => MappedStatus::Failed,
        other => return Err(format!("unknown vendor code {other}")),
    };
    let data = body.get("data");
    let outputs = match (mapped, data) {
        (MappedStatus::Succeeded, Some(d)) => map_file_outputs(d)?,
        _ => Vec::new(),
    };
    if matches!(mapped, MappedStatus::Succeeded) {
        match slot {
            "video" | "keyframe" => {
                if !outputs.iter().any(|o| o.media_kind == "video") {
                    return Err("SUCCESS without a video URL".into());
                }
            }
            _ => {
                if !outputs.iter().any(|o| o.media_kind == "image") {
                    return Err("SUCCESS without an image URL".into());
                }
            }
        }
    }
    let vendor_code = Some(code.to_string());
    let vendor_message = body
        .get("msg")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "success")
        .map(|s| s.to_string());
    let terminal_failure = if matches!(mapped, MappedStatus::Failed) {
        let reason = data
            .and_then(|d| d.get("failedReason"))
            .cloned()
            .or_else(|| vendor_message.clone().map(serde_json::Value::String))
            .unwrap_or_else(|| serde_json::Value::String("vendor task failed".into()));
        Some(bgx_vendor_adapter_sdk::TerminalFailure::from_vendor_reason(
            "vendorTaskFailed",
            vendor_code.clone(),
            &reason,
        )?)
    } else {
        None
    };
    Ok(MappedQuery {
        status: mapped,
        outputs,
        vendor_code,
        vendor_message,
        terminal_failure,
    })
}

pub fn parse_upload_download(body: &Value) -> Result<String, String> {
    let code = body.get("code").and_then(json_i64).unwrap_or(-1);
    if code != 0 {
        return Err(format!("upload rejected code={code}"));
    }
    body.get("data")
        .and_then(|d| d.get("fileName").or_else(|| d.get("download_url")))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "upload missing data.fileName".into())
}

pub fn outputs_to_json(outputs: &[CanonicalOutput]) -> Value {
    Value::Array(
        outputs
            .iter()
            .map(|o| {
                json!({
                    "mediaKind": o.media_kind,
                    "source": o.source,
                    "value": o.value,
                    "mime": o.mime,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_info_keeps_string_field_value() {
        let list = node_info_list("a cat");
        assert_eq!(list[0]["fieldValue"], "a cat");
        assert_eq!(list[0]["nodeId"], "6");
    }

    #[test]
    fn submit_requires_ai_app_data_task_id() {
        assert!(parse_submit_task_id(&json!({"status":"RUNNING"})).is_err());
        assert!(parse_submit_task_id(&json!({"taskId":"201"})).is_err());
        assert_eq!(
            parse_submit_task_id(&json!({
                "code": 0,
                "data": { "taskId": "201", "taskStatus": "RUNNING" }
            }))
            .unwrap(),
            "201"
        );
        assert_eq!(
            parse_submit_task_id(&json!({
                "code": 0,
                "data": { "taskId": 2098838032504553474_i64 }
            }))
            .unwrap(),
            "2098838032504553474"
        );
        assert!(parse_submit_task_id(&json!({"code": 1, "data": {"taskId":"201"}})).is_err());
    }

    #[test]
    fn submit_rejects_node_errors() {
        let err = parse_submit_task_id(&json!({
            "code": 0,
            "data": {
                "taskId": "201",
                "promptTips": "{\"result\":true,\"error\":null,\"node_errors\":{\"6\":{\"errors\":[\"bad\"]}}}"
            }
        }))
        .unwrap_err();
        assert!(err.contains("node_errors"));
    }

    #[test]
    fn query_codes_804_813_0_805() {
        assert_eq!(
            parse_query_for_slot(&json!({"code": 804}), "video")
                .unwrap()
                .status,
            MappedStatus::Waiting
        );
        assert_eq!(
            parse_query_for_slot(&json!({"code": 813}), "video")
                .unwrap()
                .status,
            MappedStatus::Waiting
        );
        let ok = parse_query(&json!({
            "code": 0,
            "data": [{ "fileUrl": "https://cdn.example.invalid/out.mp4" }]
        }))
        .expect("ok");
        assert_eq!(ok.status, MappedStatus::Succeeded);
        assert_eq!(ok.outputs[0].media_kind, "video");
        let fail = parse_query(&json!({
            "code": 805,
            "data": { "failedReason": { "node_name": "6", "exception_message": "bad" } }
        }))
        .expect("fail");
        assert_eq!(fail.status, MappedStatus::Failed);
        let tf = fail.terminal_failure.expect("structured failure");
        assert_eq!(tf.message, "bad");
        assert_eq!(tf.vendor_code.as_deref(), Some("805"));
        assert!(parse_query(&json!({"code": 999})).is_err());
        assert!(parse_query(&json!({"status": "SUCCESS", "results": []})).is_err());
    }

    #[test]
    fn video_slot_rejects_png_file_url() {
        let err = parse_query_for_slot(
            &json!({
                "code": 0,
                "data": [{ "fileUrl": "https://cdn.example.invalid/a.png" }]
            }),
            "video",
        )
        .unwrap_err();
        assert!(err.contains("video"));
    }

    #[test]
    fn image_slot_requires_image_url() {
        let err = parse_query_for_slot(
            &json!({
                "code": 0,
                "data": [{ "fileUrl": "https://cdn.example.invalid/out.mp4" }]
            }),
            "image",
        )
        .unwrap_err();
        assert!(err.contains("image"));
    }

    #[test]
    fn mime_matrix() {
        assert_eq!(
            mime_for_output_type("jpg").unwrap(),
            ("image".into(), "image/jpeg".into())
        );
        assert_eq!(
            mime_for_output_type("mp4").unwrap(),
            ("video".into(), "video/mp4".into())
        );
        assert!(mime_for_output_type("bin").is_err());
    }

    #[test]
    fn mapped_submit_body_is_ai_app() {
        let (body, applied) = submit_body_mapped("a walking shot", None, None, None).expect("mapped");
        assert_eq!(body["webappId"], WEBAPP_ID);
        assert_eq!(body["apiKey"], "");
        assert_eq!(body["nodeInfoList"][0]["fieldValue"], "a walking shot");
        assert_eq!(body["nodeInfoList"][0]["nodeId"], "6");
        assert!(body.get("addMetadata").is_none());
        assert!(body.get("usePersonalQueue").is_none());
        assert!(applied.duration.is_none());
        let q = query_body("tid-1");
        assert_eq!(q["apiKey"], "");
        assert_eq!(q["taskId"], "tid-1");
    }

    #[test]
    fn mapped_submit_refuses_empty_prompt() {
        assert!(submit_body("").is_err());
    }

    #[test]
    fn empty_timing_bindings_do_not_claim_applied_params() {
        let (body, applied) =
            submit_body_mapped("p", Some(8.0), Some(24.0), Some("1080p")).expect("ok");
        assert_eq!(body["nodeInfoList"].as_array().map(|a| a.len()), Some(1));
        assert!(applied.duration.is_none());
        assert!(applied.fps.is_none());
        assert!(applied.resolution.is_none());
    }

    #[test]
    fn upload_only_accepts_code_zero() {
        assert!(parse_upload_download(&json!({
            "code": 0,
            "data": {"fileName": "api/a.png"}
        }))
        .is_ok());
        assert!(parse_upload_download(&json!({
            "code": 200,
            "data": {"fileName": "api/a.png"}
        }))
        .is_err());
    }
}
