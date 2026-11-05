#![cfg_attr(target_arch = "wasm32", no_std)]
//! Wan Animate 2 (distilled) character-animation adapter. Depends only on the public SDK;
//! segment chaining, trimming and node ids stay here.
extern crate alloc;

pub mod capability;
pub mod comfy;
pub mod graph;
pub mod mapping;
pub mod parameters;
pub mod planner;

use alloc::string::{String, ToString};
use bgx_adapter_v2_sdk::{guest, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};

use comfy::{text, Ctx, Remote};

/// Standard SDK error code plus a stable, non-vendor message.
#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: &'static str,
    pub vendor: Option<String>,
}
impl Failure {
    pub fn new(code: &'static str, message: &'static str) -> Self {
        Failure {
            code,
            message,
            vendor: None,
        }
    }
}

fn lease<'a>(p: &'a Value, id: &str) -> Result<&'a Value, Failure> {
    p["media"]
        .as_array()
        .and_then(|m| m.iter().find(|x| x["id"] == id))
        .ok_or_else(|| {
            Failure::new(
                "INVALID_INPUT",
                "A media reference does not name a supplied media lease",
            )
        })
}

/// Reuses a successful explicit upload proof, otherwise uploads once (never retried).
fn uploaded(h: &mut dyn Host, ctx: &Ctx, p: &Value, id: &str) -> Result<String, Failure> {
    if let Some(proof) = p["uploads"]
        .as_array()
        .and_then(|u| u.iter().find(|x| x["mediaId"] == id))
    {
        return Ok(String::from(text(proof, "uploadRef")?));
    }
    comfy::upload(h, ctx, lease(p, id)?)
}

fn operation(p: &Value) -> Result<(), Failure> {
    match text(p, "operationId")? {
        capability::OPERATION_ID => Ok(()),
        _ => Err(Failure::new("UNSUPPORTED_FEATURE", "Unknown operation")),
    }
}

fn submit(h: &mut dyn Host, request: &Invocation, p: &Value) -> Result<Value, Failure> {
    operation(p)?;
    let ctx = Ctx {
        config: &p["configuration"],
        budget_ms: request.r#budget_ms,
    };
    let r = parameters::parse(&p["input"])?;
    let plan = planner::plan(&r)?;
    // All locally decidable rejections happen before any upload or GPU work.
    let source = lease(p, &r.source_media_id)?;
    let replacement = lease(p, &r.replacement_media_id)?;
    if source["kind"] != "video" || replacement["kind"] != "image" {
        return Err(Failure::new(
            "INVALID_INPUT",
            "source must be a video and replacement an image",
        ));
    }
    let source_file = uploaded(h, &ctx, p, &r.source_media_id)?;
    let replacement_file = uploaded(h, &ctx, p, &r.replacement_media_id)?;
    let graph = graph::build(
        &r,
        &plan,
        &graph::Inputs {
            source_file: &source_file,
            replacement_file: &replacement_file,
        },
    );
    let id = comfy::submit(h, &ctx, &graph, &request.r#request_id)?;
    // SDK rule: appliedParams may only report keys of the frozen input (merged and revalidated).
    let applied = json!({"width": r.width, "height": r.height, "fps": r.fps});
    let accepted = json!({"kind": "accepted", "vendorTaskId": id, "appliedParams": applied, "resumeToken": {"promptId": id}, "usage": null});
    persist_accepted(h, request, &accepted)?;
    Ok(accepted)
}

fn query(h: &mut dyn Host, request: &Invocation, p: &Value) -> Result<Value, Failure> {
    operation(p)?;
    let ctx = Ctx {
        config: &p["configuration"],
        budget_ms: request.r#budget_ms,
    };
    let id = text(p, "vendorTaskId")?;
    let (status, outputs, failure) = match comfy::status(h, &ctx, id, graph::OUTPUT_NODE)? {
        Remote::Queued => ("queued", json!([]), Value::Null),
        Remote::Running => ("running", json!([]), Value::Null),
        Remote::Succeeded(files) => {
            let artifact = comfy::download(h, &ctx, &files[0])?;
            ("succeeded", json!([artifact]), Value::Null)
        }
        Remote::Failed(vendor) => (
            "failed",
            json!([]),
            json!({"category": "vendorTaskFailed", "message": "ComfyUI reported an execution error", "vendorCode": vendor, "details": {"promptId": id}}),
        ),
        Remote::Lost => (
            "failed",
            json!([]),
            json!({"category": "vendorTaskFailed", "message": "ComfyUI no longer knows this prompt (service restarted or history cleared)", "vendorCode": "PROMPT_NOT_FOUND", "details": {"promptId": id}}),
        ),
    };
    let progress = if matches!(status, "queued" | "running") {
        json!({"fraction": null, "message": status})
    } else {
        Value::Null
    };
    Ok(
        json!({"status": status, "outputs": outputs, "data": {"promptId": id}, "usage": null, "progress": progress,
        "terminalFailure": failure, "resumeToken": p["resumeToken"].clone()}),
    )
}

fn explicit_upload(h: &mut dyn Host, request: &Invocation, p: &Value) -> Result<Value, Failure> {
    let ctx = Ctx {
        config: &p["configuration"],
        budget_ms: request.r#budget_ms,
    };
    let reference = comfy::upload(h, &ctx, &p["media"])?;
    Ok(json!({"mediaId": p["media"]["id"], "uploadRef": reference}))
}

fn probe(h: &mut dyn Host, request: &Invocation, p: &Value) -> Result<Value, Failure> {
    match text(p, "kind")? {
        "package_validation" => Ok(
            json!({"packageValid": true, "connection": "not_checked", "authentication": "not_checked", "resources": "not_checked", "issues": []}),
        ),
        "connectivity" => {
            let ctx = Ctx {
                config: &p["configuration"],
                budget_ms: request.r#budget_ms,
            };
            comfy::status_probe(h, &ctx)?;
            Ok(
                json!({"packageValid": true, "connection": "ready", "authentication": "not_checked", "resources": "not_checked", "issues": []}),
            )
        }
        _ => Err(Failure::new("INVALID_INPUT", "Unknown probe kind")),
    }
}

fn persist_accepted(
    h: &mut dyn Host,
    request: &Invocation,
    accepted: &Value,
) -> Result<(), Failure> {
    let lost = || Failure {
        code: "ACCEPTANCE_UNKNOWN",
        message: "ComfyUI accepted the prompt but checkpoint storage failed; do not resubmit",
        vendor: Some(String::from("CHECKPOINT_COMMIT_FAILED")),
    };
    let read = comfy::port(
        h,
        "host-checkpoint.read",
        json!({"requestId": request.r#request_id}),
    )
    .map_err(|_| lost())?;
    let mut checkpoint: Value =
        serde_json::from_str(text(&read, "valueJson").map_err(|_| lost())?).map_err(|_| lost())?;
    let old = text(&read, "revision").map_err(|_| lost())?;
    let revision = old
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(lost)?;
    checkpoint["revision"] = json!(alloc::format!("{revision}"));
    checkpoint["state"] = json!("accepted");
    checkpoint["vendorTaskId"] = accepted["vendorTaskId"].clone();
    checkpoint["resumeToken"] = accepted["resumeToken"].clone();
    checkpoint["result"] = accepted.clone();
    comfy::port(h, "host-checkpoint.write", json!({"requestId": request.r#request_id, "expectedRevision": old, "valueJson": checkpoint.to_string()}))
        .map_err(|_| lost())?;
    Ok(())
}

fn finish(request: &Invocation, result: Result<Value, Failure>) -> Response {
    match result {
        Ok(value) => guest::completed_response(request, &value).expect("standard serializable result"),
        Err(e) => serde_json::from_value(json!({"protocolVersion": 2, "operation": request.r#operation, "error": {"code": e.code,
            "message": e.message, "retryable": false, "vendorCode": e.vendor, "vendorRequestId": null}})).expect("public error"),
    }
}

pub struct WanAnimate2;
impl Adapter for WanAnimate2 {
    fn invoke(request: Invocation, h: &mut dyn Host) -> Response {
        let value = serde_json::to_value(&request).expect("typed invocation");
        let result = (|| {
            let p: Value = serde_json::from_str(&request.r#payload_json)
                .map_err(|_| Failure::new("INVALID_INPUT", "Payload is not JSON"))?;
            match text(&value, "operation")? {
                "capabilities" => Ok(capability::capabilities()),
                "probe" => probe(h, &request, &p),
                "upload" => explicit_upload(h, &request, &p),
                "submit" => submit(h, &request, &p),
                "query" => query(h, &request, &p),
                _ => Err(Failure::new(
                    "UNSUPPORTED_FEATURE",
                    "Unsupported transport operation",
                )),
            }
        })();
        finish(&request, result)
    }
}
bgx_adapter_v2_sdk::export_adapter!(WanAnimate2);
