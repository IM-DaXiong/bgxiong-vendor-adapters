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

use crate::config::{query_url, submit_url};
use crate::runninghub::{parse_submit_task_id, query_body, MappedStatus};
use bgxiong::vendor_adapter::types::{AdapterError, Operation};

struct Adapter;

impl Guest for Adapter {
    fn invoke(request: Invocation) -> Response {
        match request.operation {
            Operation::Capabilities => capabilities_op(),
            Operation::Probe => ok(Operation::Probe, serde_json::json!({"ok":true})),
            Operation::Submit => submit(&request),
            Operation::Query => query(&request),
            other => err(other, "adapterCapabilityDenied", "specimen does not implement this op"),
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
        Err(e) => {
            let msg = format!("caps json: {e}");
            err(Operation::Capabilities, "adapterBadOutput", &msg)
        }
    }
}

fn submit(request: &Invocation) -> Response {
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(e) => return err(Operation::Submit, "adapterBadOutput", &e.to_string()),
    };
    let prompt = payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let duration = json_opt_f64(payload.get("durationSeconds"));
    let fps = json_opt_f64(payload.get("fps"));
    let resolution = payload.get("resolution").and_then(|v| v.as_str());
    let body = match crate::runninghub::submit_body_mapped(prompt, duration, fps, resolution) {
        Ok((v, applied)) => (v.to_string(), applied),
        Err(e) => return err(Operation::Submit, "adapterBadOutput", &e),
    };
    match http_json(Operation::Submit, "POST", &submit_url(), &body.0) {
        Ok(v) => match parse_submit_task_id(&v) {
            Ok(id) => {
                let sdk = bgx_vendor_adapter_sdk::submit_accepted_with_applied(id, Some(body.1));
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
    let body = query_body(id).to_string();
    match http_json(Operation::Query, "POST", &query_url(), &body) {
        Ok(v) => match crate::runninghub::parse_query_for_slot(&v, &request.capability_slot) {
            Ok(q) => {
                let status = match q.status {
                    MappedStatus::Waiting => "running",
                    MappedStatus::Succeeded => "succeeded",
                    MappedStatus::Failed => "failed",
                };
                let outputs = q
                    .outputs
                    .into_iter()
                    .map(|o| bgx_vendor_adapter_sdk::Output::Media {
                        media_kind: o.media_kind,
                        source: o.source,
                        value: o.value,
                        mime: o.mime,
                    })
                    .collect();
                let sdk = bgx_vendor_adapter_sdk::query_result(&bgx_vendor_adapter_sdk::QueryResult {
                    status: status.into(),
                    outputs,
                    progress_text: if q.terminal_failure.is_some() {
                        None
                    } else {
                        q.vendor_message
                    },
                    retry_after_ms: None,
                    terminal_failure: q.terminal_failure,
                });
                sdk_resp(Operation::Query, sdk)
            }
            Err(e) => err(Operation::Query, "adapterBadOutput", &e),
        },
        Err(e) => e,
    }
}

fn json_opt_f64(v: Option<&serde_json::Value>) -> Option<f64> {
    let v = v?;
    v.as_f64()
        .or_else(|| v.as_u64().map(|n| n as f64))
        .or_else(|| v.as_i64().map(|n| n as f64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn bearer_headers() -> Vec<bgxiong::vendor_adapter::host_http::GeneratedHeader> {
    vec![bgxiong::vendor_adapter::host_http::GeneratedHeader {
        name: "authorization".into(),
        kind: "bearer".into(),
        target_field: "authorization".into(),
    }]
}

fn http_json(op: Operation, method: &str, url: &str, body: &str) -> Result<serde_json::Value, Response> {
    let plan = bgxiong::vendor_adapter::host_http::RequestPlan {
        method: method.into(),
        url: url.into(),
        headers: vec![bgxiong::vendor_adapter::host_http::Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        generated_headers: bearer_headers(),
        body: bgxiong::vendor_adapter::host_http::Body::Bytes(body.as_bytes().to_vec()),
        sink: bgxiong::vendor_adapter::host_http::ResponseSink::Buffer(1_048_576),
        timeout_ms: 0,
    };
    match bgxiong::vendor_adapter::host_http::execute(&plan, None) {
        Ok(resp) => {
            let bytes = match resp.payload {
                bgxiong::vendor_adapter::host_http::ResponsePayload::Buffered(b) => b,
                bgxiong::vendor_adapter::host_http::ResponsePayload::Media(_) => {
                    return Err(err(
                        op,
                        "adapterBadOutput",
                        "expected buffered HTTP body",
                    ));
                }
            };
            if resp.status >= 400 {
                return Err(err(
                    op,
                    "adapterVendorHttpError",
                    &format!("status={}", resp.status),
                ));
            }
            serde_json::from_slice(&bytes).map_err(|e| {
                err(op, "adapterBadOutput", &format!("vendor json: {e}"))
            })
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
