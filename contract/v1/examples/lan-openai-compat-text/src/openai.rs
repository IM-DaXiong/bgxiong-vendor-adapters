//! OpenAI-shaped JSON mappers. No HTTP.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::config::{normalize_origin, EXAMPLE_MODEL_ID};

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
        .map(|s| s.to_string())
}

pub fn require_endpoint_origin(payload: &Value) -> Result<String, String> {
    let raw = payload
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
        });
    match raw {
        None => Err("去设置填内网地址".into()),
        Some(s) => normalize_origin(s),
    }
}

pub fn offline_probe_doc() -> Value {
    json!({"ok": true, "live": false})
}

pub fn parse_model_ids(body: &Value) -> Result<Vec<String>, String> {
    let data = body
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "models list missing data[]".to_string())?;
    let mut ids = Vec::new();
    for item in data {
        if let Some(id) = item.get("id").and_then(|v| v.as_str()).map(str::trim) {
            if !id.is_empty() {
                ids.push(id.to_string());
            }
        }
    }
    Ok(ids)
}

pub fn model_is_allowed(requested: &str, listed: &[String]) -> bool {
    let req = requested.trim();
    if req.is_empty() {
        return false;
    }
    if req == EXAMPLE_MODEL_ID {
        return true;
    }
    listed.iter().any(|id| id == req)
}

pub fn parse_chat_content(body: &Value) -> Result<String, String> {
    let content = body
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"));
    match content {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err("choices[0].message.content must be a string".into()),
        None => Err("missing choices[0].message.content".into()),
    }
}

pub fn chat_body(
    model: &str,
    messages: &Value,
    temperature: Option<f64>,
    max_tokens: Option<u64>,
    response_format_json: bool,
) -> Result<Value, String> {
    if !messages.is_array() {
        return Err("messages must be an array".into());
    }
    let mut obj = serde_json::Map::new();
    obj.insert("model".into(), json!(model));
    obj.insert("messages".into(), messages.clone());
    if let Some(t) = temperature {
        obj.insert("temperature".into(), json!(t));
    }
    if let Some(n) = max_tokens {
        obj.insert("max_tokens".into(), json!(n));
    }
    if response_format_json {
        obj.insert(
            "response_format".into(),
            json!({"type": "json_object"}),
        );
    }
    Ok(Value::Object(obj))
}

pub fn thinking_rejected(payload: &Value) -> Option<String> {
    match payload.get("thinking") {
        Some(Value::Bool(true)) => Some(
            "this example rejects thinking=true; map it in guest if the engine supports it".into(),
        ),
        _ => None,
    }
}

pub fn requested_model(payload: &Value) -> Result<String, String> {
    payload
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("submit requires model"))
}
