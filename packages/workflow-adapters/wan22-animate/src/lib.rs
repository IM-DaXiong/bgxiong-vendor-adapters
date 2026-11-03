#![cfg_attr(target_arch = "wasm32", no_std)]
//! Wan 2.2 Animate character-replace adapter. Depends only on the public SDK;
//! frame windows, overlap, offsets, trimming, point encoding and node ids stay here.
extern crate alloc;

pub mod capability;
pub mod comfy;
pub mod graph;
pub mod mapping;
pub mod parameters;
pub mod planner;
pub mod target_keyframes;
pub mod tracking;

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

fn frame_timestamps_ms(stat: &Value) -> Result<Option<alloc::vec::Vec<u64>>, Failure> {
    match stat.get("timestampsMs") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.is_empty() => Err(Failure::new(
            "INVALID_INPUT",
            "source frame timestamps are unavailable",
        )),
        Some(Value::Array(items)) => {
            let mut stamps = alloc::vec::Vec::with_capacity(items.len());
            for item in items {
                let stamp = if let Some(n) = item.as_u64() {
                    n
                } else if let Some(text) = item.as_str() {
                    text.parse::<u64>().map_err(|_| {
                        Failure::new("INVALID_INPUT", "source frame timestamps are unavailable")
                    })?
                } else {
                    return Err(Failure::new(
                        "INVALID_INPUT",
                        "source frame timestamps are unavailable",
                    ));
                };
                stamps.push(stamp);
            }
            Ok(Some(stamps))
        }
        Some(_) => Err(Failure::new(
            "INVALID_INPUT",
            "source frame timestamps are unavailable",
        )),
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

fn operation(p: &Value) -> Result<parameters::Mode, Failure> {
    match text(p, "operationId")? {
        capability::OPERATION_ID => Ok(parameters::Mode::Mix),
        capability::MOVE_OPERATION_ID => Ok(parameters::Mode::Move),
        capability::TRACKING_OPERATION_ID => Ok(parameters::Mode::Tracking),
        _ => Err(Failure::new("UNSUPPORTED_FEATURE", "Unknown operation")),
    }
}

fn submit(h: &mut dyn Host, request: &Invocation, p: &Value) -> Result<Value, Failure> {
    let mode = operation(p)?;
    if mode == parameters::Mode::Tracking {
        return submit_tracking(h, request, p);
    }
    let decided = (|| {
        let ctx = Ctx {
            config: &p["configuration"],
            budget_ms: request.r#budget_ms,
        };
        let r = parameters::parse(&p["input"], mode)?;
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
        // This policy belongs to this independent adapter, never to the SDK/host.
        comfy::ensure_reference_preparation(h, &ctx)?;
        if r.mask_artifact.is_some() {
            comfy::ensure_companion_nodes_tracking(h, &ctx)?;
        }
        let points = match &r.replace {
            Some(rep) => {
                // Companion nodes must be present before any upload or GPU work.
                comfy::ensure_companion_nodes(h, &ctx)?;
                let stat = comfy::port(
                    h,
                    "host-media.info",
                    json!({"handle": text(source, "handle")?}),
                )?;
                let (sw, sh) = (
                    stat["width"].as_u64().unwrap_or(0),
                    stat["height"].as_u64().unwrap_or(0),
                );
                mapping::scale_points(&rep.positive_points, sw, sh, r.width, r.height)?
            }
            None => alloc::vec::Vec::new(),
        };
        Ok((ctx, r, plan, points))
    })();
    let (ctx, r, plan, points) = match decided {
        Ok(ready) => ready,
        Err(error) => return Err(persist_local_rejection(h, request, error)),
    };
    let source_file = uploaded(h, &ctx, p, &r.source_media_id)?;
    let replacement_file = uploaded(h, &ctx, p, &r.replacement_media_id)?;
    let replacement_file = comfy::prepare_reference(h, &ctx, &replacement_file, r.width, r.height)?;
    let graph = graph::build(
        &r,
        &plan,
        &graph::Inputs {
            source_file: &source_file,
            replacement_file: &replacement_file,
            points: &points,
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
    let mode = operation(p)?;
    if mode == parameters::Mode::Tracking {
        let ctx = Ctx { config: &p["configuration"], budget_ms: request.r#budget_ms };
        let id = text(p, "vendorTaskId")?;
        return tracking::query(h, &ctx, p, id);
    }
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
            // Package-wide resource inventory: tracking vs generation have different needs.
            // Missing generation models must not pretend the whole package is unusable.
            let report = comfy::resource_inventory(h, &ctx);
            Ok(json!({
                "packageValid": true,
                "connection": "ready",
                "authentication": "not_checked",
                "resources": report.status,
                "issues": report.issues
            }))
        }
        _ => Err(Failure::new("INVALID_INPUT", "Unknown probe kind")),
    }
}

/// Record a pre-upload rejection. A failed CAS leaves the checkpoint unknown.
fn persist_local_rejection(h: &mut dyn Host, request: &Invocation, failure: Failure) -> Failure {
    let Ok(read) = comfy::port(h, "host-checkpoint.read", json!({"requestId": request.r#request_id})) else {
        return failure;
    };
    let Ok(raw) = text(&read, "valueJson") else { return failure };
    let Ok(checkpoint) = serde_json::from_str::<Value>(raw) else { return failure };
    let Ok(revision) = text(&read, "revision") else { return failure };
    let Ok(next) = bgx_adapter_v2_sdk::submission_rejection::checkpoint_candidate(
        &checkpoint,
        "before_submission",
        failure.message,
        &json!({"code": failure.code}),
    ) else {
        return failure;
    };
    if comfy::port(
        h,
        "host-checkpoint.write",
        json!({"requestId": request.r#request_id, "expectedRevision": revision, "valueJson": next.to_string()}),
    )
    .is_err()
    {
        return failure;
    }
    failure
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


fn submit_tracking(h: &mut dyn Host, request: &Invocation, p: &Value) -> Result<Value, Failure> {
    let decided = (|| {
        let ctx = Ctx {
            config: &p["configuration"],
            budget_ms: request.r#budget_ms,
        };
        let parsed = target_keyframes::parse_tracking(&p["input"])?;
        let source = lease(p, &parsed.source_media_id)?;
        if source["kind"] != "video" {
            return Err(Failure::new("INVALID_INPUT", "tracking source must be a video"));
        }
        let stat = comfy::port(h, "host-media.info", json!({"handle": text(source, "handle")?}))?;
        let (sw, sh) = (
            stat["width"].as_u64().unwrap_or(0),
            stat["height"].as_u64().unwrap_or(0),
        );
        if sw == 0 || sh == 0 {
            return Err(Failure::new("INVALID_INPUT", "source video dimensions are unavailable"));
        }
        let source_digest = stat
            .get("sha256")
            .and_then(Value::as_str)
            .map(|s| {
                if s.starts_with("sha256:") {
                    String::from(s)
                } else {
                    alloc::format!("sha256:{s}")
                }
            })
            .filter(|s| s != "sha256:unknown" && s.len() > "sha256:".len())
            .ok_or_else(|| Failure::new("INVALID_INPUT", "source content digest is unavailable"))?;
        if source_digest != parsed.source_digest {
            return Err(Failure::new("INVALID_INPUT", "source digest does not match the prepared video"));
        }
        // Public MediaStat has no fps field; tracking requires real timestampsMs only.
        let tr = if let Some(stamps) = frame_timestamps_ms(&stat)? {
            target_keyframes::apply_source_sampling_at_frames(&p["input"], sw, sh, &stamps)?
        } else {
            return Err(Failure::new(
                "INVALID_INPUT",
                "source frame timing is unavailable",
            ));
        };
        Ok((ctx, tr, sw, sh, source_digest))
    })();
    let (ctx, tr, sw, sh, source_digest) = match decided {
        Ok(ready) => ready,
        Err(error) => return Err(persist_local_rejection(h, request, error)),
    };
    let source_file = uploaded(h, &ctx, p, &tr.source_media_id)?;
    let caps = capability::capabilities();
    let input_schema_digest = caps["operations"]
        .as_array()
        .and_then(|ops| ops.iter().find(|o| o["id"] == capability::TRACKING_OPERATION_ID))
        .and_then(|o| o["inputSchema"]["digest"].as_str())
        .unwrap_or("")
        .to_string();
    let accepted = tracking::submit(
        h,
        &ctx,
        &request.r#request_id,
        p,
        &tr,
        &source_file,
        sw,
        sh,
        &source_digest,
        &input_schema_digest,
    )?;
    persist_accepted(h, request, &accepted)?;
    Ok(accepted)
}

fn cancel(h: &mut dyn Host, request: &Invocation, p: &Value) -> Result<Value, Failure> {
    let _mode = operation(p)?;
    let ctx = Ctx {
        config: &p["configuration"],
        budget_ms: request.r#budget_ms,
    };
    let id = text(p, "vendorTaskId")?;
    tracking::cancel(h, &ctx, id)
}

fn finish(request: &Invocation, result: Result<Value, Failure>) -> Response {
    match result {
        Ok(value) => guest::completed_response(request, &value).expect("standard serializable result"),
        Err(e) => serde_json::from_value(json!({"protocolVersion": 2, "operation": request.r#operation, "error": {"code": e.code,
            "message": e.message, "retryable": false, "vendorCode": e.vendor, "vendorRequestId": null}})).expect("public error"),
    }
}

pub struct Wan22Animate;
impl Adapter for Wan22Animate {
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
                "cancel" => cancel(h, &request, &p),
                _ => Err(Failure::new(
                    "UNSUPPORTED_FEATURE",
                    "Unsupported transport operation",
                )),
            }
        })();
        finish(&request, result)
    }
}
bgx_adapter_v2_sdk::export_adapter!(Wan22Animate);

#[cfg(test)]
mod rejection_tests {
    use super::*;
    use bgx_adapter_v2_sdk::{
        host::invalid_wire, HostPortCallV2, Invocation, InvocationCapabilitySlot,
        InvocationNegotiatedVersion, InvocationOperation, InvocationProtocolVersion,
    };

    struct Fake {
        checkpoint: Value,
        digest: String,
        fail_write: bool,
    }

    impl Host for Fake {
        fn call(&mut self, call: HostPortCallV2) -> Result<Value, bgx_adapter_v2_sdk::host::HostError> {
            let wire = serde_json::to_value(&call).expect("port call");
            match wire["method"].as_str() {
                Some("host-checkpoint.read") => Ok(json!({
                    "revision": "1",
                    "valueJson": self.checkpoint.to_string(),
                })),
                Some("host-checkpoint.write") => {
                    if self.fail_write || wire["args"]["expectedRevision"] != "1" {
                        return Err(invalid_wire("checkpoint cas conflict"));
                    }
                    self.checkpoint = serde_json::from_str(wire["args"]["valueJson"].as_str().unwrap()).unwrap();
                    Ok(json!({}))
                }
                Some("host-media.info") => Ok(json!({
                    "width": 64,
                    "height": 64,
                    "sha256": self.digest,
                })),
                other => Err(invalid_wire(other.unwrap_or("missing method"))),
            }
        }
    }

    fn request() -> Invocation {
        Invocation {
            r#protocol_version: InvocationProtocolVersion,
            r#operation: InvocationOperation::V3,
            r#profile_id: String::from("video-target-keyframes-v1"),
            r#capability_slot: InvocationCapabilitySlot::V2,
            r#request_id: String::from("req-timing"),
            r#budget_ms: 1000,
            r#payload_json: String::from("{}"),
            r#binding_id: String::from("binding"),
            r#negotiated_version: InvocationNegotiatedVersion,
            r#enabled_features: alloc::vec::Vec::new(),
        }
    }

    fn unknown_checkpoint() -> Value {
        json!({
            "requestId": "req-timing",
            "requestDigest": "digest",
            "state": "acceptance_unknown",
            "revision": "1",
            "vendorTaskId": null,
            "resumeToken": null,
            "result": null
        })
    }

    #[test]
    fn missing_frame_timing_writes_rejected_checkpoint() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../contract/v2/profiles/video-target-keyframes-v1/fixtures/receipts.json"
        ))
        .unwrap();
        let input = fixtures["valid"]["trackingInput"].clone();
        let digest = input["sourceIdentity"]["contentDigest"].as_str().unwrap().to_string();
        let media_id = input["sourceIdentity"]["mediaId"].as_str().unwrap();
        let mut host = Fake { checkpoint: unknown_checkpoint(), digest, fail_write: false };
        let payload = json!({
            "configuration": {},
            "input": input,
            "media": [{"id": media_id, "kind": "video", "handle": "lease-1"}]
        });
        let error = submit_tracking(&mut host, &request(), &payload).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
        assert_eq!(error.message, "source frame timing is unavailable");
        assert_eq!(host.checkpoint["state"], "rejected");
        assert_eq!(host.checkpoint["submissionRejection"]["basis"], "before_submission");
    }

    #[test]
    fn checkpoint_cas_conflict_keeps_acceptance_unknown() {
        let mut host = Fake { checkpoint: unknown_checkpoint(), digest: String::new(), fail_write: true };
        let error = persist_local_rejection(&mut host, &request(), Failure::new("INVALID_INPUT", "source frame timing is unavailable"));
        assert_eq!(error.code, "INVALID_INPUT");
        assert_eq!(host.checkpoint["state"], "acceptance_unknown");
        assert!(host.checkpoint.get("submissionRejection").is_none());
    }

    #[test]
    fn fps_field_on_mediastat_is_ignored_without_timestamps() {
        // MediaStat.fps is not a public contract field; even if a host forged it,
        // tracking must still require timestampsMs.
        struct FpsHost { checkpoint: Value, digest: String }
        impl Host for FpsHost {
            fn call(&mut self, call: HostPortCallV2) -> Result<Value, bgx_adapter_v2_sdk::host::HostError> {
                let wire = serde_json::to_value(&call).expect("port call");
                match wire["method"].as_str() {
                    Some("host-checkpoint.read") => Ok(json!({
                        "revision": "1",
                        "valueJson": self.checkpoint.to_string(),
                    })),
                    Some("host-checkpoint.write") => {
                        self.checkpoint = serde_json::from_str(wire["args"]["valueJson"].as_str().unwrap()).unwrap();
                        Ok(json!({}))
                    }
                    Some("host-media.info") => Ok(json!({
                        "width": 64,
                        "height": 64,
                        "fps": 24,
                        "sha256": self.digest,
                    })),
                    other => Err(invalid_wire(other.unwrap_or("missing method"))),
                }
            }
        }
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../contract/v2/profiles/video-target-keyframes-v1/fixtures/receipts.json"
        ))
        .unwrap();
        // Prefer worktree SDK path if present
        let input = fixtures["valid"]["trackingInput"].clone();
        let digest = input["sourceIdentity"]["contentDigest"].as_str().unwrap().to_string();
        let media_id = input["sourceIdentity"]["mediaId"].as_str().unwrap();
        let mut host = FpsHost { checkpoint: unknown_checkpoint(), digest };
        let payload = json!({
            "configuration": {},
            "input": input,
            "media": [{"id": media_id, "kind": "video", "handle": "lease-1"}]
        });
        let error = submit_tracking(&mut host, &request(), &payload).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
        assert_eq!(error.message, "source frame timing is unavailable");
        assert_eq!(host.checkpoint["state"], "rejected");
        assert_eq!(host.checkpoint["submissionRejection"]["basis"], "before_submission");
    }
}
