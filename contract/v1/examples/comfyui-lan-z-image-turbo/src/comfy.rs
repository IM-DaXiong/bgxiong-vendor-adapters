//! Comfy `/prompt` mapper for pinned Z-Image-Turbo API graphs (t2i only).

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{json, Map, Value};

use crate::config::{
    join_url, size_label, LATENT_NODE_ID, MODEL_ID_1080, MODEL_ID_4K, MODEL_ID_TURBO,
    NATIVE_H_1080, NATIVE_H_4K, NATIVE_H_TURBO, NATIVE_W_1080, NATIVE_W_4K, NATIVE_W_TURBO,
    PROMPT_FIELD_NAME, PROMPT_NODE_ID, SAMPLER_NODE_ID, SAVE_IMAGE_NODE_ID,
};

pub const API_TURBO_JSON: &str = include_str!("../workflows/z.turbo.api.json");
pub const API_1080_JSON: &str = include_str!("../workflows/z.turbo.1080.api.json");
pub const API_4K_JSON: &str = include_str!("../workflows/z.turbo.4k.api.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Turbo,
    Turbo1080,
    Turbo4k,
}

impl ModelKind {
    pub fn native_wh(self) -> (u32, u32) {
        match self {
            ModelKind::Turbo => (NATIVE_W_TURBO, NATIVE_H_TURBO),
            ModelKind::Turbo1080 => (NATIVE_W_1080, NATIVE_H_1080),
            ModelKind::Turbo4k => (NATIVE_W_4K, NATIVE_H_4K),
        }
    }

    pub fn pinned_json(self) -> &'static str {
        match self {
            ModelKind::Turbo => API_TURBO_JSON,
            ModelKind::Turbo1080 => API_1080_JSON,
            ModelKind::Turbo4k => API_4K_JSON,
        }
    }

    pub fn model_id(self) -> &'static str {
        match self {
            ModelKind::Turbo => MODEL_ID_TURBO,
            ModelKind::Turbo1080 => MODEL_ID_1080,
            ModelKind::Turbo4k => MODEL_ID_4K,
        }
    }
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

pub fn parse_model_kind(model: Option<&str>) -> Result<ModelKind, String> {
    match model.map(str::trim).filter(|s| !s.is_empty()) {
        Some(m) if m == MODEL_ID_TURBO => Ok(ModelKind::Turbo),
        Some(m) if m == MODEL_ID_1080 => Ok(ModelKind::Turbo1080),
        Some(m) if m == MODEL_ID_4K => Ok(ModelKind::Turbo4k),
        Some(m) => Err(format!(
            "unknown model {m}; expected {MODEL_ID_TURBO}, {MODEL_ID_1080} or {MODEL_ID_4K}"
        )),
        None => Err(format!(
            "model is required; expected {MODEL_ID_TURBO}, {MODEL_ID_1080} or {MODEL_ID_4K}"
        )),
    }
}

/// Parse optional `size` (`WxH` or `W*H`). Empty / absent => None.
pub fn parse_size_wh(size: Option<&str>) -> Result<Option<(u32, u32)>, String> {
    let Some(raw) = size.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let sep = if raw.contains('x') {
        'x'
    } else if raw.contains('X') {
        'X'
    } else if raw.contains('*') {
        '*'
    } else {
        return Err(format!("invalid size {raw}; expected WxH"));
    };
    let mut parts = raw.split(sep);
    let w = parts
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .ok_or_else(|| format!("invalid size {raw}; expected WxH"))?;
    let h = parts
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .ok_or_else(|| format!("invalid size {raw}; expected WxH"))?;
    if parts.next().is_some() || w == 0 || h == 0 {
        return Err(format!("invalid size {raw}; expected WxH"));
    }
    Ok(Some((w, h)))
}

fn reject_refs(payload: &Value) -> Result<(), String> {
    let has_refs = payload
        .get("referenceImagesB64")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().any(|x| x.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)))
        .unwrap_or(false);
    let has_one = payload
        .get("referenceImageB64")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some();
    if has_refs || has_one {
        return Err("z-image turbo is text-to-image only; reference images are not supported".into());
    }
    Ok(())
}

fn next_seed() -> u64 {
    // Fallback when client_id hash collapses to 0.
    0x5a1c_0ffe_e11c_afeu64
}

fn seed_from_client(client_id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in client_id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    if h == 0 {
        next_seed()
    } else {
        h
    }
}

pub fn patch_prompt(
    kind: ModelKind,
    prompt: &str,
    size: Option<&str>,
    seed: Option<u64>,
    client_id: &str,
) -> Result<Value, String> {
    if prompt.trim().is_empty() {
        return Err("prompt is required".into());
    }
    let (nw, nh) = kind.native_wh();
    if let Some((w, h)) = parse_size_wh(size)? {
        if w != nw || h != nh {
            return Err(format!(
                "size {}x{} does not match model {} native {} (no silent resize)",
                w,
                h,
                kind.model_id(),
                size_label(nw, nh)
            ));
        }
    }
    let mut graph: Value =
        serde_json::from_str(kind.pinned_json()).map_err(|e| format!("pinned API json: {e}"))?;
    require_node_inputs(&mut graph, PROMPT_NODE_ID)?
        .insert(PROMPT_FIELD_NAME.into(), Value::String(prompt.into()));
    {
        let latent = require_node_inputs(&mut graph, LATENT_NODE_ID)?;
        latent.insert("width".into(), json!(nw));
        latent.insert("height".into(), json!(nh));
    }
    {
        let sampler = require_node_inputs(&mut graph, SAMPLER_NODE_ID)?;
        let seed_v = seed.unwrap_or_else(|| seed_from_client(client_id));
        sampler.insert("seed".into(), json!(seed_v));
        // Keep pin steps/cfg/sampler/scheduler/denoise as-is (do not rewrite).
        let _ = SAVE_IMAGE_NODE_ID;
    }
    Ok(graph)
}

pub fn submit_body(
    kind: ModelKind,
    prompt: &str,
    size: Option<&str>,
    seed: Option<u64>,
    client_id: &str,
) -> Result<Value, String> {
    let api = patch_prompt(kind, prompt, size, seed, client_id)?;
    Ok(json!({
        "prompt": api,
        "client_id": client_id
    }))
}

pub fn validate_submit_payload(payload: &Value) -> Result<(ModelKind, String, Option<&str>), String> {
    reject_refs(payload)?;
    let kind = parse_model_kind(payload.get("model").and_then(|v| v.as_str()))?;
    let prompt = payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if prompt.is_empty() {
        return Err("prompt is required".into());
    }
    let size = payload.get("size").and_then(|v| v.as_str());
    // Touch size parse early for clearer errors.
    let _ = parse_size_wh(size)?;
    Ok((kind, prompt.into(), size))
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

fn mime_for_name(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        "image/png"
    }
}

fn collect_image_outputs(outputs: &Value, base: &str) -> Vec<CanonicalOutput> {
    let mut found = Vec::new();
    let Some(map) = outputs.as_object() else {
        return found;
    };
    for (_nid, node_out) in map {
        let Some(obj) = node_out.as_object() else {
            continue;
        };
        let Some(arr) = obj.get("images").and_then(|v| v.as_array()) else {
            continue;
        };
        for item in arr {
            let Some(name) = item.get("filename").and_then(|v| v.as_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".mp4")
                || lower.ends_with(".webm")
                || lower.ends_with(".mkv")
                || lower.ends_with(".mov")
            {
                continue;
            }
            let sub = item
                .get("subfolder")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("output");
            found.push(CanonicalOutput {
                media_kind: "image".into(),
                source: "url".into(),
                value: view_url(base, name, sub, ty),
                mime: Some(mime_for_name(name).into()),
            });
        }
    }
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
    let media = collect_image_outputs(&outputs, base);
    if completed || !media.is_empty() {
        if media.is_empty() {
            return Err("Comfy completed without an image file in history outputs".into());
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
