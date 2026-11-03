//! Mask-only tracking-preview submit/query/cancel for video-target-keyframes-v1.
//! Platform Comfy IDs and companion HTTP paths stay inside this package.
use alloc::string::{String, ToString};
use alloc::vec;
use bgx_adapter_v2_sdk::Host;
use serde_json::{json, Value};

use crate::comfy::{self, text, Ctx, Remote};
use crate::graph;
use crate::mapping;
use crate::target_keyframes::{self, Direction, TrackingRequest};
use crate::Failure;

const TRACKING_PROGRESS_BASE: &str = "/bgx/wan22-animate/tracking";

fn invalid(message: &'static str) -> Failure {
    Failure::new("INVALID_INPUT", message)
}

/// Submit a mask-only tracking preview graph. Never includes diffusion nodes.
pub fn submit(
    h: &mut dyn Host,
    ctx: &Ctx,
    request_id: &str,
    _p: &Value,
    tr: &TrackingRequest,
    source_file: &str,
    source_w: u64,
    source_h: u64,
    source_digest: &str,
    input_schema_digest: &str,
) -> Result<Value, Failure> {
    comfy::ensure_companion_nodes_tracking(h, ctx)?;
    let mapped = mapping::scale_keyframes(
        &tr.selection.keyframes,
        source_w,
        source_h,
        tr.width,
        tr.height,
    )?;
    let keyframes_json = target_keyframes::encode_keyframes_json(&mapped);
    let frame_count = if tr.sampled_frames > 0 {
        tr.sampled_frames
    } else {
        let span = tr.end_ms.saturating_sub(tr.start_ms);
        let n = (span.saturating_mul(tr.fps).saturating_add(999)) / 1000;
        if n == 0 { 1 } else { n }
    };
    // Seed queued progress via companion HTTP (real facts only; no fake %).
    seed_progress(
        h,
        ctx,
        request_id,
        &tr.selection.selection_digest,
        tr.direction,
        frame_count,
    )?;
    let graph = graph::build_tracking(
        tr,
        &graph::TrackingInputs {
            source_file,
            keyframes_json: &keyframes_json,
            frame_count,
            request_id,
            source_digest,
            input_schema_digest,
            mapped_keyframes: &mapped,
        },
    );
    let prompt_id = comfy::submit(h, ctx, &graph, request_id)?;
    Ok(json!({
        "kind": "accepted",
        "vendorTaskId": prompt_id,
        "appliedParams": {"direction": tr.direction.as_str()},
        "resumeToken": {
            "promptId": prompt_id,
            "trackingRequestId": request_id,
            "selectionDigest": tr.selection.selection_digest,
            "direction": tr.direction.as_str()
        },
        "usage": null
    }))
}

fn seed_progress(
    h: &mut dyn Host,
    ctx: &Ctx,
    request_id: &str,
    selection_digest: &str,
    direction: Direction,
    total_frames: u64,
) -> Result<(), Failure> {
    let path = alloc::format!("{TRACKING_PROGRESS_BASE}/{request_id}/progress");
    let body = serde_json::to_vec(&json!({
        "requestId": request_id,
        "revision": "1",
        "selectionDigest": selection_digest,
        "stage": "queued",
        "direction": direction.as_str(),
        "totalFrames": total_frames,
        "processedFrames": 0,
        "lastCompletedTimeMs": null,
        "coveredRanges": []
    }))
    .map_err(|_| invalid("progress payload"))?;
    comfy::exchange_public(
        h,
        ctx,
        "POST",
        &path,
        json!({"kind": "bytes", "bytes": body}),
        vec![json!({"name": "Content-Type", "value": "application/json"})],
        false,
    )?;
    Ok(())
}

/// Query mask-only task; prefer companion progress facts when present.
pub fn query(
    h: &mut dyn Host,
    ctx: &Ctx,
    p: &Value,
    vendor_task_id: &str,
) -> Result<Value, Failure> {
    let tracking_request_id = p
        .pointer("/resumeToken/trackingRequestId")
        .and_then(Value::as_str)
        .unwrap_or(vendor_task_id);
    let progress = match fetch_progress(h, ctx, tracking_request_id) {
        Ok(value) => Some(value),
        Err(error) if error.code == "UPSTREAM_ERROR" => None,
        Err(error) => return Err(error),
    };
    let selection_digest = p.pointer("/resumeToken/selectionDigest").and_then(Value::as_str).unwrap_or("");
    let direction = p.pointer("/resumeToken/direction").and_then(Value::as_str).unwrap_or("both");
    let remote = comfy::status(h, ctx, vendor_task_id, graph::TRACKING_OUTPUT_NODE)?;
    match remote {
        Remote::Queued => {
            let data = public_progress(tracking_request_id, selection_digest, direction, "queued", progress);
            Ok(running_envelope("queued", data, p))
        }
        Remote::Running => {
            let stage = progress.as_ref().and_then(|item| item["stage"].as_str()).unwrap_or("loading").to_string();
            let data = public_progress(tracking_request_id, selection_digest, direction, &stage, progress);
            Ok(running_envelope("running", data, p))
        }
        Remote::Succeeded(_files) => {
            let (receipt, outputs) = publish_frames(h, ctx, tracking_request_id, selection_digest)?;
            Ok(json!({
                "status": "succeeded",
                "outputs": outputs,
                "data": receipt,
                "usage": null,
                "progress": Value::Null,
                "terminalFailure": null,
                "resumeToken": p["resumeToken"].clone()
            }))
        }
        Remote::Failed(vendor) => Ok(json!({
            "status": "failed",
            "outputs": [],
            "data": Value::Null,
            "usage": null,
            "progress": Value::Null,
            "terminalFailure": {
                "category": "vendorTaskFailed",
                "message": "ComfyUI reported an execution error",
                "vendorCode": vendor,
                "details": {"promptId": vendor_task_id}
            },
            "resumeToken": p["resumeToken"].clone()
        })),
        Remote::Lost => Ok(json!({
            "status": "failed",
            "outputs": [],
            "data": Value::Null,
            "usage": null,
            "progress": Value::Null,
            "terminalFailure": {
                "category": "vendorTaskFailed",
                "message": "ComfyUI no longer knows this prompt (service restarted or history cleared)",
                "vendorCode": "PROMPT_NOT_FOUND",
                "details": {"promptId": vendor_task_id}
            },
            "resumeToken": p["resumeToken"].clone()
        })),
    }
}

fn running_envelope(status: &str, data: Value, checkpoint: &Value) -> Value {
    json!({
        "status": status,
        "outputs": [],
        "data": data,
        "usage": null,
        "progress": envelope_progress_from_profile(&data),
        "terminalFailure": null,
        "resumeToken": checkpoint["resumeToken"].clone()
    })
}

fn public_progress(request_id: &str, selection_digest: &str, direction: &str, stage: &str, fetched: Option<Value>) -> Value {
    let mut body = fetched.unwrap_or_else(|| json!({
        "revision": "0",
        "selectionDigest": selection_digest,
        "stage": stage,
        "direction": direction,
        "totalFrames": null,
        "processedFrames": null,
        "lastCompletedTimeMs": null,
        "coveredRanges": []
    }));
    if let Some(obj) = body.as_object_mut() {
        obj.insert("requestId".into(), json!(request_id));
        obj.entry("stage").or_insert_with(|| json!(stage));
        obj.entry("direction").or_insert_with(|| json!(direction));
        obj.entry("selectionDigest").or_insert_with(|| json!(selection_digest));
        obj.entry("revision").or_insert_with(|| json!("0"));
        obj.entry("coveredRanges").or_insert_with(|| json!([]));
        obj.entry("totalFrames").or_insert(Value::Null);
        obj.entry("processedFrames").or_insert(Value::Null);
        obj.entry("lastCompletedTimeMs").or_insert(Value::Null);
    }
    body
}

fn envelope_progress_from_profile(profile: &Value) -> Value {
    let fraction = match (profile["processedFrames"].as_u64(), profile["totalFrames"].as_u64()) {
        (Some(done), Some(total)) if total > 0 => json!(done as f64 / total as f64),
        _ => Value::Null,
    };
    json!({"fraction": fraction, "message": profile["stage"].as_str().unwrap_or("queued")})
}

fn fetch_progress(h: &mut dyn Host, ctx: &Ctx, request_id: &str) -> Result<Value, Failure> {
    let path = alloc::format!("{TRACKING_PROGRESS_BASE}/{request_id}/progress");
    comfy::exchange_public(h, ctx, "GET", &path, json!({"kind": "none"}), vec![], false)
}

/// Frames per public frame page.
const FRAMES_PER_PAGE: usize = 60;
const ARTIFACT_DIR: &str = "bgx-wan22-tracking";

fn upstream(message: &'static str) -> Failure {
    Failure::new("UPSTREAM_ERROR", message)
}

fn sha_ref(stat: &Value) -> Result<String, Failure> {
    let raw = stat["sha256"].as_str().ok_or_else(|| upstream("Managed media has no digest"))?;
    Ok(if raw.starts_with("sha256:") { raw.to_string() } else { alloc::format!("sha256:{raw}") })
}

/// Page layout of the sealed frame list: (cursor, nextCursor, startMs, endMs, frames).
fn paginate(frames: &[Value], span_ms: u64) -> Result<alloc::vec::Vec<(alloc::string::String, Option<alloc::string::String>, u64, u64, alloc::vec::Vec<Value>)>, Failure> {
    let chunks: alloc::vec::Vec<&[Value]> = frames.chunks(FRAMES_PER_PAGE).collect();
    let mut pages = alloc::vec::Vec::new();
    for (n, chunk) in chunks.iter().enumerate() {
        let start = chunk[0]["timeMs"].as_u64().ok_or_else(|| upstream("Tracking frame has no time"))?;
        let end = match chunks.get(n + 1) {
            Some(next) => next[0]["timeMs"].as_u64().ok_or_else(|| upstream("Tracking frame has no time"))?,
            None => span_ms,
        };
        if end <= start {
            return Err(upstream("Tracking frame times are not increasing"));
        }
        let next = (n + 1 < chunks.len()).then(|| alloc::format!("p{}", n + 1));
        pages.push((alloc::format!("p{n}"), next, start, end, chunk.to_vec()));
    }
    Ok(pages)
}

/// Seal a finished request as public outputs: every covered mask frame as an image, frame pages
/// and one frame index as JSON files, and the tracking receipt pointing at that index.
fn publish_frames(
    h: &mut dyn Host,
    ctx: &Ctx,
    request_id: &str,
    selection_digest: &str,
) -> Result<(Value, alloc::vec::Vec<Value>), Failure> {
    let path = alloc::format!("{TRACKING_PROGRESS_BASE}/{request_id}/manifest");
    let man = comfy::exchange_public(h, ctx, "GET", &path, json!({"kind": "none"}), vec![], false)?;
    if text(&man, "requestId")? != request_id || text(&man, "selectionDigest")? != selection_digest {
        return Err(upstream("Tracking manifest belongs to another request"));
    }
    let digest = text(&man, "digest")?.to_string();
    let span_ms = man["range"]["endMs"].as_u64().zip(man["range"]["startMs"].as_u64())
        .and_then(|(end, start)| end.checked_sub(start))
        .filter(|span| *span > 0)
        .ok_or_else(|| upstream("Tracking manifest range is invalid"))?;
    let raw_frames = man["frames"].as_array().filter(|f| !f.is_empty())
        .ok_or_else(|| upstream("Tracking manifest has no frames"))?;
    let mut outputs = alloc::vec::Vec::new();
    let mut frames = alloc::vec::Vec::new();
    let subfolder = alloc::format!("{ARTIFACT_DIR}/{request_id}/masks");
    for frame in raw_frames {
        let state = text(frame, "state")?;
        let mask_ref = if state == "covered" {
            let rel = text(frame, "relPath")?;
            let name = rel.strip_prefix("masks/")
                .filter(|name| bgx_adapter_v2_sdk::guest::valid_package_path(name) && !name.contains('/') && name.ends_with(".png"))
                .ok_or_else(|| upstream("Tracking mask frame path is invalid"))?;
            let (handle, stat) = comfy::download_image(h, ctx, name, &subfolder, "output")?;
            outputs.push(comfy::media_output("maskFrame", &handle, &stat));
            json!({"artifactId": stat["id"], "digest": sha_ref(&stat)?})
        } else {
            Value::Null
        };
        frames.push(json!({"frameIndex": frame["frameIndex"], "timeMs": frame["timeMs"], "state": state, "maskPageRef": mask_ref}));
    }
    let mut index_pages = alloc::vec::Vec::new();
    for (cursor, next, start, end, page_frames) in paginate(&frames, span_ms)? {
        let page = json!({
            "requestId": request_id,
            "selectionDigest": selection_digest,
            "page": {"cursor": cursor, "nextCursor": next, "complete": next.is_none()},
            "frames": page_frames
        });
        let bytes = serde_json::to_vec(&page).map_err(|_| upstream("Tracking page cannot be encoded"))?;
        let (handle, stat) = comfy::create_file(h, "application/json", &bytes)?;
        outputs.push(comfy::media_output("framePage", &handle, &stat));
        index_pages.push(json!({"cursor": cursor, "nextCursor": next, "startMs": start, "endMs": end,
            "mediaId": stat["id"], "available": true}));
    }
    let index = json!({"requestId": request_id, "selectionDigest": selection_digest, "complete": true, "pages": index_pages});
    let bytes = serde_json::to_vec(&index).map_err(|_| upstream("Tracking index cannot be encoded"))?;
    let (handle, stat) = comfy::create_file(h, "application/json", &bytes)?;
    outputs.push(comfy::media_output("frameIndex", &handle, &stat));
    let index_id = stat["id"].clone();
    let receipt = json!({
        "requestId": request_id,
        "sourceDigest": man["sourceDigest"],
        "range": man["range"],
        "direction": man["direction"],
        "selectionDigest": selection_digest,
        "operationRef": man["operationRef"],
        "outputGeometry": man["outputGeometry"],
        "outputFps": man.get("outputFps").cloned().unwrap_or(Value::Null),
        "resolvedKeyframes": man["resolvedKeyframes"],
        "coveredRanges": man["coveredRanges"],
        "lostRanges": man["lostRanges"],
        "uncomputedRanges": man["uncomputedRanges"],
        "skipRanges": man["skipRanges"],
        "ready": man["ready"],
        "readyReason": man.get("readyReason").cloned().unwrap_or(Value::Null),
        "previewMediaRef": Value::Null,
        "maskArtifact": {"artifactId": request_id, "digest": digest, "pageIndexRef": index_id},
        "framePageIndexRef": index_id
    });
    Ok((receipt, outputs))
}

/// Cancel at frame boundary: interrupt current execution and delete queued item.
/// ACCEPTANCE_UNKNOWN paths must not resubmit.
pub fn cancel(
    h: &mut dyn Host,
    ctx: &Ctx,
    vendor_task_id: &str,
) -> Result<Value, Failure> {
    // Best-effort interrupt; Comfy applies between nodes (frame boundary for our graph).
    let _ = comfy::exchange_public(
        h,
        ctx,
        "POST",
        "/interrupt",
        json!({"kind": "none"}),
        vec![],
        false,
    );
    let body = serde_json::to_vec(&json!({
        "delete": [vendor_task_id]
    }))
    .map_err(|_| Failure::new("INTERNAL_ERROR", "cancel payload"))?;
    // Do not treat lost acknowledgment as a signal to resubmit tracking.
    match comfy::exchange_public(
        h,
        ctx,
        "POST",
        "/queue",
        json!({"kind": "bytes", "bytes": body}),
        vec![json!({"name": "Content-Type", "value": "application/json"})],
        true,
    ) {
        Ok(_) => Ok(json!({"status": "cancelled", "vendorTaskId": vendor_task_id})),
        Err(e) if e.code == "ACCEPTANCE_UNKNOWN" => Ok(json!({
            "status": "cancel_unknown",
            "vendorTaskId": vendor_task_id,
            "message": "Cancel acknowledgment was lost; do not resubmit"
        })),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::String, vec::Vec};
    use bgx_adapter_v2_sdk::{host::HostError, HostPortCallV2};

    /// Companion manifest over HTTP, mask PNGs into managed media, package-built JSON files.
    struct FakeHost { manifest: Value, files: Vec<(String, Vec<u8>)>, next: u32 }
    impl Host for FakeHost {
        fn call(&mut self, call: HostPortCallV2) -> Result<Value, HostError> {
            let call = serde_json::to_value(call).unwrap();
            let args = &call["args"];
            self.next += 1;
            Ok(match call["method"].as_str().unwrap() {
                "host-http.execute" if args["plan"]["url"].as_str().unwrap().contains("/view?") =>
                    json!({"status": 200, "payload": {"handle": alloc::format!("png-{}", self.next)}}),
                "host-http.execute" =>
                    json!({"status": 200, "payload": {"bytes": serde_json::to_vec(&self.manifest).unwrap()}}),
                "host-media.create" => json!(alloc::format!("file-{}", self.next)),
                "host-media.write" => {
                    let handle = args["handle"].as_str().unwrap().to_string();
                    let bytes: Vec<u8> = serde_json::from_value(args["bytes"].clone()).unwrap();
                    self.files.push((handle, bytes));
                    Value::Null
                }
                "host-media.finish" => Value::Null,
                "host-media.info" => {
                    let handle = args["handle"].as_str().unwrap();
                    let kind = if handle.starts_with("png-") { "image" } else { "file" };
                    json!({"id": alloc::format!("id-{handle}"), "kind": kind, "mime": "x/y", "fileName": "f.bin",
                        "byteLength": "3", "sha256": "ab".repeat(32), "width": null, "height": null,
                        "durationMs": null, "timestampsMs": null})
                }
                other => panic!("unexpected port {other}"),
            })
        }
    }

    #[test]
    fn finished_request_publishes_pages_index_and_covered_masks_only() {
        let mut frames = Vec::new();
        for i in 0..61u64 {
            let state = if i == 5 { "lost" } else if i == 60 { "uncomputed" } else { "covered" };
            frames.push(json!({"frameIndex": i, "timeMs": i * 40, "state": state,
                "relPath": alloc::format!("masks/frame_{i:05}.png")}));
        }
        let manifest = json!({"requestId": "req-1", "selectionDigest": "sha256:sel", "digest": "sha256:mask",
            "range": {"startMs": 0, "endMs": 2440}, "frames": frames, "sourceDigest": "sha256:src",
            "direction": "both", "operationRef": {}, "outputGeometry": {"width": 16, "height": 16},
            "resolvedKeyframes": [], "coveredRanges": [], "lostRanges": [], "uncomputedRanges": [],
            "skipRanges": [], "ready": false, "readyReason": "uncomputed frames remain"});
        let config = json!({"endpointBaseUrl": "http://127.0.0.1:8188", "credentialRef": null});
        let ctx = Ctx { config: &config, budget_ms: 10000 };
        let mut h = FakeHost { manifest, files: Vec::new(), next: 0 };
        let (receipt, outputs) = publish_frames(&mut h, &ctx, "req-1", "sha256:sel").unwrap();
        let count = |role: &str| outputs.iter().filter(|o| o["role"] == role).count();
        assert_eq!((count("maskFrame"), count("framePage"), count("frameIndex")), (59, 2, 1));
        let index_id = &outputs.iter().find(|o| o["role"] == "frameIndex").unwrap()["media"]["id"];
        assert_eq!(&receipt["framePageIndexRef"], index_id);
        assert_eq!(&receipt["maskArtifact"]["pageIndexRef"], index_id);
        assert!(receipt.get("progress").is_none());
        let pages: Vec<Value> = h.files.iter().map(|(_, b)| serde_json::from_slice(b).unwrap()).collect();
        let index = pages.last().unwrap();
        assert_eq!(index["pages"][0]["nextCursor"], "p1");
        assert_eq!((index["pages"][1]["startMs"].as_u64(), index["pages"][1]["endMs"].as_u64()), (Some(2400), Some(2440)));
        let first = &pages[0];
        assert_eq!(first["page"]["complete"], false);
        assert!(first["frames"][5]["maskPageRef"].is_null());
        assert_eq!(first["frames"][0]["maskPageRef"]["digest"], alloc::format!("sha256:{}", "ab".repeat(32)));
        assert!(pages[1]["frames"][0]["maskPageRef"].is_null());
        assert_eq!(pages[1]["page"]["complete"], true);
    }

    #[test]
    fn manifest_of_another_request_is_rejected() {
        let config = json!({"endpointBaseUrl": "http://127.0.0.1:8188", "credentialRef": null});
        let ctx = Ctx { config: &config, budget_ms: 10000 };
        let mut h = FakeHost { manifest: json!({"requestId": "other", "selectionDigest": "sha256:sel"}), files: Vec::new(), next: 0 };
        assert!(publish_frames(&mut h, &ctx, "req-1", "sha256:sel").is_err());
    }
}
