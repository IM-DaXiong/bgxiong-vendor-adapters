//! Wasm guest: host-http only. Compiled on wasm32.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

wit_bindgen::generate!({
    path: "wit/vendor-adapter.wit",
    world: "vendor-adapter",
});

use crate::config::{
    join_url, AUTH_USES_BEARER, CHAT_COMPLETIONS_PATH, EXAMPLE_TIMEOUT_MS, MODELS_PATH,
};
use crate::openai::{
    chat_body, configured_endpoint_base_url, model_is_allowed, offline_probe_doc, parse_chat_content,
    parse_model_ids, require_endpoint_origin, requested_model, thinking_rejected,
};
use bgxiong::vendor_adapter::types::{AdapterError, Operation};

struct Adapter;

impl Guest for Adapter {
    fn invoke(request: Invocation) -> Response {
        match request.operation {
            Operation::Capabilities => capabilities_op(),
            Operation::Probe => probe(&request),
            Operation::Submit => submit(&request),
            Operation::Query => err(
                Operation::Query,
                "adapterCapabilityDenied",
                "text slot is synchronous; query is not implemented",
            ),
            other => err(other, "adapterCapabilityDenied", "example does not implement this op"),
        }
    }
}

fn capabilities_op() -> Response {
    let doc = match crate::caps::runtime_caps_doc() {
        Ok(v) => v,
        Err(e) => return err(Operation::Capabilities, "adapterBadOutput", &e),
    };
    match serde_json::to_string(&doc) {
        Ok(s) => ok_json_string(Operation::Capabilities, s),
        Err(e) => err(
            Operation::Capabilities,
            "adapterBadOutput",
            &format!("caps json: {e}"),
        ),
    }
}

fn probe(request: &Invocation) -> Response {
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(_) => serde_json::json!({}),
    };
    if configured_endpoint_base_url(&payload).is_none() {
        return ok(Operation::Probe, offline_probe_doc());
    }
    let origin = match require_endpoint_origin(&payload) {
        Ok(s) => s,
        Err(e) => return err(Operation::Probe, "adapterInvalidRequest", &e),
    };
    let url = join_url(&origin, MODELS_PATH);
    match http_json(Operation::Probe, "GET", &url, None) {
        Ok(v) => match parse_model_ids(&v) {
            Ok(_) => ok(Operation::Probe, serde_json::json!({"ok": true, "live": true})),
            Err(e) => err(Operation::Probe, "adapterBadOutput", &e),
        },
        Err(e) => e,
    }
}

fn submit(request: &Invocation) -> Response {
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(e) => return err(Operation::Submit, "adapterBadOutput", &e.to_string()),
    };
    if let Some(msg) = thinking_rejected(&payload) {
        return err(Operation::Submit, "adapterInvalidRequest", &msg);
    }
    let model = match requested_model(&payload) {
        Ok(m) => m,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let origin = match require_endpoint_origin(&payload) {
        Ok(s) => s,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let listed = match http_json(
        Operation::Submit,
        "GET",
        &join_url(&origin, MODELS_PATH),
        None,
    ) {
        Ok(v) => match parse_model_ids(&v) {
            Ok(ids) => ids,
            Err(e) => return err(Operation::Submit, "adapterBadOutput", &e),
        },
        Err(e) => return e,
    };
    if !model_is_allowed(&model, &listed) {
        return err(
            Operation::Submit,
            "adapterInvalidRequest",
            &format!("model {model} is not declared and not in /v1/models"),
        );
    }
    let messages = payload.get("messages").cloned().unwrap_or(serde_json::Value::Null);
    let body = match chat_body(
        &model,
        &messages,
        payload.get("temperature").and_then(|v| v.as_f64()),
        payload.get("maxTokens").and_then(|v| v.as_u64()),
        payload
            .get("responseFormatJson")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    ) {
        Ok(v) => v,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let raw = match serde_json::to_string(&body) {
        Ok(s) => s,
        Err(e) => return err(Operation::Submit, "adapterBadOutput", &e.to_string()),
    };
    let url = join_url(&origin, CHAT_COMPLETIONS_PATH);
    match http_json(Operation::Submit, "POST", &url, Some(&raw)) {
        Ok(v) => match parse_chat_content(&v) {
            Ok(text) => {
                let sdk = bgx_vendor_adapter_sdk::submit_completed(vec![
                    bgx_vendor_adapter_sdk::Output::text(text),
                ]);
                sdk_resp(Operation::Submit, sdk)
            }
            Err(e) => err(Operation::Submit, "adapterBadOutput", &e),
        },
        Err(e) => e,
    }
}

fn generated_headers() -> Vec<bgxiong::vendor_adapter::host_http::GeneratedHeader> {
    if AUTH_USES_BEARER {
        vec![bgxiong::vendor_adapter::host_http::GeneratedHeader {
            name: "authorization".into(),
            kind: "bearer".into(),
            target_field: "authorization".into(),
        }]
    } else {
        vec![]
    }
}

fn http_json(
    op: Operation,
    method: &str,
    url: &str,
    body: Option<&str>,
) -> Result<serde_json::Value, Response> {
    let mut headers = vec![];
    let plan_body = if let Some(b) = body {
        headers.push(bgxiong::vendor_adapter::host_http::Header {
            name: "content-type".into(),
            value: "application/json".into(),
        });
        bgxiong::vendor_adapter::host_http::Body::Bytes(b.as_bytes().to_vec())
    } else {
        bgxiong::vendor_adapter::host_http::Body::None
    };
    let plan = bgxiong::vendor_adapter::host_http::RequestPlan {
        method: method.into(),
        url: url.into(),
        headers,
        generated_headers: generated_headers(),
        body: plan_body,
        sink: bgxiong::vendor_adapter::host_http::ResponseSink::Buffer(4_194_304),
        timeout_ms: EXAMPLE_TIMEOUT_MS,
    };
    match bgxiong::vendor_adapter::host_http::execute(&plan, None) {
        Ok(resp) => {
            if resp.status >= 400 {
                return Err(err(
                    op,
                    "adapterVendorHttpError",
                    &format!("HTTP {} url={}", resp.status, url),
                ));
            }
            let bytes = match resp.payload {
                bgxiong::vendor_adapter::host_http::ResponsePayload::Buffered(b) => b,
                bgxiong::vendor_adapter::host_http::ResponsePayload::Media(_) => {
                    return Err(err(op, "adapterBadOutput", "expected buffered HTTP body"));
                }
            };
            if bytes.is_empty() {
                return Ok(serde_json::json!({}));
            }
            serde_json::from_slice(&bytes)
                .map_err(|e| err(op, "adapterBadOutput", &format!("vendor json: {e}")))
        }
        Err(e) => Err(err(op, &e.code, &e.message)),
    }
}

fn ok(op: Operation, data: serde_json::Value) -> Response {
    Response {
        protocol_version: 1,
        operation: op,
        data_json: Some(data.to_string()),
        error: None,
    }
}

fn ok_json_string(op: Operation, data: String) -> Response {
    Response {
        protocol_version: 1,
        operation: op,
        data_json: Some(data),
        error: None,
    }
}

fn err(op: Operation, code: &str, message: &str) -> Response {
    Response {
        protocol_version: 1,
        operation: op,
        data_json: None,
        error: Some(AdapterError {
            code: code.into(),
            message: message.into(),
            retryable: false,
            vendor_code: None,
            vendor_request_id: None,
        }),
    }
}

fn sdk_resp(op: Operation, sdk: bgx_vendor_adapter_sdk::Response) -> Response {
    Response {
        protocol_version: sdk.protocol_version,
        operation: op,
        data_json: sdk.data_json,
        error: sdk.error.map(|e| AdapterError {
            code: e.code,
            message: e.message,
            retryable: e.retryable,
            vendor_code: e.vendor_code,
            vendor_request_id: e.vendor_request_id,
        }),
    }
}

export!(Adapter);
