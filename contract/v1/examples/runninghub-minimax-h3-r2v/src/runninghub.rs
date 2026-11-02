//! RunningHub OpenAPI V2 AI App mapper. Isolated H3 example - not a client vendor.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::config::{
    submit_ai_app_url, ASPECT_FIELD_NAME, APP_ID_1SLOT, APP_ID_2SLOT, APP_ID_3SLOT, APP_ID_4SLOT,
    IMAGE_FIELD_NAME, KindWidgetTable, MODEL_ID_1SLOT, MODEL_ID_2SLOT, MODEL_ID_3SLOT,
    MODEL_ID_4SLOT, REF_IMAGE_NODE_IDS_1, REF_IMAGE_NODE_IDS_2, REF_IMAGE_NODE_IDS_3,
    REF_IMAGE_NODE_IDS_4, RESOLUTION_FIELD_NAME, SLOT1_WIDGETS, SLOT2_WIDGETS, SLOT3_WIDGETS,
    SLOT4_WIDGETS, SUBMIT_MODE, WORKFLOW_NODE_MISMATCH_PREFIX,
};
use bgx_vendor_adapter_sdk::AppliedParams;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Slot1,
    Slot2,
    Slot3,
    Slot4,
}

impl ModelKind {
    pub fn app_id(self) -> &'static str {
        match self {
            ModelKind::Slot1 => APP_ID_1SLOT,
            ModelKind::Slot2 => APP_ID_2SLOT,
            ModelKind::Slot3 => APP_ID_3SLOT,
            ModelKind::Slot4 => APP_ID_4SLOT,
        }
    }

    pub fn max_refs(self) -> usize {
        match self {
            ModelKind::Slot1 => 1,
            ModelKind::Slot2 => 2,
            ModelKind::Slot3 => 3,
            ModelKind::Slot4 => 4,
        }
    }

    pub fn ref_node_ids(self) -> &'static [&'static str] {
        match self {
            ModelKind::Slot1 => &REF_IMAGE_NODE_IDS_1,
            ModelKind::Slot2 => &REF_IMAGE_NODE_IDS_2,
            ModelKind::Slot3 => &REF_IMAGE_NODE_IDS_3,
            ModelKind::Slot4 => &REF_IMAGE_NODE_IDS_4,
        }
    }

    pub fn widgets(self) -> KindWidgetTable {
        match self {
            ModelKind::Slot1 => SLOT1_WIDGETS,
            ModelKind::Slot2 => SLOT2_WIDGETS,
            ModelKind::Slot3 => SLOT3_WIDGETS,
            ModelKind::Slot4 => SLOT4_WIDGETS,
        }
    }

    pub fn submit_url(self) -> String {
        submit_ai_app_url(self.app_id())
    }

    pub fn model_id(self) -> &'static str {
        match self {
            ModelKind::Slot1 => MODEL_ID_1SLOT,
            ModelKind::Slot2 => MODEL_ID_2SLOT,
            ModelKind::Slot3 => MODEL_ID_3SLOT,
            ModelKind::Slot4 => MODEL_ID_4SLOT,
        }
    }
}

pub fn parse_model_kind(model: Option<&str>) -> Result<ModelKind, String> {
    match model.map(str::trim).filter(|s| !s.is_empty()) {
        Some(m) if m == MODEL_ID_1SLOT => Ok(ModelKind::Slot1),
        Some(m) if m == MODEL_ID_2SLOT => Ok(ModelKind::Slot2),
        Some(m) if m == MODEL_ID_3SLOT => Ok(ModelKind::Slot3),
        Some(m) if m == MODEL_ID_4SLOT => Ok(ModelKind::Slot4),
        Some(m) => Err(format!(
            "unknown model {m}; expected {MODEL_ID_1SLOT}, {MODEL_ID_2SLOT}, {MODEL_ID_3SLOT} or {MODEL_ID_4SLOT}"
        )),
        None => Err(format!(
            "model is required; expected {MODEL_ID_1SLOT}, {MODEL_ID_2SLOT}, {MODEL_ID_3SLOT} or {MODEL_ID_4SLOT}"
        )),
    }
}

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

fn push_node_str(list: &mut Vec<Value>, node_id: &str, field_name: &str, field_value: &str) {
    if node_id.trim().is_empty() {
        return;
    }
    list.push(json!({
        "nodeId": node_id,
        "fieldName": field_name,
        "fieldValue": field_value
    }));
}

fn format_num_string(n: f64) -> String {
    // no_std wasm has no f64::trunc.
    let whole = n as i64;
    if (n - (whole as f64)).abs() < 1e-12 {
        format!("{whole}")
    } else {
        format!("{n}")
    }
}

fn h3_length_expression(fps: f64) -> Result<String, String> {
    // no_std wasm has no f64::round; only the workbench enum values 24 / 25 / 30 are legal.
    let n = if fps == 24.0 {
        24
    } else if fps == 25.0 {
        25
    } else if fps == 30.0 {
        30
    } else {
        return Err("closed_set_fps:24,25,30".into());
    };
    Ok(format!(
        "max(5, round(a * {n})) + (5 - (max(5, round(a * {n})) % 17)) % 17"
    ))
}

pub fn resolution_combo(resolution: &str) -> Result<String, String> {
    match resolution.trim() {
        "1:1" | "1:1 (Square)" => Ok("1:1 (Square)".into()),
        "16:9" | "16:9 (Widescreen)" => Ok("16:9 (Widescreen)".into()),
        "9:16" | "9:16 (Portrait Widescreen)" => Ok("9:16 (Portrait Widescreen)".into()),
        "4:3" | "4:3 (Standard)" => Ok("4:3 (Standard)".into()),
        other => Err(format!("unsupported aspect {other}")),
    }
}

/// Closed H3 megapixel tiers. Echo `mp:0.4` so RCD-12 matches the workbench id.
pub fn megapixels_from_resolution(resolution: &str) -> Result<(String, f64), String> {
    let t = resolution.trim();
    let n = if let Some(rest) = t.strip_prefix("mp:") {
        rest.parse::<f64>()
            .map_err(|_| format!("unsupported resolution {t}"))?
    } else {
        t.parse::<f64>()
            .map_err(|_| format!("unsupported resolution {t}"))?
    };
    let (id, mp) = if (n - 0.25).abs() < 1e-9 {
        ("mp:0.25", 0.25)
    } else if (n - 0.4).abs() < 1e-9 {
        ("mp:0.4", 0.4)
    } else if (n - 0.6).abs() < 1e-9 {
        ("mp:0.6", 0.6)
    } else if (n - 1.0).abs() < 1e-9 {
        ("mp:1", 1.0)
    } else {
        return Err(format!("unsupported resolution {t}"));
    };
    Ok((id.into(), mp))
}

/// PNG / JPEG / WebP / GIF magic 鈫?filename stem + MIME. Unknown bytes fail.
pub fn sniff_image_filename_and_mime(bytes: &[u8], stem: &str) -> Result<(String, String), String> {
    let (ext, mime) = sniff_image_ext_mime(bytes)?;
    Ok((format!("{stem}.{ext}"), mime.into()))
}

pub fn sniff_image_ext_mime(bytes: &[u8]) -> Result<(&'static str, &'static str), String> {
    if bytes.len() >= 8 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return Ok(("png", "image/png"));
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Ok(("jpg", "image/jpeg"));
    }
    if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Ok(("gif", "image/gif"));
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Ok(("webp", "image/webp"));
    }
    Err("unknown image magic; refused (no default PNG)".into())
}

/// Map only workbench-owned widgets. Weight/VAE/CLIP stay on the vendor graph.
pub fn mapped_node_info(
    kind: ModelKind,
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    aspect: Option<&str>,
    uploaded: &[&str],
) -> Result<(Value, AppliedParams), String> {
    if SUBMIT_MODE != "mapped_run" {
        return Ok((json!([]), AppliedParams::default()));
    }
    if prompt.is_empty() {
        return Err("submit requires prompt".into());
    }
    let w = kind.widgets();
    let mut list = Vec::new();
    let mut applied = AppliedParams::default();
    push_node_str(&mut list, w.prompt_node, w.prompt_field, prompt);
    {
        let Some(d) = duration else {
            return Err("duration node is configured; durationSeconds is required".into());
        };
        if d < 1.0 || d > 15.0 {
            return Err("durationSeconds must be 1 to 15".into());
        }
        push_node_str(
            &mut list,
            w.duration_node,
            w.duration_field,
            &format_num_string(d),
        );
        applied.duration = Some(json!(d));
    }
    if let Some(f) = fps {
        let expr = h3_length_expression(f)?;
        push_node_str(&mut list, w.fps_node, w.fps_field, &format_num_string(f));
        push_node_str(&mut list, w.math_node, w.math_field, &expr);
        applied.fps = Some(json!(f));
    }
    {
        let Some(a) = aspect.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("aspect node is configured; aspect is required".into());
        };
        let combo = resolution_combo(a)?;
        push_node_str(&mut list, w.aspect_node, ASPECT_FIELD_NAME, &combo);
        applied.aspect = Some(json!(a));
    }
    {
        let Some(r) = resolution.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("resolution node is configured; resolution is required".into());
        };
        let (id, mp) = megapixels_from_resolution(r)?;
        push_node_str(
            &mut list,
            w.resolution_node,
            RESOLUTION_FIELD_NAME,
            &format_num_string(mp),
        );
        applied.resolution = Some(json!(id));
    }
    let names: Vec<&str> = uploaded
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        return Err("r2v requires at least one reference image".into());
    }
    if names.len() > kind.max_refs() {
        return Err(format!(
            "r2v model accepts at most {} reference images",
            kind.max_refs()
        ));
    }
    let nodes = kind.ref_node_ids();
    for (i, n) in names.iter().enumerate() {
        let node_id = nodes[i];
        if node_id == "141" {
            return Err(format!(
                "{} graph must not write image onto TurboLoRA node 141",
                kind.model_id()
            ));
        }
        push_node_str(&mut list, node_id, IMAGE_FIELD_NAME, n);
    }
    if kind == ModelKind::Slot1 && list.iter().any(|n| n["nodeId"] == "138") {
        return Err("1-slot nodeInfoList must not contain node 138".into());
    }
    Ok((Value::Array(list), applied))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedImage {
    pub handle: String,
    pub file_name: String,
    pub mime: String,
}

fn reject_legacy_inline_b64(payload: &Value) -> Result<(), String> {
    for k in [
        "referenceImagesB64",
        "reference_images_b64",
        "startFrameB64",
        "endFrameB64",
        "lastFrameB64",
        "firstFrameB64",
        "start_frame_b64",
        "end_frame_b64",
    ] {
        if payload.get(k).is_some() {
            return Err(format!(
                "legacy B64 field {k} forbidden when using video-reference-media-v1"
            ));
        }
    }
    Ok(())
}

fn staged_from_media_object(v: &Value) -> Result<Option<StagedImage>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let handle = v
        .get("handle")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "MediaInputV1.handle required".to_string())?;
    let file_name = v
        .get("fileName")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("ref.png")
        .to_string();
    let mime = v
        .get("mime")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("image/png")
        .to_string();
    Ok(Some(StagedImage {
        handle: handle.to_string(),
        file_name,
        mime,
    }))
}

/// Empty `referenceImages` folds start/end frames. Cap = kind.maxRefs.
/// Inline Base64 fields are forbidden (INV-VENDOR-ADAPTER-SUBMIT-MEDIA).
pub fn collect_r2v_refs(payload: &Value, max_refs: usize) -> Result<Vec<StagedImage>, String> {
    reject_legacy_inline_b64(payload)?;
    let mut refs: Vec<StagedImage> = Vec::new();
    if let Some(arr) = payload.get("referenceImages").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = staged_from_media_object(item)? {
                refs.push(s);
            }
        }
    }
    if refs.is_empty() {
        for key in ["startFrame", "endFrame"] {
            if let Some(s) = payload.get(key).map(staged_from_media_object).transpose()?.flatten()
            {
                refs.push(s);
            }
        }
    }
    if refs.is_empty() {
        return Err("r2v requires at least one reference image".into());
    }
    if refs.len() > max_refs {
        return Err(format!("r2v model accepts at most {max_refs} reference images"));
    }
    Ok(refs)
}

pub fn submit_body(kind: ModelKind, prompt: &str) -> Result<Value, String> {
    Ok(submit_body_mapped(kind, prompt, None, None, None, None, &[])?.0)
}

pub fn submit_body_mapped(
    kind: ModelKind,
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    aspect: Option<&str>,
    uploaded: &[&str],
) -> Result<(Value, AppliedParams), String> {
    let (node_info, applied) = mapped_node_info(
        kind,
        prompt,
        duration,
        fps,
        resolution,
        aspect,
        uploaded,
    )?;
    Ok((
        json!({
            "nodeInfoList": node_info,
            "instanceType": "default",
            "usePersonalQueue": false
        }),
        applied,
    ))
}

pub fn query_body(task_id: &str) -> Value {
    json!({ "taskId": task_id })
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

pub const VENDOR_REJECT_PREFIX: &str = "vendor_reject:";
pub const MISSING_ACCEPT_ID_PREFIX: &str = "missing_accept_id keys=";

pub fn submit_parse_error_code(detail: &str) -> &'static str {
    if detail.starts_with(VENDOR_REJECT_PREFIX)
        || detail.starts_with(WORKFLOW_NODE_MISMATCH_PREFIX)
    {
        "adapterVendorTaskFailed"
    } else {
        "adapterBadOutput"
    }
}

fn json_nonempty_str(body: &Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "success")
        .map(|s| s.chars().take(80).collect())
}

fn top_level_keys(body: &Value) -> String {
    let Some(obj) = body.as_object() else {
        return String::new();
    };
    let mut keys: Vec<String> = obj.keys().cloned().collect();
    keys.sort();
    keys.join(",")
}

fn extract_mismatch_node_id(msg: &str) -> Option<&str> {
    // NODE_INFO_MISMATCH(nodeId=138, fieldName=value, ...
    let key = "nodeId=";
    let idx = msg.find(key)?;
    let rest = &msg[idx + key.len()..];
    let end = rest
        .find(|c: char| c == ',' || c == ')' || c.is_whitespace())
        .unwrap_or(rest.len());
    let id = rest[..end].trim();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

fn submit_vendor_reject_detail(body: &Value) -> Option<String> {
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_uppercase());
    let failed = status.as_deref() == Some("FAILED");
    let err_code = json_nonempty_str(body, "errorCode");
    let err_msg = json_nonempty_str(body, "errorMessage").or_else(|| json_nonempty_str(body, "msg"));
    if !failed && err_code.is_none() && err_msg.is_none() {
        return None;
    }
    let msg = err_msg.unwrap_or_default();
    let base = format!(
        "{VENDOR_REJECT_PREFIX} status={} errorCode={} msg={}",
        status.as_deref().unwrap_or(""),
        err_code.as_deref().unwrap_or(""),
        msg
    );
    let lower = msg.to_ascii_lowercase();
    if lower.contains("node_info_mismatch")
        || lower.contains("node_not_found")
        || err_code.as_deref() == Some("803")
    {
        let node = extract_mismatch_node_id(&msg).unwrap_or("unknown");
        return Some(format!("{WORKFLOW_NODE_MISMATCH_PREFIX}{node}; {base}"));
    }
    Some(base)
}

/// V2 official submit: top-level `taskId`. AI App `data.taskId` is illegal here.
pub fn parse_submit_task_id(body: &Value) -> Result<String, String> {
    if body.get("data").and_then(|d| d.get("taskId")).is_some() {
        return Err("AI App data.taskId envelope is not accepted".into());
    }
    if let Some(detail) = submit_vendor_reject_detail(body) {
        return Err(detail);
    }
    let tips = body
        .get("promptTips")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if prompt_tips_has_node_errors(tips) {
        return Err("submit promptTips.node_errors is not empty".into());
    }
    body.get("taskId")
        .and_then(task_id_from_value)
        .ok_or_else(|| format!("{MISSING_ACCEPT_ID_PREFIX}{}", top_level_keys(body)))
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
            .get("url")
            .or_else(|| item.get("fileUrl"))
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

/// V2 query only: `status` + `results`. AI App `code` 804/813/0/805 is illegal here.
pub fn parse_query_for_slot(body: &Value, slot: &str) -> Result<MappedQuery, String> {
    if body.get("code").is_some() {
        return Err("AI App code query envelope is not accepted".into());
    }
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_uppercase())
        .ok_or("query missing V2 status")?;
    let mapped = match status.as_str() {
        "QUEUED" | "RUNNING" | "CREATE" => MappedStatus::Waiting,
        "SUCCESS" => MappedStatus::Succeeded,
        "FAILED" => MappedStatus::Failed,
        other => return Err(format!("unknown vendor status {other}")),
    };
    let results = body.get("results");
    let outputs = match (mapped, results) {
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
    let vendor_message = body
        .get("errorMessage")
        .or_else(|| body.get("msg"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "success")
        .map(|s| s.to_string());
    let terminal_failure = if matches!(mapped, MappedStatus::Failed) {
        let reason = body
            .get("failedReason")
            .cloned()
            .or_else(|| vendor_message.clone().map(serde_json::Value::String))
            .unwrap_or_else(|| serde_json::Value::String("vendor task failed".into()));
        Some(bgx_vendor_adapter_sdk::TerminalFailure::from_vendor_reason(
            "vendorTaskFailed",
            Some(status.clone()),
            &reason,
        )?)
    } else {
        None
    };
    Ok(MappedQuery {
        status: mapped,
        outputs,
        vendor_code: Some(status),
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
    fn slot2_prompt_field_value_is_string() {
        let (body, _) = submit_body_mapped(
            ModelKind::Slot2,
            "a cat",
            Some(5.0),
            None,
            Some("mp:0.4"),
            Some("16:9"),
            &["a.png"],
        )
        .expect("ok");
        let list = body["nodeInfoList"].as_array().expect("list");
        let p = list.iter().find(|n| n["nodeId"] == "138").expect("138");
        assert_eq!(p["fieldValue"], "a cat");
        assert!(p["fieldValue"].is_string());
    }

    #[test]
    fn submit_requires_v2_top_level_task_id() {
        assert_eq!(
            parse_submit_task_id(&json!({"taskId":"201","status":"RUNNING"})).unwrap(),
            "201"
        );
        let missing = parse_submit_task_id(&json!({"status":"RUNNING"})).unwrap_err();
        assert!(missing.starts_with(MISSING_ACCEPT_ID_PREFIX), "{missing}");
        assert_eq!(submit_parse_error_code(&missing), "adapterBadOutput");
        assert!(parse_submit_task_id(&json!({
            "code": 0,
            "data": { "taskId": "201", "taskStatus": "RUNNING" }
        }))
        .is_err());
        let reject = parse_submit_task_id(&json!({
            "errorCode": "AUTH",
            "errorMessage": "key refused"
        }))
        .unwrap_err();
        assert!(reject.starts_with(VENDOR_REJECT_PREFIX), "{reject}");
        assert_eq!(submit_parse_error_code(&reject), "adapterVendorTaskFailed");
    }

    #[test]
    fn node_mismatch_gets_stable_prefix() {
        let err = parse_submit_task_id(&json!({
            "errorCode": "803",
            "errorMessage": "NODE_INFO_MISMATCH(nodeId=138, fieldName=value, reason=node_not_found_in_workflow)"
        }))
        .unwrap_err();
        assert!(err.starts_with(WORKFLOW_NODE_MISMATCH_PREFIX), "{err}");
        assert!(err.contains("node=138"), "{err}");
        assert_eq!(submit_parse_error_code(&err), "adapterVendorTaskFailed");
    }

    #[test]
    fn submit_urls_are_ai_app_and_do_not_cross() {
        let u1 = ModelKind::Slot1.submit_url();
        let u2 = ModelKind::Slot2.submit_url();
        let u3 = ModelKind::Slot3.submit_url();
        let u4 = ModelKind::Slot4.submit_url();
        assert!(u1.contains("/openapi/v2/run/ai-app/"));
        assert!(u1.contains(APP_ID_1SLOT));
        assert!(u2.contains(APP_ID_2SLOT));
        assert!(u3.contains(APP_ID_3SLOT));
        assert!(u4.contains("/openapi/v2/run/ai-app/"));
        assert!(u4.contains(APP_ID_4SLOT));
        assert!(!u4.contains("/run/workflow/"));
        assert_ne!(u1, u2);
    }

    #[test]
    fn mapped_submit_body_is_ai_app_v2_envelope() {
        let (body, applied) = submit_body_mapped(
            ModelKind::Slot2,
            "a walking shot",
            Some(5.0),
            Some(25.0),
            Some("mp:0.4"),
            Some("16:9"),
            &["openapi/first.jpg", "openapi/second.jpg"],
        )
        .expect("mapped");
        assert!(body.get("addMetadata").is_none());
        assert_eq!(body["usePersonalQueue"], false);
        assert_eq!(body["instanceType"], "default");
        let list = body["nodeInfoList"].as_array().expect("list");
        assert_eq!(list.len(), 8);
        assert!(list.iter().all(|n| n["nodeId"] != "141"));
        assert_eq!(list[0]["nodeId"], "138");
        assert_eq!(list[0]["fieldValue"], "a walking shot");
        assert_eq!(list[1]["fieldValue"], "5");
        assert_eq!(list[2]["fieldValue"], "25");
        let mp = list
            .iter()
            .find(|n| n["nodeId"] == "115" && n["fieldName"] == "megapixels")
            .expect("mp");
        assert_eq!(mp["fieldValue"], "0.4");
        assert_eq!(applied.duration, Some(json!(5.0)));
    }

    #[test]
    fn slot1_never_writes_node_138_and_uses_114() {
        let (body, _) = submit_body_mapped(
            ModelKind::Slot1,
            "p",
            Some(5.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            &["openapi/a.png"],
        )
        .expect("1slot");
        let list = body["nodeInfoList"].as_array().expect("list");
        assert!(list.iter().all(|n| n["nodeId"] != "138"));
        assert!(list.iter().any(|n| n["nodeId"] == "132" && n["fieldName"] == "prompt"));
        assert!(list.iter().any(|n| n["nodeId"] == "114"));
        assert!(list.iter().any(|n| n["nodeId"] == "134"));
    }

    #[test]
    fn slot3_writes_143_not_141() {
        let (body, _) = submit_body_mapped(
            ModelKind::Slot3,
            "p",
            Some(5.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            &["a.png", "b.png", "c.png"],
        )
        .expect("3slot");
        let list = body["nodeInfoList"].as_array().expect("list");
        assert!(list.iter().any(|n| n["nodeId"] == "143"));
        assert!(list.iter().all(|n| n["nodeId"] != "141"));
        assert!(list.iter().all(|n| n["nodeId"] != "144"));
    }

    #[test]
    fn slot4_writes_four_load_image_nodes() {
        let (body, _) = submit_body_mapped(
            ModelKind::Slot4,
            "p",
            Some(5.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            &["a.png", "b.png", "c.png", "d.png"],
        )
        .expect("4slot");
        let list = body["nodeInfoList"].as_array().expect("list");
        for id in ["137", "139", "143", "144"] {
            assert!(list.iter().any(|n| n["nodeId"] == id && n["fieldName"] == "image"));
        }
        assert!(list.iter().all(|n| n["nodeId"] != "141"));
        assert!(list.iter().any(|n| n["nodeId"] == "138" && n["fieldName"] == "value"));
    }

    #[test]
    fn mapped_submit_omits_fps_nodes_when_unspecified() {
        let (body, applied) = submit_body_mapped(
            ModelKind::Slot1,
            "p",
            Some(5.0),
            None,
            Some("mp:0.4"),
            Some("16:9"),
            &["openapi/a.png"],
        )
        .expect("ok");
        let list = body["nodeInfoList"].as_array().expect("list");
        assert!(list.iter().all(|n| !(n["nodeId"] == "131" && n["fieldName"] == "fps")));
        assert!(list.iter().all(|n| n["nodeId"] != "133"));
        assert!(applied.fps.is_none());
    }

    #[test]
    fn mapped_submit_requires_reference_image() {
        let err = submit_body_mapped(
            ModelKind::Slot2,
            "p",
            Some(5.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            &[],
        )
        .unwrap_err();
        assert!(err.contains("at least one reference image"));
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

    #[test]
    fn sniff_png_jpeg_webp_gif_and_unknown() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(
            sniff_image_filename_and_mime(&png, "first").unwrap(),
            ("first.png".into(), "image/png".into())
        );
        assert!(sniff_image_ext_mime(b"not-an-image").is_err());
    }

    fn media(handle: &str) -> Value {
        json!({"handle": handle, "fileName": "r.png", "mime": "image/png"})
    }

    #[test]
    fn collect_refs_respects_kind_max_and_folds_start_end() {
        let one = collect_r2v_refs(&json!({"referenceImages": [media("aaa")]}), 1).unwrap();
        assert_eq!(one[0].handle, "aaa");
        assert!(collect_r2v_refs(&json!({"referenceImages": [media("aaa"), media("bbb")]}), 1).is_err());
        let folded = collect_r2v_refs(
            &json!({ "startFrame": media("start"), "endFrame": media("end") }),
            2,
        )
        .unwrap();
        assert_eq!(folded.len(), 2);
    }
}
