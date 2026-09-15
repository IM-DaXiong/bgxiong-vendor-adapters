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

use crate::comfy::{
    configured_endpoint_base_url, offline_probe_doc, parse_history, parse_submit_prompt_id,
    resolve_base_url, submit_body, validate_submit_payload, MappedStatus,
};
use crate::config::join_url;
use bgxiong::vendor_adapter::types::{AdapterError, Operation};

struct Adapter;

impl Guest for Adapter {
    fn invoke(request: Invocation) -> Response {
        match request.operation {
            Operation::Capabilities => capabilities_op(),
            Operation::Probe => probe(&request),
            Operation::Submit => submit(&request),
            Operation::Query => query(&request),
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
    let base = resolve_base_url(&payload);
    let url = join_url(&base, "/system_stats");
    match http_json(Operation::Probe, "GET", &url, None) {
        Ok(_) => ok(Operation::Probe, serde_json::json!({"ok": true, "live": true})),
        Err(e) => e,
    }
}

fn submit(request: &Invocation) -> Response {
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(e) => return err(Operation::Submit, "adapterBadOutput", &e.to_string()),
    };
    let (kind, prompt, size) = match validate_submit_payload(&payload) {
        Ok(v) => v,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let base = resolve_base_url(&payload);
    let body = match submit_body(kind, &prompt, size, None, &request.request_id) {
        Ok(v) => v,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let raw = match serde_json::to_string(&body) {
        Ok(s) => s,
        Err(e) => return err(Operation::Submit, "adapterBadOutput", &e.to_string()),
    };
    let url = join_url(&base, "/prompt");
    match http_json(Operation::Submit, "POST", &url, Some(&raw)) {
        Ok(v) => match parse_submit_prompt_id(&v) {
            Ok(id) => {
                let sdk = bgx_vendor_adapter_sdk::submit_accepted(id);
                sdk_resp(Operation::Submit, sdk)
            }
            Err(e) => err(Operation::Submit, "adapterBadOutput", &e),
        },
        Err(e) => e,
    }
}

fn query(request: &Invocation) -> Response {
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(e) => return err(Operation::Query, "adapterBadOutput", &e.to_string()),
    };
    let id = match payload.get("vendorTaskId").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return err(Operation::Query, "adapterBadOutput", "query requires vendorTaskId"),
    };
    let base = resolve_base_url(&payload);
    let url = join_url(&base, &format!("/history/{id}"));
    match http_json(Operation::Query, "GET", &url, None) {
        Ok(v) => match parse_history(&v, id, &base) {
            Ok(q) => {
                let status = match q.status {
                    MappedStatus::Queued => "queued",
                    MappedStatus::Running => "running",
                    MappedStatus::Succeeded => "succeeded",
                    MappedStatus::Failed => "failed",
                };
                let outputs = q
                    .outputs
                    .into_iter()
                    .map(|o| bgx_vendor_adapter_sdk::Output {
                        media_kind: o.media_kind,
                        source: o.source,
                        value: o.value,
                        mime: o.mime,
                    })
                    .collect();
                let sdk = bgx_vendor_adapter_sdk::query_result(&bgx_vendor_adapter_sdk::QueryResult {
                    status: status.into(),
                    outputs,
                    progress_text: q.vendor_message,
                });
                sdk_resp(Operation::Query, sdk)
            }
            Err(e) => err(Operation::Query, "adapterBadOutput", &e),
        },
        Err(e) => e,
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
        generated_headers: vec![],
        body: plan_body,
        sink: bgxiong::vendor_adapter::host_http::ResponseSink::Buffer(1_048_576),
        timeout_ms: 30_000,
    };
    match bgxiong::vendor_adapter::host_http::execute(&plan, None) {
        Ok(resp) => {
            if resp.status >= 400 {
                return Err(err(
                    op,
                    "adapterVendorHttpError",
                    &format!("HTTP {}", resp.status),
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
