//! Comfy `/prompt` mapper for pinned MiniMax H3 API graphs.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{json, Map, Value};

use crate::config::{
    join_url, DURATION_FIELD_NAME, DURATION_NODE_ID, FPS_FIELD_NAME, FPS_NODE_ID,
    I2V_LOAD_IMAGE_NODE_ID, MATH_EXPRESSION_FIELD_NAME, MATH_EXPRESSION_NODE_ID, MODEL_ID,
    MODEL_ID_I2V_TURBO, MODEL_ID_R2V_TURBO, PROMPT_FIELD_NAME, PROMPT_NODE_ID, R2V_DURATION_NODE_ID,
    R2V_LOAD_IMAGE_NODE_IDS, R2V_LORA_NODE_ID, R2V_MATH_NODE_ID, R2V_PROMPT_NODE_ID, R2V_REF_NODE_ID,
    RESOLUTION_FIELD_NAME, RESOLUTION_NODE_ID, SAVE_VIDEO_NODE_ID, TURBO_SWITCH_NODE_ID,
};
use bgx_vendor_adapter_sdk::AppliedParams;

pub const API_PROMPT_JSON: &str = include_str!("../workflows/h3.t2v.api.json");
pub const API_I2V_TURBO_JSON: &str = include_str!("../workflows/h3.i2v.turbo.api.json");
pub const API_R2V_TURBO_JSON: &str = include_str!("../workflows/h3.r2v.turbo.api.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    T2v,
    I2vTurbo,
    R2vTurbo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappedStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
pub struct MappedQuery {
    pub status: MappedStatus,
    pub outputs: Vec<CanonicalOutput>,
    pub vendor_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalOutput {
    pub media_kind: String,
    pub source: String,
    pub value: String,
    pub mime: Option<String>,
}

/// Host-injected origin only. Empty payload (import L3 / settings L3) must
/// not fall back to `COMFY_BASE_URL` — that would force host-http.
pub fn configured_endpoint_base_url(payload: &Value) -> Option<String> {
    payload
        .get("endpointBaseUrl")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            payload
                .get("extra")
                .and_then(|e| e.get("endpointBaseUrl"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .map(|s| s.trim_end_matches('/').to_string())
}

pub fn resolve_base_url(payload: &Value) -> String {
    configured_endpoint_base_url(payload).unwrap_or_else(|| {
        crate::config::COMFY_BASE_URL
            .trim_end_matches('/')
            .to_string()
    })
}

pub fn offline_probe_doc() -> Value {
    json!({"ok": true, "live": false})
}

fn h3_length_expression(fps: f64) -> Result<String, String> {
    let n = if fps == 24.0 {
        24
    } else if fps == 25.0 {
        25
    } else if fps == 30.0 {
        30
    } else {
        return Err("fps must be 24, 25 or 30".into());
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
        other => Err(format!("unsupported resolution {other}")),
    }
}

fn require_node_inputs<'a>(
    prompt: &'a mut Value,
    id: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    prompt
        .get_mut(id)
        .and_then(|n| n.get_mut("inputs"))
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| format!("pinned API prompt missing node {id}"))
}

fn apply_timing(
    graph: &mut Value,
    duration_node: &str,
    math_node: &str,
    duration: f64,
    fps: f64,
    resolution: &str,
) -> Result<AppliedParams, String> {
    if duration < 1.0 || duration > 15.0 {
        return Err("durationSeconds must be 1..=15".into());
    }
    let combo = resolution_combo(resolution)?;
    let expr = h3_length_expression(fps)?;
    require_node_inputs(graph, duration_node)?.insert(DURATION_FIELD_NAME.into(), json!(duration));
    require_node_inputs(graph, FPS_NODE_ID)?.insert(FPS_FIELD_NAME.into(), json!(fps));
    require_node_inputs(graph, math_node)?
        .insert(MATH_EXPRESSION_FIELD_NAME.into(), Value::String(expr));
    require_node_inputs(graph, RESOLUTION_NODE_ID)?
        .insert(RESOLUTION_FIELD_NAME.into(), Value::String(combo));
    Ok(AppliedParams {
        duration: Some(json!(duration)),
        fps: Some(json!(fps)),
        resolution: Some(json!(resolution.trim())),
        aspect: None,
    })
}

pub fn parse_model_kind(model: Option<&str>) -> Result<ModelKind, String> {
    match model.map(str::trim).filter(|s| !s.is_empty()) {
        Some(m) if m == MODEL_ID => Ok(ModelKind::T2v),
        Some(m) if m == MODEL_ID_I2V_TURBO => Ok(ModelKind::I2vTurbo),
        Some(m) if m == MODEL_ID_R2V_TURBO => Ok(ModelKind::R2vTurbo),
        Some(m) => Err(format!(
            "unknown model {m}; expected {MODEL_ID}, {MODEL_ID_I2V_TURBO} or {MODEL_ID_R2V_TURBO}"
        )),
        None => Err(format!(
            "model is required; expected {MODEL_ID}, {MODEL_ID_I2V_TURBO} or {MODEL_ID_R2V_TURBO}"
        )),
    }
}

pub fn require_t2v_model(model: Option<&str>) -> Result<(), String> {
    match parse_model_kind(model)? {
        ModelKind::T2v => Ok(()),
        other => Err(format!("expected {MODEL_ID}, got {other:?}")),
    }
}

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

pub fn parse_comfy_upload_name(body: &Value) -> Result<String, String> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "upload/image missing name".to_string())?;
    let sub = body
        .get("subfolder")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    if sub.is_empty() {
        Ok(name.into())
    } else {
        Ok(format!("{sub}/{name}"))
    }
}

pub fn decode_b64(input: &str) -> Result<Vec<u8>, String> {
    let s = input.split(',').next_back().unwrap_or(input).trim();
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut n = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => continue,
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return Err("invalid base64".into()),
        };
        buf = (buf << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((buf >> n) as u8);
            buf &= (1 << n) - 1;
        }
    }
    if out.is_empty() {
        return Err("empty base64".into());
    }
    Ok(out)
}

pub fn patch_t2v_prompt(
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
) -> Result<(Value, AppliedParams), String> {
    if prompt.trim().is_empty() {
        return Err("prompt is required for h3.t2v".into());
    }
    let duration = duration.ok_or("durationSeconds is required")?;
    let fps = fps.ok_or("fps is required")?;
    let resolution = resolution.ok_or("resolution is required")?;
    let mut graph: Value =
        serde_json::from_str(API_PROMPT_JSON).map_err(|e| format!("pinned API json: {e}"))?;
    require_node_inputs(&mut graph, PROMPT_NODE_ID)?
        .insert(PROMPT_FIELD_NAME.into(), Value::String(prompt.into()));
    let applied = apply_timing(
        &mut graph,
        DURATION_NODE_ID,
        MATH_EXPRESSION_NODE_ID,
        duration,
        fps,
        resolution,
    )?;
    Ok((graph, applied))
}

pub fn patch_i2v_turbo_prompt(
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    first_image: &str,
) -> Result<(Value, AppliedParams), String> {
    if prompt.trim().is_empty() {
        return Err("prompt is required for h3.i2v.turbo".into());
    }
    if first_image.trim().is_empty() {
        return Err("h3.i2v.turbo requires a first-frame image".into());
    }
    let duration = duration.ok_or("durationSeconds is required")?;
    let fps = fps.ok_or("fps is required")?;
    let resolution = resolution.ok_or("resolution is required")?;
    let mut graph: Value =
        serde_json::from_str(API_I2V_TURBO_JSON).map_err(|e| format!("pinned i2v turbo json: {e}"))?;
    require_node_inputs(&mut graph, PROMPT_NODE_ID)?
        .insert(PROMPT_FIELD_NAME.into(), Value::String(prompt.into()));
    require_node_inputs(&mut graph, I2V_LOAD_IMAGE_NODE_ID)?
        .insert("image".into(), Value::String(first_image.into()));
    let turbo_on = require_node_inputs(&mut graph, TURBO_SWITCH_NODE_ID)?
        .get("value")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !turbo_on {
        return Err("h3.i2v.turbo pin must keep lightning switch true".into());
    }
    let applied = apply_timing(
        &mut graph,
        DURATION_NODE_ID,
        MATH_EXPRESSION_NODE_ID,
        duration,
        fps,
        resolution,
    )?;
    Ok((graph, applied))
}

pub fn patch_r2v_turbo_prompt(
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    image_names: &[String],
) -> Result<(Value, AppliedParams), String> {
    if prompt.trim().is_empty() {
        return Err("prompt is required for h3.r2v.turbo".into());
    }
    if image_names.is_empty() {
        return Err("h3.r2v.turbo requires referenceImagesB64 (at least one image)".into());
    }
    if image_names.len() > R2V_LOAD_IMAGE_NODE_IDS.len() {
        return Err("h3.r2v.turbo accepts at most 3 reference images".into());
    }
    let duration = duration.ok_or("durationSeconds is required")?;
    let fps = fps.ok_or("fps is required")?;
    let resolution = resolution.ok_or("resolution is required")?;
    let mut graph: Value =
        serde_json::from_str(API_R2V_TURBO_JSON).map_err(|e| format!("pinned r2v turbo json: {e}"))?;
    require_node_inputs(&mut graph, R2V_PROMPT_NODE_ID)?
        .insert("value".into(), Value::String(prompt.into()));
    let lora = require_node_inputs(&mut graph, R2V_LORA_NODE_ID)?
        .get("lora_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !lora.contains("turbo") {
        return Err("h3.r2v.turbo pin missing turbo LoRA".into());
    }
    let steps = graph
        .get("124")
        .and_then(|n| n.get("inputs"))
        .and_then(|i| i.get("steps"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if steps != 8 {
        return Err("h3.r2v.turbo pin must keep scheduler steps at 8 (no 20-step fallback)".into());
    }
    {
        let inputs = require_node_inputs(&mut graph, R2V_REF_NODE_ID)?;
        for i in 0..R2V_LOAD_IMAGE_NODE_IDS.len() {
            inputs.remove(&format!("ref_images.ref_image_{i}"));
        }
        for (i, _name) in image_names.iter().enumerate() {
            let load_id = R2V_LOAD_IMAGE_NODE_IDS[i];
            inputs.insert(format!("ref_images.ref_image_{i}"), json!([load_id, 0]));
        }
    }
    for (i, name) in image_names.iter().enumerate() {
        require_node_inputs(&mut graph, R2V_LOAD_IMAGE_NODE_IDS[i])?
            .insert("image".into(), Value::String(name.clone()));
    }
    let applied = apply_timing(
        &mut graph,
        R2V_DURATION_NODE_ID,
        R2V_MATH_NODE_ID,
        duration,
        fps,
        resolution,
    )?;
    Ok((graph, applied))
}

pub fn submit_body(
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    client_id: &str,
) -> Result<(Value, AppliedParams), String> {
    let (api, applied) = patch_t2v_prompt(prompt, duration, fps, resolution)?;
    Ok((
        json!({
            "prompt": api,
            "client_id": client_id
        }),
        applied,
    ))
}

pub fn submit_body_for_kind(
    kind: ModelKind,
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    first_image: Option<&str>,
    ref_images: &[String],
    client_id: &str,
) -> Result<(Value, AppliedParams), String> {
    let (api, applied) = match kind {
        ModelKind::T2v => patch_t2v_prompt(prompt, duration, fps, resolution)?,
        ModelKind::I2vTurbo => {
            let name = first_image
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("h3.i2v.turbo requires a first-frame image")?;
            patch_i2v_turbo_prompt(prompt, duration, fps, resolution, name)?
        }
        ModelKind::R2vTurbo => {
            patch_r2v_turbo_prompt(prompt, duration, fps, resolution, ref_images)?
        }
    };
    Ok((
        json!({
            "prompt": api,
            "client_id": client_id
        }),
        applied,
    ))
}

pub fn parse_submit_prompt_id(body: &Value) -> Result<String, String> {
    if let Some(err) = body.get("error").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        return Err(err.into());
    }
    if let Some(obj) = body.get("node_errors").and_then(|v| v.as_object()) {
        if !obj.is_empty() {
            return Err(format!("Comfy node_errors: {obj:?}"));
        }
    }
    body.get("prompt_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "Comfy submit missing prompt_id".into())
}

fn query_percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

fn view_url(base: &str, filename: &str, subfolder: &str, ty: &str) -> String {
    let mut q = format!(
        "filename={}&type={}",
        query_percent_encode(filename),
        query_percent_encode(ty)
    );
    if !subfolder.is_empty() {
        q.push_str("&subfolder=");
        q.push_str(&query_percent_encode(subfolder));
    }
    join_url(base, &format!("/view?{q}"))
}

fn collect_media_outputs(outputs: &Value, base: &str) -> Vec<CanonicalOutput> {
    let mut found = Vec::new();
    let Some(map) = outputs.as_object() else {
        return found;
    };
    for (_nid, node_out) in map {
        let Some(obj) = node_out.as_object() else {
            continue;
        };
        for key in ["videos", "gifs", "images", "files"] {
            let Some(arr) = obj.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for item in arr {
                let Some(name) = item.get("filename").and_then(|v| v.as_str()) else {
                    continue;
                };
                let lower = name.to_ascii_lowercase();
                let is_video = lower.ends_with(".mp4")
                    || lower.ends_with(".webm")
                    || lower.ends_with(".mkv")
                    || lower.ends_with(".mov")
                    || key == "videos"
                    || key == "gifs";
                if !is_video {
                    continue;
                }
                let sub = item
                    .get("subfolder")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("output");
                found.push(CanonicalOutput {
                    media_kind: "video".into(),
                    source: "url".into(),
                    value: view_url(base, name, sub, ty),
                    mime: Some("video/mp4".into()),
                });
            }
        }
    }
    let _ = SAVE_VIDEO_NODE_ID;
    found
}

fn inner_history_entry<'a>(body: &'a Value, prompt_id: &str) -> Option<&'a Value> {
    if let Some(v) = body.get(prompt_id) {
        return Some(v);
    }
    body.as_object().and_then(|m| m.values().next())
}

pub fn parse_history(body: &Value, prompt_id: &str, base: &str) -> Result<MappedQuery, String> {
    if body.as_object().map(|m| m.is_empty()).unwrap_or(false) {
        return Ok(MappedQuery {
            status: MappedStatus::Queued,
            outputs: Vec::new(),
            vendor_message: None,
        });
    }
    let Some(entry) = inner_history_entry(body, prompt_id) else {
        return Ok(MappedQuery {
            status: MappedStatus::Queued,
            outputs: Vec::new(),
            vendor_message: None,
        });
    };
    let status_obj = entry.get("status");
    let status_str = status_obj
        .and_then(|s| s.get("status_str"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let completed = status_obj
        .and_then(|s| s.get("completed"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if status_str.contains("error") || status_str == "failed" {
        return Ok(MappedQuery {
            status: MappedStatus::Failed,
            outputs: Vec::new(),
            vendor_message: Some(status_str),
        });
    }
    let outputs = entry.get("outputs").cloned().unwrap_or(json!({}));
    let media = collect_media_outputs(&outputs, base);
    if completed || !media.is_empty() {
        if media.is_empty() {
            return Err("Comfy completed without a video file in history outputs".into());
        }
        return Ok(MappedQuery {
            status: MappedStatus::Succeeded,
            outputs: media,
            vendor_message: None,
        });
    }
    Ok(MappedQuery {
        status: MappedStatus::Running,
        outputs: Vec::new(),
        vendor_message: None,
    })
}
