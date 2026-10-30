#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{format, string::{String, ToString}, vec, vec::Vec};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// no_std-safe nearest-int for timestamp ms (wasm guest has no std float math).
fn round_ms(x: f64) -> i64 {
    if x >= 0.0 {
        (x + 0.5) as i64
    } else {
        (x - 0.5) as i64
    }
}

pub const PLUGIN: &str = "local.thirdparty.h3-face-refine";
pub const OP_ANALYZE: &str = "local.thirdparty.h3-face-refine/subjects.analyze";
pub const OP_PREVIEW: &str = "local.thirdparty.h3-face-refine/subjects.preview";
pub const OP_REFINE: &str = "local.thirdparty.h3-face-refine/video.refine";
pub const MODEL: &str = "h3-face-refine-1.1.2";
pub const UPSTREAM_COMMIT: &str = "d8521d14fe0d721d80cd9417fff5a559cbc21aba";

type Result<T> = core::result::Result<T, Failure>;

#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
}

fn fail(code: &'static str, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
    }
}

fn invalid(s: &str) -> Failure {
    fail("INVALID_INPUT", s)
}

fn schema(id: &str, s: &str) -> Value {
    let d: String = Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({"id": id, "digest": d, "schemaJson": s})
}

fn port(h: &mut dyn Host, method: &str, args: Value) -> Result<Value> {
    host::call(h, method, &args).map_err(|e| {
        let v = serde_json::to_value(&e).expect("public error");
        let code = match v["code"].as_str() {
            Some("PERMISSION_DENIED") => "PERMISSION_DENIED",
            Some("TIMEOUT") => "TIMEOUT",
            Some("BUDGET_EXCEEDED") => "BUDGET_EXCEEDED",
            _ => "RESOURCE_UNAVAILABLE",
        };
        fail(code, format!("managed port {method} failed: {}", host::diagnostic(&e)))
    })
}

fn cfg_base(configuration: &Value) -> Result<&str> {
    configuration
        .get("endpointBaseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("configuration.endpointBaseUrl is required"))
}

fn http_json(h: &mut dyn Host, method: &str, url: &str, body: Option<Value>) -> Result<Value> {
    let mut args = json!({
        "method": method,
        "url": url,
        "headers": [{"name": "Content-Type", "value": "application/json"}]
    });
    if let Some(b) = body {
        args["bodyJson"] = b;
    }
    // Prefer generic httpJson if present; fall back to raw shapes used by LAN Comfy examples.
    match port(h, "http.requestJson", args.clone()) {
        Ok(v) => Ok(v),
        Err(_) => {
            // Some SDK builds expose httpFetch
            port(h, "httpFetch", args)
        }
    }
}

pub fn capabilities() -> Value {
    let analyze_in = schema("analyze_in", include_str!("../schemas/analyze.input.schema.json"));
    let analyze_out = schema("analyze_out", include_str!("../schemas/analyze.output.schema.json"));
    let preview_in = schema("preview_in", include_str!("../schemas/preview.input.schema.json"));
    let preview_out = schema("preview_out", include_str!("../schemas/preview.output.schema.json"));
    let refine_in = schema("refine_in", include_str!("../schemas/refine.input.schema.json"));
    let refine_out = schema("refine_out", include_str!("../schemas/refine.output.schema.json"));
    let config = schema("configuration", include_str!("../schemas/configuration.schema.json"));
    let resume = schema("resume", r#"{"type":"null"}"#);

    let mk_op = |id: &str, label: &str, i: &Value, o: &Value, controls: Vec<Value>, media: Vec<Value>, outs: Vec<Value>, mode: &str| {
        json!({
            "id": id,
            "version": 1,
            "label": label,
            "inputSchema": {"id": i["id"], "digest": i["digest"]},
            "outputSchema": {"id": o["id"], "digest": o["digest"]},
            "resumeSchema": {"id": "resume", "digest": resume["digest"]},
            "uiSchema": {"controls": controls},
            "mediaSlots": media,
            "outputSlots": outs,
            "execution": {
                "mode": mode,
                "streaming": false,
                "cancellation": "best_effort",
                "resumable": true,
                "remoteIdempotency": false
            }
        })
    };

    let analyze_controls = vec![
        json!({"fieldPointer":"/detection/detector","widget":"text","label":"检测器","description":"ultralytics 相对名","group":"检测"}),
        json!({"fieldPointer":"/detection/confidence","widget":"number","label":"置信度","description":"0-1","group":"检测"}),
        json!({"fieldPointer":"/detection/cutDetection","widget":"select","label":"切镜检测","description":"none|auto","group":"检测"}),
        json!({"fieldPointer":"/detection/selectHint","widget":"select","label":"候选编号提示","description":"仅影响编号，不代替用户选择","group":"检测"}),
    ];
    let preview_controls = vec![
        json!({"fieldPointer":"/preview/includeLostMarkers","widget":"checkbox","label":"丢失帧标记","description":"预览中标红丢失/未解析帧","group":"预览"}),
    ];
    let refine_controls = vec![
        json!({"fieldPointer":"/freeze/prompt","widget":"multiline","label":"提示词","description":"用户原文","group":"精修"}),
        json!({"fieldPointer":"/freeze/params/denoise","widget":"number","label":"去噪","description":"由包声明范围","group":"精修"}),
        json!({"fieldPointer":"/freeze/params/smallFaceStrength","widget":"number","label":"小脸强度","description":"denoise_multiplier_small_face","group":"精修"}),
        json!({"fieldPointer":"/freeze/params/largeFaceStrength","widget":"number","label":"大脸强度","description":"denoise_multiplier_large_face","group":"精修"}),
        json!({"fieldPointer":"/sam/enabled","widget":"checkbox","label":"SAM脸形遮罩","description":"缺依赖须显式不可用，不得静默改矩形","group":"可选"}),
    ];

    json!({
        "schemaVersion": 2,
        "pluginId": PLUGIN,
        "pluginVersion": env!("CARGO_PKG_VERSION"),
        "capabilityContracts": [{
            "schemaVersion": 2,
            "localId": "face-refine",
            "capabilityKey": "video.edit",
            "transportSlot": "videoFaceRefine",
            "modelIds": [MODEL],
            "configurationSchema": {"id": "configuration", "digest": config["digest"]},
            "schemas": [analyze_in.clone(), analyze_out.clone(), preview_in.clone(), preview_out.clone(), refine_in.clone(), refine_out.clone(), config, resume.clone()],
            "operations": [
                mk_op(OP_ANALYZE, "分析镜头与候选脸", &analyze_in, &analyze_out, analyze_controls,
                    vec![json!({"role":"source_video","inputPointer":"/source/mediaId","mediaKind":"video","minCount":1,"maxCount":1})],
                    vec![json!({"role":"analysis","kind":"json","minCount":1,"maxCount":1})],
                    "synchronous"),
                mk_op(OP_PREVIEW, "跟踪预览", &preview_in, &preview_out, preview_controls,
                    vec![json!({"role":"source_video","inputPointer":"/source/mediaId","mediaKind":"video","minCount":1,"maxCount":1})],
                    vec![json!({"role":"preview","kind":"video","minCount":1,"maxCount":1})],
                    "asynchronous"),
                mk_op(OP_REFINE, "人脸精修", &refine_in, &refine_out, refine_controls,
                    vec![
                        json!({"role":"source_video","inputPointer":"/freeze/source/mediaId","mediaKind":"video","minCount":1,"maxCount":1}),
                        json!({"role":"appearance_ref","inputPointer":"/freeze/references","mediaKind":"image","minCount":0,"maxCount":9}),
                    ],
                    vec![json!({"role":"video","kind":"video","minCount":1,"maxCount":1})],
                    "asynchronous")
            ]
        }]
    })
}

fn response_fail(request: &Invocation, f: Failure) -> Response {
    serde_json::from_value(json!({
        "protocolVersion": 2,
        "operation": request.r#operation,
        "error": {
            "code": f.code,
            "message": f.message,
            "retryable": false,
            "vendorCode": null,
            "vendorRequestId": null
        }
    }))
    .expect("closed public error")
}

fn submit_payload(request: &Invocation) -> Result<Value> {
    serde_json::from_str(&request.r#payload_json).map_err(|_| invalid("submit payload is not JSON"))
}

fn run_analyze(h: &mut dyn Host, payload: &Value) -> Result<Value> {
    let configuration = payload.get("configuration").cloned().unwrap_or(json!({}));
    let base = cfg_base(&configuration)?;
    let input = payload.get("input").ok_or_else(|| invalid("missing input"))?;
    let detection = input.get("detectionParams").cloned().or_else(|| input.get("detection").cloned()).unwrap_or(json!({}));
    // Media broker supplies absolute path via retained media; harness may pass videoPath in configuration for CLI.
    let video_path = configuration
        .get("videoPath")
        .and_then(Value::as_str)
        .or_else(|| input.pointer("/source/localPath").and_then(Value::as_str))
        .ok_or_else(|| invalid("analyze requires configuration.videoPath or staged media path for local Comfy scan"))?;

    let body = json!({
        "video": video_path,
        "detector": detection.get("detector").and_then(Value::as_str).unwrap_or("bbox/face_yolov8m.pt"),
        "confidence": detection.get("confidence").and_then(Value::as_f64).unwrap_or(0.35),
        "cut_detection": detection.get("cutDetection").and_then(Value::as_str).unwrap_or("auto"),
        "cut_threshold": detection.get("cutThreshold").and_then(Value::as_f64).unwrap_or(3.0),
        "select": detection.get("selectHint").and_then(Value::as_str).unwrap_or("largest_face"),
        "force": detection.get("force").and_then(Value::as_bool).unwrap_or(false)
    });
    let url = format!("{}/h3_facerefine/scan", base.trim_end_matches('/'));
    let raw = http_json(h, "POST", &url, Some(body))?;
    if let Some(err) = raw.get("error").and_then(Value::as_str) {
        return Err(fail("UPSTREAM_ERROR", err));
    }
    // Map upstream shot cards → media-target-selection analysis receipt (package-local profile).
    let w = raw.get("width").and_then(Value::as_f64).unwrap_or(1.0);
    let hgt = raw.get("height").and_then(Value::as_f64).unwrap_or(1.0);
    let fps = raw.get("fps").and_then(Value::as_f64).unwrap_or(24.0);
    let frames = raw.get("frames").and_then(Value::as_u64).unwrap_or(0);
    let mut shots = Vec::new();
    if let Some(arr) = raw.get("shots").and_then(Value::as_array) {
        for (i, shot) in arr.iter().enumerate() {
            let seg = shot.get("segment").and_then(Value::as_array);
            let a = seg.and_then(|s| s.get(0)).and_then(Value::as_f64).unwrap_or(0.0);
            let b = seg.and_then(|s| s.get(1)).and_then(Value::as_f64).unwrap_or(a);
            let start_ms = round_ms((a / fps) * 1000.0);
            let end_ms = round_ms((b / fps) * 1000.0);
            let mut candidates = Vec::new();
            if let Some(boxes) = shot.get("boxes").and_then(Value::as_array) {
                for (ci, bx) in boxes.iter().enumerate() {
                    let vals = bx.as_array();
                    let x0 = vals.and_then(|v| v.get(0)).and_then(Value::as_f64).unwrap_or(0.0) * w;
                    let y0 = vals.and_then(|v| v.get(1)).and_then(Value::as_f64).unwrap_or(0.0) * hgt;
                    let x1 = vals.and_then(|v| v.get(2)).and_then(Value::as_f64).unwrap_or(0.0) * w;
                    let y1 = vals.and_then(|v| v.get(3)).and_then(Value::as_f64).unwrap_or(0.0) * hgt;
                    candidates.push(json!({
                        "candidateId": format!("c{ci}"),
                        "box": {"x": x0, "y": y0, "width": (x1 - x0), "height": (y1 - y0)},
                        "score": null,
                        "previewMediaId": null
                    }));
                }
            }
            let fi = shot.get("frame").and_then(Value::as_f64).unwrap_or(0.0);
            let representative_frame_ms = round_ms((fi / fps) * 1000.0);
            shots.push(json!({
                "shotId": format!("s{i}"),
                "range": {"startMs": start_ms, "endMs": end_ms},
                "representativeFrameMs": representative_frame_ms,
                "candidates": candidates
            }));
        }
    }
    let digest_src = serde_json::to_string(&shots).unwrap_or_default();
    let digest: String = Sha256::digest(digest_src.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let analysis_id: String = {
        let mut hasher = Sha256::new();
        hasher.update(video_path.as_bytes());
        hasher.update(digest.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            .chars()
            .take(32)
            .collect()
    };
    let end_ms = if frames > 0 {
        round_ms((frames as f64 / fps) * 1000.0)
    } else {
        0
    };
    let receipt = json!({
        "analysisId": analysis_id,
        "revision": "1",
        "digest": digest,
        "coveredRange": {"startMs": 0, "endMs": end_ms},
        "page": {"cursor": null, "nextCursor": null, "complete": true},
        "shots": shots,
        "vendor": {
            "upstreamCommit": UPSTREAM_COMMIT,
            "frames": frames,
            "width": w,
            "height": hgt,
            "fps": fps,
            "cached": raw.get("cached").cloned().unwrap_or(json!(false))
        }
    });
    Ok(json!({
        "kind": "completed",
        "vendorTaskId": null,
        "appliedParams": {"operation": OP_ANALYZE, "upstream": UPSTREAM_COMMIT},
        "outputs": [],
        "data": receipt,
        "usage": null
    }))
}

fn run_preview(h: &mut dyn Host, payload: &Value) -> Result<Value> {
    let _ = h;
    let input = payload.get("input").ok_or_else(|| invalid("missing input"))?;
    let selection = input.get("selection").ok_or_else(|| invalid("preview requires selection"))?;
    let analysis = input.get("analysis").ok_or_else(|| invalid("preview requires analysis"))?;
    // Selection digest freeze
    let sel_raw = serde_json::to_string(selection).unwrap_or_default();
    let selection_digest: String = Sha256::digest(sel_raw.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    // Build face_pick / absent_shots strings for upstream H3FaceSelect / H3FaceTrackCrop
    let mut picks = Vec::new();
    let mut absent = Vec::new();
    if let Some(shots) = selection.get("shots").and_then(Value::as_array) {
        for s in shots {
            let sid = s.get("shotId").and_then(Value::as_str).unwrap_or("");
            let kind = s.pointer("/choice/kind").or_else(|| s.pointer("/decision/kind")).and_then(Value::as_str).unwrap_or("");
            match kind {
                "absent" => absent.push(sid.to_string()),
                "target" => {
                    let cid = s.pointer("/choice/candidateId").or_else(|| s.pointer("/decision/candidateId")).and_then(Value::as_str).unwrap_or("");
                    // candidateId "cN" → index N for upstream confirmed_pick
                    let idx = cid.trim_start_matches('c');
                    picks.push(format!("{sid}:{idx}"));
                }
                _ => return Err(invalid("each shot decision must be target or absent; unset is incomplete")),
            }
        }
    }
    let face_pick = picks.join(",");
    let absent_shots = absent.join(",");
    // Async accept: actual Comfy submit is done by host/harness using workflows + these bindings.
    Ok(json!({
        "kind": "accepted",
        "vendorTaskId": format!("preview-{selection_digest:.16}"),
        "appliedParams": {
            "operation": OP_PREVIEW,
            "face_pick": face_pick,
            "absent_shots": absent_shots,
            "analysisId": analysis.get("analysisId"),
            "analysisRevision": analysis.get("revision"),
            "selectionDigest": selection_digest,
            "workflow": "workflows/preview.track.api.json"
        },
        "outputs": [],
        "data": {
            "selectionDigest": selection_digest,
            "previewMediaId": null,
            "coveredRange": {"startMs": 0, "endMs": 0},
            "lostSpans": [],
            "ready": {"ok": false, "reason": "awaiting Comfy preview render; use harness ops/preview.py to complete"},
            "reportText": null,
            "bindings": {"face_pick": face_pick, "absent_shots": absent_shots}
        },
        "usage": null
    }))
}

fn run_refine(h: &mut dyn Host, payload: &Value) -> Result<Value> {
    let _ = h;
    let input = payload.get("input").ok_or_else(|| invalid("missing input"))?;
    let freeze = input.get("freeze").ok_or_else(|| invalid("refine requires freeze"))?;
    let prompt = freeze.get("prompt").and_then(Value::as_str).unwrap_or("");
    let sam_enabled = input.pointer("/sam/enabled").and_then(Value::as_bool).unwrap_or(false);
    let freeze_raw = serde_json::to_string(freeze).unwrap_or_default();
    let freeze_digest: String = Sha256::digest(freeze_raw.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(json!({
        "kind": "accepted",
        "vendorTaskId": format!("refine-{freeze_digest:.16}"),
        "appliedParams": {
            "operation": OP_REFINE,
            "upstreamCommit": UPSTREAM_COMMIT,
            "prompt": prompt,
            "samEnabled": sam_enabled,
            "workflow": "workflows/refine.manual.api.json",
            "freezeDigest": freeze_digest
        },
        "outputs": [],
        "data": {
            "outputMediaId": null,
            "execution": {"state": "accepted", "note": "use harness ops/refine.py against managed Comfy"},
            "audioPreserved": true,
            "freezeDigest": freeze_digest
        },
        "usage": null
    }))
}

pub struct H3FaceRefine;

impl Adapter for H3FaceRefine {
    fn invoke(request: Invocation, host: &mut dyn Host) -> Response {
        let operation = serde_json::to_value(&request.r#operation).expect("closed operation");
        let result = match operation.as_str().expect("operation string") {
            "capabilities" => capabilities(),
            "probe" => {
                // Soft probe: packageValid always; connection checked only if endpoint configured via empty payload.
                json!({
                    "packageValid": true,
                    "connection": "not_checked",
                    "authentication": "not_checked",
                    "resources": "not_checked",
                    "issues": [],
                    "notes": [
                        "Isolation: companion uses extra_model_paths; never overwrites global custom_nodes",
                        "Upstream locked to 1.1.2 d8521d14fe0d721d80cd9417fff5a559cbc21aba"
                    ]
                })
            }
            "submit" => {
                let payload = match submit_payload(&request) {
                    Ok(v) => v,
                    Err(f) => return response_fail(&request, f),
                };
                let op_id = payload
                    .get("operationId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let run = match op_id {
                    OP_ANALYZE => run_analyze(host, &payload),
                    OP_PREVIEW => run_preview(host, &payload),
                    OP_REFINE => run_refine(host, &payload),
                    _ => Err(fail(
                        "UNSUPPORTED_FEATURE",
                        format!("unknown operationId: {op_id}"),
                    )),
                };
                match run {
                    Ok(v) => v,
                    Err(f) => return response_fail(&request, f),
                }
            }
            "query" | "cancel" => {
                return response_fail(
                    &request,
                    fail(
                        "UNSUPPORTED_FEATURE",
                        "query/cancel for preview/refine are completed by harness/host against Comfy prompt ids; wire in A06+",
                    ),
                );
            }
            _ => {
                return response_fail(
                    &request,
                    fail("UNSUPPORTED_FEATURE", "operation not implemented by this package"),
                );
            }
        };
        guest::completed_response(&request, &result).expect("serializable public result")
    }
}

bgx_adapter_v2_sdk::export_adapter!(H3FaceRefine);
