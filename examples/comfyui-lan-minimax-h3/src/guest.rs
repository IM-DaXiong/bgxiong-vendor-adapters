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
    collect_lan_images, configured_endpoint_base_url, offline_probe_doc, parse_comfy_upload_name,
    parse_history, parse_model_kind, parse_submit_prompt_id, resolve_base_url, submit_body_for_kind,
    ModelKind, MappedStatus, StagedImage,
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
    let kind = match parse_model_kind(payload.get("model").and_then(|v| v.as_str())) {
        Ok(k) => k,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let prompt = payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let duration = payload.get("durationSeconds").and_then(|v| v.as_f64());
    let fps = payload.get("fps").and_then(|v| v.as_f64());
    let resolution = payload.get("resolution").and_then(|v| v.as_str());
    let aspect = payload
        .get("aspect")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("aspectRatio").and_then(|v| v.as_str()));
    let base = resolve_base_url(&payload);
    let images = match collect_lan_images(&payload) {
        Ok(v) => v,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let first_name = match kind {
        ModelKind::I2vTurbo => {
            let Some(first) = images.first() else {
                return err(
                    Operation::Submit,
                    "adapterInvalidRequest",
                    "h3.i2v.turbo requires a first-frame image",
                );
            };
            match upload_staged(&base, first, "first") {
                Ok(Some(n)) => Some(n),
                Ok(None) => {
                    return err(
                        Operation::Submit,
                        "adapterInvalidRequest",
                        "h3.i2v.turbo requires a first-frame image",
                    )
                }
                Err(e) => return e,
            }
        }
        _ => None,
    };
    let ref_names = match kind {
        ModelKind::R2vTurbo => {
            if images.is_empty() {
                return err(
                    Operation::Submit,
                    "adapterInvalidRequest",
                    "h3.r2v.turbo requires at least one reference image",
                );
            }
            let mut names = Vec::new();
            for (i, staged) in images.iter().enumerate() {
                match upload_staged(&base, staged, &format!("ref{i}")) {
                    Ok(Some(n)) => names.push(n),
                    Ok(None) => {}
                    Err(e) => return e,
                }
            }
            if names.is_empty() {
                return err(
                    Operation::Submit,
                    "adapterInvalidRequest",
                    "h3.r2v.turbo requires at least one reference image",
                );
            }
            names
        }
        _ => Vec::new(),
    };
    let body = match submit_body_for_kind(
        kind,
        prompt,
        duration,
        fps,
        resolution,
        aspect,
        first_name.as_deref(),
        &ref_names,
        &request.request_id,
    ) {
        Ok(v) => v,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let raw = match serde_json::to_string(&body.0) {
        Ok(s) => s,
        Err(e) => return err(Operation::Submit, "adapterBadOutput", &e.to_string()),
    };
    let url = join_url(&base, "/prompt");
    match http_json(Operation::Submit, "POST", &url, Some(&raw)) {
        Ok(v) => match parse_submit_prompt_id(&v) {
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

fn upload_staged(
    base: &str,
    staged: &StagedImage,
    stem: &str,
) -> Result<Option<String>, Response> {
    let handle = staged.handle.trim();
    if handle.is_empty() {
        return Ok(None);
    }
    let filename = if staged.file_name.trim().is_empty() {
        format!("{stem}.png")
    } else {
        staged.file_name.clone()
    };
    let mime = if staged.mime.trim().is_empty() {
        "image/png".into()
    } else {
        staged.mime.clone()
    };
    let url = join_url(base, "/upload/image");
    let plan = bgxiong::vendor_adapter::host_http::RequestPlan {
        method: "POST".into(),
        url: url.clone(),
        headers: vec![],
        generated_headers: vec![],
        body: bgxiong::vendor_adapter::host_http::Body::Multipart(vec![
            bgxiong::vendor_adapter::host_http::MultipartPart::File(
                bgxiong::vendor_adapter::host_http::FilePart {
                    name: "image".into(),
                    filename,
                    handle: handle.to_string(),
                    mime,
                },
            ),
        ]),
        sink: bgxiong::vendor_adapter::host_http::ResponseSink::Buffer(1_048_576),
        timeout_ms: 0,
    };
    match bgxiong::vendor_adapter::host_http::execute(&plan, None) {
        Ok(resp) => {
            if resp.status >= 400 {
                return Err(err(
                    Operation::Submit,
                    "adapterVendorHttpError",
                    &format!("upload HTTP {} url={}", resp.status, url),
                ));
            }
            let bytes = match resp.payload {
                bgxiong::vendor_adapter::host_http::ResponsePayload::Buffered(b) => b,
                _ => {
                    return Err(err(
                        Operation::Submit,
                        "adapterBadOutput",
                        "upload expected buffered body",
                    ))
                }
            };
            let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                err(Operation::Submit, "adapterBadOutput", &format!("upload json: {e}"))
            })?;
            parse_comfy_upload_name(&v)
                .map(Some)
                .map_err(|e| err(Operation::Submit, "adapterBadOutput", &e))
        }
        Err(e) => Err(err(Operation::Submit, &e.code, &e.message)),
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
    let url_owned = url.to_string();
    let plan = bgxiong::vendor_adapter::host_http::RequestPlan {
        method: method.into(),
        url: url_owned.clone(),
        headers,
        generated_headers: vec![],
        body: plan_body,
        sink: bgxiong::vendor_adapter::host_http::ResponseSink::Buffer(1_048_576),
        timeout_ms: 0,
    };
    match bgxiong::vendor_adapter::host_http::execute(&plan, None) {
        Ok(resp) => {
            if resp.status >= 400 {
                return Err(err(
                    op,
                    "adapterVendorHttpError",
                    &format!("HTTP {} url={}", resp.status, url_owned),
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
