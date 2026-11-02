//! Wasm guest for the offline video reference-media probe.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

wit_bindgen::generate!({
    path: "wit/vendor-adapter.wit",
    world: "vendor-adapter",
});

use bgx_vendor_adapter_sdk::{
    decode_video_media_submit_v1, query_result, submit_accepted, Output, QueryResult,
};
use bgxiong::vendor_adapter::types::{AdapterError, Operation};

struct Probe;

impl Guest for Probe {
    fn invoke(request: Invocation) -> Response {
        match request.operation {
            Operation::Capabilities => capabilities_op(),
            Operation::Probe => ok_json(Operation::Probe, serde_json::json!({"ok": true})),
            Operation::Submit => submit(&request),
            Operation::Query => query(&request),
            Operation::Cancel => ok_json(
                Operation::Cancel,
                serde_json::json!({"status": "cancelled"}),
            ),
            Operation::Upload => err(
                request.operation,
                "adapterCapabilityDenied",
                "probe does not implement upload; use MediaInputV1 handles",
            ),
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

fn submit(request: &Invocation) -> Response {
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(e) => {
            return err(
                Operation::Submit,
                "adapterInvalidRequest",
                &format!("payload: {e}"),
            )
        }
    };

    let media = match decode_video_media_submit_v1(&request.payload_json) {
        Ok(m) => m,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };

    if media.model.contains("reject") {
        let n = media
            .reference_videos
            .as_ref()
            .map(|a| a.len())
            .unwrap_or(0);
        if n > 2 {
            return err(
                Operation::Submit,
                "adapterInvalidRequest",
                "probe reject mode: at most 2 reference videos",
            );
        }
    }

    let mut handles: Vec<(String, String)> = Vec::new();
    if let Some(ref sf) = media.start_frame {
        handles.push((sf.id.clone(), sf.handle.clone()));
    }
    if let Some(ref ef) = media.end_frame {
        handles.push((ef.id.clone(), ef.handle.clone()));
    }
    if let Some(ref items) = media.reference_images {
        for item in items {
            handles.push((item.id.clone(), item.handle.clone()));
        }
    }
    if let Some(ref items) = media.reference_videos {
        for item in items {
            handles.push((item.id.clone(), item.handle.clone()));
        }
    }
    if let Some(ref items) = media.reference_audios {
        for item in items {
            handles.push((item.id.clone(), item.handle.clone()));
        }
    }

    for (id, handle) in &handles {
        if handle.trim().is_empty() {
            return err(
                Operation::Submit,
                "adapterInvalidRequest",
                &format!("empty handle for media id {id}"),
            );
        }
        match bgxiong::vendor_adapter::host_media::media_info(handle) {
            Ok(stat) => {
                let mut off = 0u64;
                loop {
                    match bgxiong::vendor_adapter::host_media::media_read(handle, off, 65536) {
                        Ok(chunk) => {
                            if chunk.is_empty() {
                                break;
                            }
                            off = off.saturating_add(chunk.len() as u64);
                        }
                        Err(e) => {
                            return err(
                                Operation::Submit,
                                "adapterInvalidRequest",
                                &format!("host-media read failed for {id}: {e}"),
                            );
                        }
                    }
                }
                if off != stat.byte_length {
                    return err(
                        Operation::Submit,
                        "adapterInvalidRequest",
                        &format!("read length {off} != info {} for {id}", stat.byte_length),
                    );
                }
            }
            Err(e) => {
                return err(
                    Operation::Submit,
                    "adapterInvalidRequest",
                    &format!("host-media info failed for {id}: {e}"),
                );
            }
        }
    }

    for k in [
        "firstFrameB64",
        "startFrameB64",
        "endFrameB64",
        "referenceImagesB64",
        "referenceVideosB64",
        "referenceAudiosB64",
        "first_frame_b64",
        "last_frame_b64",
        "reference_images_b64",
    ] {
        if payload.get(k).is_some() {
            return err(
                Operation::Submit,
                "adapterInvalidRequest",
                &format!("legacy B64 field {k} forbidden when using video-reference-media-v1"),
            );
        }
    }

    if media.model == "probe-offline-http" {
        let endpoint = match probe_endpoint(&payload) {
            Ok(v) => v,
            Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
        };
        let Some((_, handle)) = handles.first() else {
            return err(
                Operation::Submit,
                "adapterInvalidRequest",
                "HTTP probe requires at least one reference media handle",
            );
        };
        if let Err(e) = upload_probe_media(&endpoint, handle) {
            return err(Operation::Submit, "adapterVendorHttpError", &e);
        }
    }

    let out_handle = match write_probe_output() {
        Ok(h) => h,
        Err(e) => return err(Operation::Submit, "adapterInvalidRequest", &e),
    };
    let rid = request.request_id.trim();
    let task_id = if !rid.is_empty() {
        format!("probe-{rid}")
    } else {
        format!(
            "probe-{}",
            media
                .model
                .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
        )
    };
    if let Err(e) =
        bgxiong::vendor_adapter::host_state::state_put(&format!("out:{task_id}"), &out_handle)
    {
        return err(Operation::Submit, "adapterInvalidRequest", &e);
    }
    let sdk = submit_accepted(task_id);
    sdk_to_wit(Operation::Submit, sdk)
}

fn write_probe_output() -> Result<String, String> {
    let h = bgxiong::vendor_adapter::host_media::media_create("video", "video/mp4")?;
    bgxiong::vendor_adapter::host_media::media_write(&h, b"probe-offline-video")?;
    bgxiong::vendor_adapter::host_media::media_finish(&h)?;
    Ok(h)
}

fn query(request: &Invocation) -> Response {
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(e) => {
            return err(
                Operation::Query,
                "adapterInvalidRequest",
                &format!("payload: {e}"),
            )
        }
    };
    let vendor_task_id = payload
        .get("vendorTaskId")
        .and_then(|v| v.as_str())
        .unwrap_or("probe-offline");
    if payload.get("model").and_then(|v| v.as_str()) == Some("probe-offline-http") {
        let endpoint = match probe_endpoint(&payload) {
            Ok(v) => v,
            Err(e) => return err(Operation::Query, "adapterInvalidRequest", &e),
        };
        let handle = match download_probe_media(&endpoint) {
            Ok(v) => v,
            Err(e) => return err(Operation::Query, "adapterVendorHttpError", &e),
        };
        return sdk_to_wit(
            Operation::Query,
            query_result(&QueryResult {
                status: "succeeded".into(),
                outputs: vec![Output::media("video", "host-media", handle)],
                progress_text: Some("offline-http".into()),
                retry_after_ms: None,
                terminal_failure: None,
            }),
        );
    }
    let out_handle =
        bgxiong::vendor_adapter::host_state::state_get(&format!("out:{vendor_task_id}"))
            .ok()
            .flatten()
            .filter(|handle| !handle.is_empty())
            .map(Ok)
            .unwrap_or_else(write_probe_output);
    let outputs = match out_handle {
        Ok(handle) => vec![Output::media("video", "host-media", handle)],
        Err(e) => return err(Operation::Query, "adapterInvalidRequest", &e),
    };
    let sdk = query_result(&QueryResult {
        status: "succeeded".into(),
        outputs,
        progress_text: Some("offline".into()),
        retry_after_ms: None,
        terminal_failure: None,
    });
    sdk_to_wit(Operation::Query, sdk)
}

fn probe_endpoint(payload: &serde_json::Value) -> Result<String, String> {
    payload
        .get("pluginParameters")
        .and_then(|v| v.get("endpoint"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_end_matches('/').to_string())
        .ok_or_else(|| "pluginParameters.endpoint is required for HTTP probe".to_string())
}

fn upload_probe_media(endpoint: &str, handle: &str) -> Result<(), String> {
    let plan = bgxiong::vendor_adapter::host_http::RequestPlan {
        method: "POST".into(),
        url: format!("{endpoint}/upload"),
        headers: vec![],
        generated_headers: vec![],
        body: bgxiong::vendor_adapter::host_http::Body::Multipart(vec![
            bgxiong::vendor_adapter::host_http::MultipartPart::Field(
                bgxiong::vendor_adapter::host_http::FieldPart {
                    name: "purpose".into(),
                    value: "reference-video".into(),
                },
            ),
            bgxiong::vendor_adapter::host_http::MultipartPart::File(
                bgxiong::vendor_adapter::host_http::FilePart {
                    name: "file".into(),
                    filename: "reference.webm".into(),
                    handle: handle.into(),
                    mime: "video/webm".into(),
                },
            ),
        ]),
        sink: bgxiong::vendor_adapter::host_http::ResponseSink::Buffer(64 * 1024),
        timeout_ms: 0,
    };
    let response = bgxiong::vendor_adapter::host_http::execute(&plan, None)
        .map_err(|e| format!("{}: {}", e.code, e.message))?;
    if !(200..300).contains(&response.status) {
        return Err(format!("upload HTTP {}", response.status));
    }
    Ok(())
}

fn download_probe_media(endpoint: &str) -> Result<String, String> {
    let plan = bgxiong::vendor_adapter::host_http::RequestPlan {
        method: "GET".into(),
        url: format!("{endpoint}/download"),
        headers: vec![],
        generated_headers: vec![],
        body: bgxiong::vendor_adapter::host_http::Body::None,
        sink: bgxiong::vendor_adapter::host_http::ResponseSink::Media,
        timeout_ms: 0,
    };
    let response = bgxiong::vendor_adapter::host_http::execute(&plan, None)
        .map_err(|e| format!("{}: {}", e.code, e.message))?;
    if !(200..300).contains(&response.status) {
        return Err(format!("download HTTP {}", response.status));
    }
    match response.payload {
        bgxiong::vendor_adapter::host_http::ResponsePayload::Media(handle) => Ok(handle),
        _ => Err("download expected media response".into()),
    }
}

fn ok_json(op: Operation, data: serde_json::Value) -> Response {
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

fn sdk_to_wit(op: Operation, sdk: bgx_vendor_adapter_sdk::Response) -> Response {
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

export!(Probe);
