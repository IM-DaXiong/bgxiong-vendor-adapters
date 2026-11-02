//! Isolated LAN OpenAI-compatible text guest.

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
pub mod openai;

#[cfg(any(target_arch = "wasm32", test))]
mod wasm_mem;

#[cfg(target_arch = "wasm32")]
#[path = "guest.rs"]
mod guest;

#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_json(operation: &str, payload_json: &str) -> Result<serde_json::Value, String> {
    match operation {
        "capabilities" => crate::caps::runtime_caps_doc(),
        "probe" => probe_op(payload_json),
        "submit" => submit_op(payload_json),
        "query" | "cancel" | "upload" => Err(format!("{operation} is out of example scope")),
        other => Err(format!("unknown operation {other}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn probe_op(payload_json: &str) -> Result<serde_json::Value, String> {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).unwrap_or_else(|_| serde_json::json!({}));
    if crate::openai::configured_endpoint_base_url(&payload).is_some() {
        let _ = crate::openai::require_endpoint_origin(&payload)?;
        return Err("live HTTP is only available inside the wasm guest via host-http".into());
    }
    Ok(crate::openai::offline_probe_doc())
}

#[cfg(not(target_arch = "wasm32"))]
fn submit_op(payload_json: &str) -> Result<serde_json::Value, String> {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|e| format!("payload: {e}"))?;
    if let Some(msg) = crate::openai::thinking_rejected(&payload) {
        return Err(msg);
    }
    let model = crate::openai::requested_model(&payload)?;
    if !crate::openai::model_is_allowed(&model, &[]) {
        return Err(format!(
            "model {model} is not EXAMPLE_MODEL_ID and no probe list is available"
        ));
    }
    let origin = crate::openai::require_endpoint_origin(&payload)?;
    let _ = crate::openai::chat_body(
        &model,
        payload.get("messages").unwrap_or(&serde_json::Value::Null),
        payload.get("temperature").and_then(|v| v.as_f64()),
        payload.get("maxTokens").and_then(|v| v.as_u64()),
        payload
            .get("responseFormatJson")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    )?;
    let _ = origin;
    Err("live HTTP is only available inside the wasm guest via host-http".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        join_url, normalize_origin, reject_unspecified_host, EXAMPLE_BASE_URL, EXAMPLE_MODEL_ID,
        PLUGIN_ID,
    };
    use crate::openai::{
        configured_endpoint_base_url, parse_chat_content, parse_model_ids,
    };

    #[test]
    fn probe_is_offline_ok() {
        let v = dispatch_json("probe", "{}").expect("probe");
        assert_eq!(v["ok"], true);
        assert_eq!(v["live"], false);
    }

    #[test]
    fn probe_with_endpoint_does_not_fake_live_http() {
        let err = dispatch_json(
            "probe",
            r#"{"endpointBaseUrl":"http://192.0.2.8:11434"}"#,
        )
        .unwrap_err();
        assert!(err.contains("host-http"));
    }

    #[test]
    fn submit_without_base_url_does_not_hit_specimen() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"example-lan-llm","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .unwrap_err();
        assert!(err.contains("去设置填内网地址"));
        assert!(!err.contains("192.168.0.10"));
    }

    #[test]
    fn thinking_true_is_hard_error() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"example-lan-llm","thinking":true,"endpointBaseUrl":"http://192.0.2.8:11434","messages":[]}"#,
        )
        .unwrap_err();
        assert!(err.contains("thinking"));
    }

    #[test]
    fn unknown_model_hard_fails() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"not-the-declared-id","endpointBaseUrl":"http://192.0.2.8:11434","messages":[]}"#,
        )
        .unwrap_err();
        assert!(err.contains("model"));
    }

    #[test]
    fn unspecified_origin_rejected() {
        assert!(reject_unspecified_host("http://0.0.0.0:11434").is_err());
        assert!(reject_unspecified_host("http://[::]:11434").is_err());
        assert!(normalize_origin("http://192.0.2.8:11434/v1/").unwrap().ends_with(":11434"));
        assert_eq!(
            join_url("http://192.0.2.8:11434/", "/v1/models"),
            "http://192.0.2.8:11434/v1/models"
        );
        assert_eq!(EXAMPLE_BASE_URL, "http://192.168.0.10:11434");
    }

    #[test]
    fn parse_chat_fixture() {
        let body = serde_json::json!({
            "choices": [{"message": {"content": "hello from lan"}}]
        });
        assert_eq!(parse_chat_content(&body).unwrap(), "hello from lan");
        let ids = parse_model_ids(&serde_json::json!({
            "data": [{"id": "example-lan-llm"}, {"id": "other"}]
        }))
        .unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "example-lan-llm");
        assert_eq!(ids[1], "other");
    }

    #[test]
    fn capabilities_returns_text_slot() {
        let v = dispatch_json("capabilities", "{}").expect("caps");
        assert_eq!(v["schemaVersion"], 1);
        assert_eq!(v["pluginId"], PLUGIN_ID);
        assert_eq!(v["slots"][0]["slot"], "text");
        assert_eq!(v["slots"][0]["modelId"], EXAMPLE_MODEL_ID);
    }

    #[test]
    fn empty_payload_has_no_endpoint() {
        assert_eq!(configured_endpoint_base_url(&serde_json::json!({})), None);
        assert_eq!(
            configured_endpoint_base_url(&serde_json::json!({"endpointBaseUrl":"  "})),
            None
        );
    }

    #[test]
    fn guest_probe_gates_http() {
        let guest = include_str!("guest.rs");
        assert!(guest.contains("configured_endpoint_base_url"));
        assert!(guest.contains("generated_headers"));
        assert!(!guest.contains("Bearer sk-"));
    }
}
