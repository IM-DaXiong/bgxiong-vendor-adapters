//! RunningHub OpenAPI V2 workflow mapper. Isolated H3 example — not a client vendor.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::config::{
    ASPECT_FIELD_NAME, ASPECT_NODE_ID, DURATION_FIELD_NAME, DURATION_NODE_ID, FPS_FIELD_NAME,
    FPS_NODE_ID, IMAGE_FIELD_NAME, MATH_EXPRESSION_FIELD_NAME, MATH_EXPRESSION_NODE_ID,
    PROMPT_FIELD_NAME, PROMPT_NODE_ID, REF_IMAGE_NODE_IDS, RESOLUTION_FIELD_NAME,
    RESOLUTION_NODE_ID, SUBMIT_MODE,
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

/// PNG / JPEG / WebP / GIF magic → filename stem + MIME. Unknown bytes fail.
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

/// Map only widgets that have a configured node id. Unconfigured fields stay on the vendor graph.
pub fn mapped_node_info(
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    aspect: Option<&str>,
    first_image: Option<&str>,
    last_image: Option<&str>,
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
        if d < 1.0 || d > 15.0 {
            return Err("durationSeconds must be 1 to 15".into());
        }
        push_node(
            &mut list,
            DURATION_NODE_ID,
            DURATION_FIELD_NAME,
            json!(d),
        );
        applied.duration = Some(json!(d));
    }
    if !FPS_NODE_ID.trim().is_empty() {
        if let Some(f) = fps {
            let expr = h3_length_expression(f)?;
            push_node(&mut list, FPS_NODE_ID, FPS_FIELD_NAME, json!(f));
            if MATH_EXPRESSION_NODE_ID.trim().is_empty() {
                return Err("fps node requires MATH_EXPRESSION_NODE_ID".into());
            }
            push_node(
                &mut list,
                MATH_EXPRESSION_NODE_ID,
                MATH_EXPRESSION_FIELD_NAME,
                json!(expr),
            );
            applied.fps = Some(json!(f));
        }
    }
    if !ASPECT_NODE_ID.trim().is_empty() {
        let Some(a) = aspect.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("aspect node is configured; aspect is required".into());
        };
        let combo = resolution_combo(a)?;
        push_node(
            &mut list,
            ASPECT_NODE_ID,
            ASPECT_FIELD_NAME,
            json!(combo),
        );
        applied.aspect = Some(json!(a));
    }
    if !RESOLUTION_NODE_ID.trim().is_empty() {
        let Some(r) = resolution.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("resolution node is configured; resolution is required".into());
        };
        let (id, mp) = megapixels_from_resolution(r)?;
        push_node(
            &mut list,
            RESOLUTION_NODE_ID,
            RESOLUTION_FIELD_NAME,
            json!(mp),
        );
        applied.resolution = Some(json!(id));
    }
    let refs: [Option<&str>; 2] = [
        first_image.map(str::trim).filter(|s| !s.is_empty()),
        last_image.map(str::trim).filter(|s| !s.is_empty()),
    ];
    if refs[0].is_none() {
        return Err("r2v turbo requires referenceImagesB64 (at least one image)".into());
    }
    for (i, name) in refs.iter().enumerate() {
        if let Some(n) = name {
            push_node(
                &mut list,
                REF_IMAGE_NODE_IDS[i],
                IMAGE_FIELD_NAME,
                json!(*n),
            );
        }
    }
    Ok((Value::Array(list), applied))
}

/// r2v turbo: 1..=2 refs. Empty `referenceImagesB64` folds start/end frames
/// into 137/139. Does not invent I2V first/last semantics.
pub fn collect_r2v_ref_b64(payload: &Value) -> Result<Vec<String>, String> {
    let mut refs: Vec<String> = payload
        .get("referenceImagesB64")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    if refs.is_empty() {
        for key in ["startFrameB64", "lastFrameB64", "endFrameB64"] {
            if let Some(s) = payload
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                refs.push(s.to_string());
            }
        }
    }
    if refs.is_empty() {
        return Err("r2v turbo requires referenceImagesB64 (at least one image)".into());
    }
    if refs.len() > 2 {
        return Err("r2v turbo accepts at most 2 reference images".into());
    }
    Ok(refs)
}

pub fn submit_body(prompt: &str) -> Result<Value, String> {
    Ok(submit_body_mapped(prompt, None, None, None, None, None, None)?.0)
}

pub fn submit_body_mapped(
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    aspect: Option<&str>,
    first_image: Option<&str>,
    last_image: Option<&str>,
) -> Result<(Value, AppliedParams), String> {
    let (node_info, applied) = mapped_node_info(
        prompt,
        duration,
        fps,
        resolution,
        aspect,
        first_image,
        last_image,
    )?;
    Ok((
        json!({
            "addMetadata": true,
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

/// V2 official submit: top-level `taskId`. AI App `data.taskId` is illegal here.
pub fn parse_submit_task_id(body: &Value) -> Result<String, String> {
    if body.get("data").and_then(|d| d.get("taskId")).is_some() {
        return Err("AI App data.taskId envelope is not accepted".into());
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
        .ok_or_else(|| "submit response missing top-level taskId".into())
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
    let mut vendor_message = body
        .get("errorMessage")
        .or_else(|| body.get("msg"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "success")
        .map(|s| s.to_string());
    if matches!(mapped, MappedStatus::Failed) {
        if let Some(fr) = body.get("failedReason") {
            vendor_message = Some(fr.to_string());
        }
    }
    Ok(MappedQuery {
        status: mapped,
        outputs,
        vendor_code: Some(status),
        vendor_message,
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
        assert_eq!(list[0]["nodeId"], "138");
        assert_eq!(list[0]["fieldName"], "value");
    }

    #[test]
    fn submit_requires_v2_top_level_task_id() {
        assert_eq!(
            parse_submit_task_id(&json!({"taskId":"201","status":"RUNNING"})).unwrap(),
            "201"
        );
        assert_eq!(
            parse_submit_task_id(&json!({"taskId": 2098838032504553474_i64})).unwrap(),
            "2098838032504553474"
        );
        assert!(parse_submit_task_id(&json!({"status":"RUNNING"})).is_err());
        assert!(parse_submit_task_id(&json!({
            "code": 0,
            "data": { "taskId": "201", "taskStatus": "RUNNING" }
        }))
        .is_err());
    }

    #[test]
    fn submit_rejects_node_errors() {
        let err = parse_submit_task_id(&json!({
            "taskId": "201",
            "promptTips": "{\"result\":true,\"error\":null,\"node_errors\":{\"134\":{\"errors\":[\"bad\"]}}}"
        }))
        .unwrap_err();
        assert!(err.contains("node_errors"));
    }

    #[test]
    fn query_accepts_v2_status_rejects_ai_app_codes() {
        assert_eq!(
            parse_query_for_slot(&json!({"status": "RUNNING"}), "video")
                .unwrap()
                .status,
            MappedStatus::Waiting
        );
        let ok = parse_query(&json!({
            "status": "SUCCESS",
            "results": [{ "url": "https://cdn.example.invalid/out.mp4", "outputType": "mp4" }]
        }))
        .expect("v2");
        assert_eq!(ok.status, MappedStatus::Succeeded);
        assert_eq!(ok.outputs[0].media_kind, "video");
        let fail = parse_query(&json!({
            "status": "FAILED",
            "failedReason": { "node_name": "134", "exception_message": "bad" }
        }))
        .expect("fail");
        assert_eq!(fail.status, MappedStatus::Failed);
        assert!(fail.vendor_message.unwrap().contains("exception_message"));
        assert!(parse_query(&json!({"code": 804})).is_err());
        assert!(parse_query(&json!({"code": 813})).is_err());
        assert!(parse_query(&json!({"code": 0, "data": []})).is_err());
        assert!(parse_query(&json!({"code": 805})).is_err());
        assert!(parse_query(&json!({"status": "SUCCESS", "results": []})).is_err());
    }

    #[test]
    fn video_slot_rejects_png_file_url() {
        let err = parse_query_for_slot(
            &json!({
                "status": "SUCCESS",
                "results": [{ "url": "https://cdn.example.invalid/a.png" }]
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
                "status": "SUCCESS",
                "results": [{ "url": "https://cdn.example.invalid/out.mp4" }]
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
    fn sniff_png_jpeg_webp_gif_and_unknown() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(
            sniff_image_filename_and_mime(&png, "first").unwrap(),
            ("first.png".into(), "image/png".into())
        );
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0];
        assert_eq!(
            sniff_image_filename_and_mime(&jpeg, "last").unwrap(),
            ("last.jpg".into(), "image/jpeg".into())
        );
        let mut webp = [0u8; 12];
        webp[..4].copy_from_slice(b"RIFF");
        webp[8..12].copy_from_slice(b"WEBP");
        assert_eq!(sniff_image_ext_mime(&webp).unwrap(), ("webp", "image/webp"));
        assert_eq!(
            sniff_image_ext_mime(b"GIF89a....").unwrap(),
            ("gif", "image/gif")
        );
        assert!(sniff_image_ext_mime(b"not-an-image").is_err());
    }

    #[test]
    fn mapped_submit_body_is_v2_not_ai_app() {
        assert!(crate::config::submit_url().contains("2100758868111486978"));
        let (body, applied) = submit_body_mapped(
            "a walking shot",
            Some(5.0),
            Some(25.0),
            Some("mp:0.4"),
            Some("16:9"),
            Some("openapi/first.jpg"),
            Some("openapi/second.jpg"),
        )
        .expect("mapped");
        assert_eq!(body["addMetadata"], true);
        assert_eq!(body["usePersonalQueue"], false);
        assert!(body.get("webappId").is_none());
        assert!(body.get("apiKey").is_none());
        let list = body["nodeInfoList"].as_array().expect("list");
        assert_eq!(list.len(), 8);
        assert!(list.iter().all(|n| n["nodeId"] != "141"));
        assert_eq!(list[0]["nodeId"], "138");
        assert_eq!(list[0]["fieldName"], "value");
        assert_eq!(list[0]["fieldValue"], "a walking shot");
        assert_eq!(list[1]["nodeId"], "132");
        assert_eq!(list[1]["fieldName"], "value");
        assert_eq!(list[1]["fieldValue"], 5.0);
        assert_eq!(list[2]["nodeId"], "130");
        assert_eq!(list[2]["fieldValue"], 25.0);
        assert_eq!(list[3]["nodeId"], "131");
        assert_eq!(list[3]["fieldName"], "expression");
        assert!(list[3]["fieldValue"].as_str().unwrap().contains("a * 25"));
        let aspect = list
            .iter()
            .find(|n| n["nodeId"] == "115" && n["fieldName"] == "aspect_ratio")
            .expect("aspect");
        assert_eq!(aspect["fieldValue"], "16:9 (Widescreen)");
        let mp = list
            .iter()
            .find(|n| n["nodeId"] == "115" && n["fieldName"] == "megapixels")
            .expect("mp");
        assert_eq!(mp["fieldValue"], 0.4);
        assert_eq!(list[6]["nodeId"], "137");
        assert_eq!(list[6]["fieldValue"], "openapi/first.jpg");
        assert_eq!(list[7]["nodeId"], "139");
        assert_eq!(list[7]["fieldValue"], "openapi/second.jpg");
        assert_eq!(applied.duration, Some(json!(5.0)));
        assert_eq!(applied.fps, Some(json!(25.0)));
        assert_eq!(applied.aspect, Some(json!("16:9")));
        assert_eq!(applied.resolution, Some(json!("mp:0.4")));
        let q = query_body("tid-1");
        assert_eq!(q["taskId"], "tid-1");
        assert!(q.get("apiKey").is_none());
    }

    #[test]
    fn duration_writes_user_seconds_not_impact_switch() {
        for d in [1.0, 4.0, 15.0] {
            let (body, applied) = submit_body_mapped(
                "p",
                Some(d),
                Some(24.0),
                Some("mp:0.4"),
                Some("16:9"),
                Some("openapi/a.png"),
                None,
            )
            .expect("ok");
            let list = body["nodeInfoList"].as_array().expect("list");
            let dur = list.iter().find(|n| n["nodeId"] == "132").expect("132");
            assert_eq!(dur["fieldName"], "value");
            assert_eq!(dur["fieldValue"], d);
            assert_eq!(applied.duration, Some(json!(d)));
            assert!(list.iter().all(|n| n["nodeId"] != "205"));
            assert!(list.iter().all(|n| n["nodeId"] != "141"));
        }
        let (body, _) = submit_body_mapped(
            "p",
            Some(15.0),
            Some(24.0),
            Some("mp:0.25"),
            Some("1:1"),
            Some("openapi/a.png"),
            None,
        )
        .expect("15");
        let list = body["nodeInfoList"].as_array().expect("list");
        assert_eq!(
            list.iter()
                .find(|n| n["nodeId"] == "115" && n["fieldName"] == "aspect_ratio")
                .unwrap()["fieldValue"],
            "1:1 (Square)"
        );
        assert_eq!(
            list.iter()
                .find(|n| n["nodeId"] == "115" && n["fieldName"] == "megapixels")
                .unwrap()["fieldValue"],
            0.25
        );
        assert_eq!(list.iter().filter(|n| n["nodeId"] == "137").count(), 1);
        assert_eq!(list.iter().filter(|n| n["nodeId"] == "139").count(), 0);
    }

    #[test]
    fn mapped_submit_requires_reference_image() {
        let err = submit_body_mapped("p", Some(5.0), Some(24.0), Some("mp:0.4"), Some("16:9"), None, None)
            .unwrap_err();
        assert!(err.contains("referenceImagesB64"));
    }

    #[test]
    fn collect_refs_allows_one_or_two_and_folds_start_end() {
        let one = collect_r2v_ref_b64(&json!({"referenceImagesB64": ["aaa"]})).unwrap();
        assert_eq!(one, vec!["aaa"]);
        let two = collect_r2v_ref_b64(&json!({"referenceImagesB64": ["aaa", "bbb"]})).unwrap();
        assert_eq!(two, vec!["aaa", "bbb"]);
        let folded = collect_r2v_ref_b64(&json!({
            "startFrameB64": "start",
            "endFrameB64": "end"
        }))
        .unwrap();
        assert_eq!(folded, vec!["start", "end"]);
        let start_only = collect_r2v_ref_b64(&json!({"startFrameB64": "only"})).unwrap();
        assert_eq!(start_only, vec!["only"]);
        assert!(collect_r2v_ref_b64(&json!({})).is_err());
        assert!(collect_r2v_ref_b64(&json!({"referenceImagesB64": ["a", "b", "c"]})).is_err());
        let prefers_refs = collect_r2v_ref_b64(&json!({
            "referenceImagesB64": ["ref"],
            "startFrameB64": "ignored"
        }))
        .unwrap();
        assert_eq!(prefers_refs, vec!["ref"]);
    }

    #[test]
    fn mapped_submit_refuses_empty_prompt() {
        assert!(submit_body("").is_err());
    }

    #[test]
    fn mapped_submit_requires_timing_when_nodes_bound() {
        let img = Some("openapi/a.png");
        assert!(submit_body_mapped("p", None, None, None, None, img, None).is_err());
        assert!(submit_body_mapped("p", Some(5.0), Some(23.0), Some("mp:0.4"), Some("16:9"), img, None).is_err());
        assert!(submit_body_mapped("p", Some(5.0), Some(25.0), None, Some("16:9"), img, None).is_err());
        assert!(submit_body_mapped("p", Some(5.0), Some(25.0), Some("mp:0.4"), None, img, None).is_err());
        assert!(submit_body_mapped("p", Some(16.0), Some(25.0), Some("mp:0.4"), Some("16:9"), img, None).is_err());
    }

    #[test]
    fn mapped_submit_omits_fps_nodes_when_unspecified() {
        let img = Some("openapi/a.png");
        let (body, applied) = submit_body_mapped(
            "p",
            Some(5.0),
            None,
            Some("mp:0.4"),
            Some("16:9"),
            img,
            None,
        )
        .expect("ok");
        let list = body["nodeInfoList"].as_array().expect("list");
        assert!(list.iter().all(|n| n["nodeId"] != "130"));
        assert!(list.iter().all(|n| n["nodeId"] != "131"));
        assert!(applied.fps.is_none());
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
