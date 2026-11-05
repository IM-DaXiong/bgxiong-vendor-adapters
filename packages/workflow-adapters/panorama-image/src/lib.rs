#![cfg_attr(target_arch = "wasm32", no_std)]
//! local.thirdparty.comfy-panorama-image: one perspective image -> one ERP panorama via local ComfyUI.
//! Live Comfy sampling is NOT_RUN on the authoring machine; protocol/mapping tests are local-only.
extern crate alloc;

mod capability;
mod comfy;
mod mapping;

#[cfg(test)]
mod tests;

use alloc::{format, string::String, string::ToString};
use bgx_adapter_v2_sdk::{guest, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};

use capability::{capabilities, OPERATION_ID};
use mapping::{build_graph, reference_id, require_operation, Failure};

const OUTPUT_NODE: &str = "66";

fn fail_response(request: &Invocation, failure: Failure) -> Response {
    let err = json!({
        "protocolVersion": 2,
        "operation": request.r#operation,
        "error": {
            "code": failure.code,
            "message": failure.message,
            "retryable": serde_json::to_value(&request.r#operation).ok().as_ref() == Some(&json!("query"))
                && matches!(failure.code, "TIMEOUT" | "RESOURCE_UNAVAILABLE" | "UPSTREAM_ERROR"),
            "vendorCode": null,
            "vendorRequestId": null
        }
    });
    serde_json::from_value(err).expect("closed public error")
}

fn fail_response_vendor(request: &Invocation, failure: Failure, vendor: Option<String>) -> Response {
    let response = fail_response(request, failure);
    if let Some(v) = vendor {
        // Re-serialize with vendorCode when present.
        let mut value = serde_json::to_value(&response).expect("response");
        if let Some(err) = value.get_mut("error") {
            err["vendorCode"] = json!(v);
        }
        serde_json::from_value(value).expect("closed public error")
    } else {
        response
    }
}

fn persist_accepted(
    host: &mut dyn Host,
    request: &Invocation,
    accepted: &Value,
) -> Result<(), Failure> {
    let unknown = || Failure {
        code: "ACCEPTANCE_UNKNOWN",
        message: "Remote accepted; checkpoint failed. Do not resubmit.".into(),
    };
    let read = comfy::port(
        host,
        "host-checkpoint.read",
        json!({"requestId": request.r#request_id}),
    )
    .map_err(|_| unknown())?;
    let mut checkpoint: Value =
        serde_json::from_str(comfy::text(&read, "valueJson").map_err(|_| unknown())?)
            .map_err(|_| unknown())?;
    let old = comfy::text(&read, "revision")
        .map_err(|_| unknown())?
        .to_string();
    let revision = old
        .parse::<u64>()
        .ok()
        .and_then(|r| r.checked_add(1))
        .ok_or_else(unknown)?;
    checkpoint["revision"] = json!(revision.to_string());
    checkpoint["state"] = json!("accepted");
    checkpoint["vendorTaskId"] = accepted["vendorTaskId"].clone();
    checkpoint["resumeToken"] = accepted["resumeToken"].clone();
    checkpoint["result"] = accepted.clone();
    let written = comfy::port(
        host,
        "host-checkpoint.write",
        json!({
            "requestId": request.r#request_id,
            "expectedRevision": old,
            "valueJson": checkpoint.to_string()
        }),
    )
    .map_err(|_| Failure {
        code: "ACCEPTANCE_UNKNOWN",
        message: "ComfyUI accepted the prompt but checkpoint storage failed; do not resubmit"
            .into(),
    })?;
    if let Some(got) = written.get("revision").and_then(Value::as_str) {
        if got != revision.to_string() {
            return Err(Failure {
                code: "ACCEPTANCE_UNKNOWN",
                message: "Checkpoint write returned unexpected revision; do not resubmit".into(),
            });
        }
    }
    Ok(())
}

fn find_media<'a>(payload: &'a Value, media_id: &str) -> Result<&'a Value, Failure> {
    payload
        .get("media")
        .and_then(Value::as_array)
        .and_then(|m| m.iter().find(|x| x["id"] == media_id && x["kind"] == "image"))
        .ok_or_else(|| {
            Failure::invalid("The referenced original image has no matching media lease")
        })
}

fn submit(host: &mut dyn Host, request: &Invocation, payload: &Value) -> Result<Value, Failure> {
    require_operation(payload)?;
    let input = &payload["input"];
    // Validate mapping before any upload.
    let _ = build_graph(input, "pending-upload.png")?;
    let media_id = reference_id(input)?;
    let media = find_media(payload, media_id)?;
    let ctx = comfy::Ctx {
        config: &payload["configuration"],
        budget_ms: request.r#budget_ms,
    };
    let uploaded = comfy::upload(host, &ctx, media)?;
    let (graph, applied) = build_graph(input, &uploaded)?;
    let remote = comfy::submit(host, &ctx, &graph, &request.r#request_id)?;
    let accepted = json!({
        "kind": "accepted",
        "vendorTaskId": remote,
        "resumeToken": {"promptId": remote},
        "appliedParams": applied,
        "usage": null
    });
    persist_accepted(host, request, &accepted)?;
    Ok(accepted)
}

fn query(host: &mut dyn Host, request: &Invocation, payload: &Value) -> Result<Value, Failure> {
    require_operation(payload)?;
    let ctx = comfy::Ctx {
        config: &payload["configuration"],
        budget_ms: request.r#budget_ms,
    };
    let id = comfy::text(payload, "vendorTaskId").map_err(|_| {
        Failure::invalid("vendorTaskId (original Comfy prompt id) is required")
    })?;
    if payload["resumeToken"]["promptId"] != id {
        return Err(Failure::invalid(
            "Resume token must name the original prompt id",
        ));
    }
    match comfy::status(host, &ctx, id, OUTPUT_NODE)? {
        comfy::Remote::Queued => Ok(json!({
            "status": "queued",
            "resumeToken": {"promptId": id},
            "outputs": [],
            "data": {"imageArtifacts": []},
            "terminalFailure": null,
            "progress": null,
            "usage": null
        })),
        comfy::Remote::Running => Ok(json!({
            "status": "running",
            "resumeToken": {"promptId": id},
            "outputs": [],
            "data": {"imageArtifacts": []},
            "terminalFailure": null,
            "progress": null,
            "usage": null
        })),
        comfy::Remote::Succeeded(files) => {
            let artifact = comfy::download(host, &ctx, &files[0])?;
            let media_id = artifact["media"]["id"].clone();
            Ok(json!({
                "status": "succeeded",
                    "resumeToken": {"promptId": id},
                "outputs": [artifact],
                "data": {
                    "imageArtifacts": [{
                        "mediaId": media_id,
                        "role": "result",
                        "projection": "equirectangular360"
                    }]
                },
                "terminalFailure": null,
                "progress": null,
            "usage": null
            }))
        }
        comfy::Remote::Failed(vendor) => Ok(json!({
            "status": "failed",
            "resumeToken": {"promptId": id},
            "outputs": [],
            "data": {"imageArtifacts": []},
            "terminalFailure": {
                "category": "vendorTaskFailed",
                "message": "Panorama ComfyUI execution failed",
                "vendorCode": vendor,
                "details": {"promptId": id}
            },
            "progress": null,
            "usage": null
        })),
        comfy::Remote::Lost => Err(Failure {
            code: "RESOURCE_UNAVAILABLE",
            message: "Original history is unavailable; preserve prompt ID and do not resubmit".into(),
        }),
    }
}

fn probe(host: &mut dyn Host, payload: &Value, budget_ms: u64) -> Result<Value, Failure> {
    match payload.get("kind").and_then(Value::as_str) {
        Some("package_validation") => Ok(json!({
            "packageValid": true,
            "connection": "not_checked",
            "authentication": "not_checked",
            "resources": "not_checked",
            "issues": []
        })),
        Some("connectivity") => {
            let ctx = comfy::Ctx {
                config: &payload["configuration"],
                budget_ms,
            };
            comfy::status_probe(host, &ctx)?;
            Ok(json!({
                "packageValid": true,
                "connection": "ready",
                "authentication": "not_checked",
                "resources": "not_checked",
                "issues": []
            }))
        }
        _ => Err(Failure::invalid("Unknown probe kind")),
    }
}

pub struct PanoramaImage;

impl Adapter for PanoramaImage {
    fn invoke(request: Invocation, host: &mut dyn Host) -> Response {
        let operation = serde_json::to_value(&request.r#operation).expect("closed operation");
        let result = match operation.as_str().expect("operation string") {
            "capabilities" => capabilities(),
            "probe" => {
                let payload: Value = match serde_json::from_str(&request.r#payload_json) {
                    Ok(v) => v,
                    Err(_) => {
                        return fail_response(
                            &request,
                            Failure::invalid("Probe payload is not valid JSON"),
                        )
                    }
                };
                match probe(host, &payload, request.r#budget_ms) {
                    Ok(v) => v,
                    Err(e) => return fail_response(&request, e),
                }
            }
            "submit" => {
                let payload: Value = match serde_json::from_str(&request.r#payload_json) {
                    Ok(v) => v,
                    Err(_) => {
                        return fail_response(
                            &request,
                            Failure::invalid("Submit does not match the public request contract"),
                        )
                    }
                };
                match submit(host, &request, &payload) {
                    Ok(v) => v,
                    Err(e) => {
                        let vendor = if e.code == "ACCEPTANCE_UNKNOWN" {
                            Some("CHECKPOINT_OR_TRANSPORT".into())
                        } else {
                            None
                        };
                        return fail_response_vendor(&request, e, vendor);
                    }
                }
            }
            "query" => {
                let payload: Value = match serde_json::from_str(&request.r#payload_json) {
                    Ok(v) => v,
                    Err(_) => {
                        return fail_response(
                            &request,
                            Failure::invalid("Query does not match the public request contract"),
                        )
                    }
                };
                match query(host, &request, &payload) {
                    Ok(v) => v,
                    Err(e) => return fail_response(&request, e),
                }
            }
            "cancel" => {
                return fail_response(
                    &request,
                    Failure {
                        code: "UNSUPPORTED_FEATURE",
                        message: "Cancellation is unsupported for this panorama package".into(),
                    },
                )
            }
            _ => {
                return fail_response(
                    &request,
                    Failure {
                        code: "UNSUPPORTED_FEATURE",
                        message: "Operation is not implemented by this package".into(),
                    },
                )
            }
        };
        // Silence unused warning in wasm builds without rewriting.
        let _ = format!("{OPERATION_ID}");
        guest::completed_response(&request, &result).expect("serializable public result")
    }
}

bgx_adapter_v2_sdk::export_adapter!(PanoramaImage);
