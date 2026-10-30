#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{borrow::ToOwned, format, string::{String, ToString}, vec, vec::Vec};
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
    retryable: bool,
    service_rejected: bool,
}

fn fail(code: &'static str, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
        retryable: false,
        service_rejected: false,
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

mod response_diagnostic;

fn http_body(h: &mut dyn Host, method: &str, url: &str, body: Value) -> Result<Value> {
    let headers = if body["kind"] == "bytes" { json!([{"name":"Content-Type","value":"application/json"}]) } else { json!([]) };
    let response = port(h, "host-http.execute", json!({"plan":{"method":method,"url":url,"headers":headers,"generatedHeaders":[],"body":body,"sink":{"kind":"buffer","maxBytes":1048576},"timeoutMs":600000},"credential":null}))?;
    let status = response["status"].as_u64().ok_or_else(|| fail("UPSTREAM_ERROR", "HTTP response missing status"))?;

    let bytes: Vec<u8> = serde_json::from_value(response["payload"]["bytes"].clone()).map_err(|_| fail("UPSTREAM_ERROR", "HTTP response missing buffered bytes"))?;
    let raw: Value = serde_json::from_slice(&bytes).map_err(|_| fail("UPSTREAM_ERROR", format!("Comfy HTTP {status}: response is not JSON; {}", response_diagnostic::describe(&bytes))))?;
    if !(200..300).contains(&status) {
        let kind = raw.pointer("/error/type").and_then(Value::as_str).unwrap_or("unclassified");
        let rejected = status == 400 && matches!(kind, "prompt_outputs_failed_validation" | "prompt_no_outputs" | "invalid_prompt");
        let mut failure = fail(if rejected { "INVALID_INPUT" } else { "UPSTREAM_ERROR" }, format!("Comfy HTTP {status}: {kind}; {}", response_diagnostic::describe(&bytes)));
        failure.service_rejected = rejected;
        return Err(failure);
    }
    Ok(raw)
}
fn http_json(h: &mut dyn Host, method: &str, url: &str, body: Option<Value>) -> Result<Value> {
    let body = match body {
        Some(v) => json!({"kind":"bytes","bytes":serde_json::to_vec(&v).map_err(|_| invalid("invalid JSON body"))?}),
        None => json!({"kind":"none"}),
    };
    http_body(h, method, url, body)
}
fn upload_source(h: &mut dyn Host, payload: &Value, base: &str, id: &str, kind: &str) -> Result<String> {
    let media = payload["media"].as_array().ok_or_else(|| invalid("missing managed media"))?;
    let matching: Vec<_> = media.iter().filter(|m| m["id"].as_str() == Some(id)).collect();
    if matching.len() != 1 { return Err(invalid("source media identity is missing or ambiguous")); }
    let m = matching[0];
    if m["kind"] != kind { return Err(invalid("managed media kind does not match slot")); }
    let stat = port(h, "host-media.info", json!({"handle":m["handle"]}))?;
    for key in ["kind", "mime", "fileName", "byteLength"] {
        if stat[key] != m[key] { return Err(invalid("managed media differs from frozen descriptor")); }
    }
    let uploaded = http_body(h, "POST", &format!("{}/upload/image", base.trim_end_matches('/')), json!({"kind":"multipart","parts":[{"kind":"file","name":"image","fileName":m["fileName"],"handle":m["handle"],"mime":m["mime"]}]}))?;
    let name = uploaded["name"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| fail("UPSTREAM_ERROR", "upload response missing name"))?;
    let subfolder = uploaded["subfolder"].as_str().unwrap_or("");
    Ok(if subfolder.is_empty() { name.to_owned() } else { format!("{subfolder}/{name}") })
}

pub fn capabilities() -> Value {
    let analyze_in = schema("analyze_in", include_str!("../schemas/analyze.input.schema.json"));
    let analyze_out = schema("analyze_out", include_str!("../schemas/analyze.output.schema.json"));
    let preview_in = schema("preview_in", include_str!("../schemas/preview.input.schema.json"));
    let preview_out = schema("preview_out", include_str!("../schemas/preview.output.schema.json"));
    let refine_in = schema("refine_in", include_str!("../schemas/refine.input.schema.json"));
    let refine_out = schema("refine_out", include_str!("../schemas/refine.output.schema.json"));
    let config = schema("configuration", include_str!("../schemas/configuration.schema.json"));
    let resume = schema("resume", include_str!("../schemas/resume.schema.json"));

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
        // Analyze input schema uses detectionParams (not /detection).
        // ui-schema.schema.json allows only fieldPointer/widget/label/description/group.
        json!({"fieldPointer":"/detectionParams/detector","widget":"text","label":"检测器","description":"ultralytics 相对名；默认 bbox/face_yolov8m.pt","group":"检测"}),
        json!({"fieldPointer":"/detectionParams/confidence","widget":"number","label":"置信度","description":"0-1；默认 0.35","group":"检测"}),
        json!({"fieldPointer":"/detectionParams/cutDetection","widget":"select","label":"切镜检测","description":"none|auto；默认 auto","group":"检测"}),
        json!({"fieldPointer":"/detectionParams/selectHint","widget":"select","label":"候选编号提示","description":"仅影响编号，不代替用户选择；默认 largest_face","group":"检测"}),
    ];
    let preview_controls = vec![
        json!({"fieldPointer":"/preview/includeLostMarkers","widget":"checkbox","label":"丢失帧标记","description":"预览中标红丢失/未解析帧","group":"预览"}),
    ];
    let refine_controls = vec![
        json!({"fieldPointer":"/freeze/referenceMediaIds","widget":"media","label":"外观参考图","description":"提供希望采用的外观参考","group":"素材"}),
        json!({"fieldPointer":"/freeze/userParams/identityMediaIds","widget":"media","label":"身份参考图","description":"用于确认人物身份","group":"素材"}),
        // Pointers must resolve into public freeze fields (prompt / userParams / referenceMediaIds).
        // ui-schema.schema.json allows only fieldPointer/widget/label/description/group.
        json!({"fieldPointer":"/freeze/prompt","widget":"multiline","label":"提示词","description":"用户原文","group":"精修"}),
        json!({"fieldPointer":"/freeze/userParams/denoise","widget":"number","label":"去噪","description":"0-1；旧对照约0.45","group":"精修"}),
        json!({"fieldPointer":"/freeze/userParams/smallFaceStrength","widget":"number","label":"小脸强度","description":"小脸使用的精修强度比例，数值越大增强越明显","group":"精修"}),
        json!({"fieldPointer":"/freeze/userParams/largeFaceStrength","widget":"number","label":"大脸强度","description":"大脸使用的精修强度比例，数值越小越保留细节","group":"精修"}),
        json!({"fieldPointer":"/freeze/userParams/samModel","widget":"text","label":"SAM模型文件名","description":"启用脸形遮罩时填写已安装的模型名称","group":"可选"}),
        json!({"fieldPointer":"/freeze/userParams/samEnabled","widget":"checkbox","label":"SAM脸形遮罩","description":"缺依赖须显式不可用，不得静默改矩形","group":"可选"}),
        json!({"fieldPointer":"/freeze/userParams/scaleMode","widget":"select","label":"脸尺寸缩放模式","description":"选择按实际像素或在本视频中的相对大小调节增强","group":"可选"}),
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
                    vec![],
                    "synchronous"),
                mk_op(OP_PREVIEW, "跟踪预览", &preview_in, &preview_out, preview_controls,
                    vec![json!({"role":"source_video","inputPointer":"/analysis/source/mediaId","mediaKind":"video","minCount":1,"maxCount":1})],
                    vec![json!({"role":"preview","kind":"image","minCount":1,"maxCount":1})],
                    "asynchronous"),
                mk_op(OP_REFINE, "人脸精修", &refine_in, &refine_out, refine_controls,
                    vec![
                        json!({"role":"source_video","inputPointer":"/freeze/source/mediaId","mediaKind":"video","minCount":1,"maxCount":1}),
                        json!({"role":"appearance_ref","inputPointer":"/freeze/referenceMediaIds","mediaKind":"image","minCount":0,"maxCount":9}),
                        json!({"role":"identity_ref","inputPointer":"/freeze/userParams/identityMediaIds","mediaKind":"image","minCount":0,"maxCount":1}),
                    ],
                    vec![json!({"role":"video","kind":"video","minCount":1,"maxCount":1})],
                    "asynchronous")
            ]
        }]
    })
}

fn response_fail(request: &Invocation, f: Failure) -> Response {
    use bgx_adapter_v2_sdk::{InvocationOperation as I, ResponseBranch1Operation as O, ResponseBranch1ErrorCode as E, ResponseBranch1, ResponseBranch1Error, ResponseBranch1ProtocolVersion};
    let operation = match request.operation { I::V0=>O::V0,I::V1=>O::V1,I::V2=>O::V2,I::V3=>O::V3,I::V4=>O::V4,I::V5=>O::V5 };
    let code = match f.code {
        "INVALID_PACKAGE"=>E::V0,"UNSUPPORTED_VERSION"=>E::V1,"UNSUPPORTED_FEATURE"=>E::V2,"INVALID_INPUT"=>E::V3,"UNAUTHORIZED"=>E::V4,"PERMISSION_DENIED"=>E::V5,"UPSTREAM_ERROR"=>E::V6,"RATE_LIMITED"=>E::V7,"TIMEOUT"=>E::V8,"ACCEPTANCE_UNKNOWN"=>E::V9,"RESOURCE_UNAVAILABLE"|"BUDGET_EXCEEDED"=>E::V10,"CANCEL_UNSUPPORTED"=>E::V11,_=>E::V12,
    };
    Response::V1(ResponseBranch1 {protocol_version:ResponseBranch1ProtocolVersion,operation,error:ResponseBranch1Error {
        code,message:format!("{}: {}",f.code,f.message),retryable:f.retryable,vendor_code:None,vendor_request_id:None,
    }})
}

fn submit_payload(request: &Invocation) -> Result<Value> {
    serde_json::from_str(&request.r#payload_json).map_err(|_| invalid("submit payload is not JSON"))
}

fn run_analyze(h: &mut dyn Host, payload: &Value, phase: &mut Phase) -> Result<Value> {
    let configuration = payload.get("configuration").cloned().unwrap_or(json!({}));
    let base = cfg_base(&configuration)?;
    let input = payload.get("input").ok_or_else(|| invalid("missing input"))?;
    let detection = input.get("detectionParams").cloned().or_else(|| input.get("detection").cloned()).unwrap_or(json!({}));
    let source = input.get("source").ok_or_else(|| invalid("missing source identity"))?;
    let source_id = source["mediaId"].as_str().ok_or_else(|| invalid("missing source mediaId"))?;
    let video_path = upload_source(h, payload, base, source_id, "video")?;

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
    *phase = Phase::Unknown;
    let raw = http_json(h, "POST", &url, Some(body.clone()))?;
    if let Some(err) = raw.get("error").and_then(Value::as_str) {
        return Err(fail("UPSTREAM_ERROR", err));
    }
    // Map upstream shot cards → media-target-selection analysis receipt (package-local profile).
    let positive = |key: &str| raw[key].as_f64().filter(|n| n.is_finite() && *n > 0.0).ok_or_else(|| fail("UPSTREAM_ERROR", format!("scan missing valid {key}")));
    let w = positive("width")?;
    let hgt = positive("height")?;
    let fps = positive("fps")?;
    let frames = raw["frames"].as_u64().filter(|n| *n > 0).ok_or_else(|| fail("UPSTREAM_ERROR", "scan missing frames"))?;
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
                let mut ordered: Vec<_> = boxes.iter().collect();
                ordered.sort_by(|a,b| {
                    let center = |v: &Value| (v[0].as_f64().unwrap_or(0.0) + v[2].as_f64().unwrap_or(0.0)) / 2.0;
                    center(a).total_cmp(&center(b))
                });
                for (ci, bx) in ordered.into_iter().enumerate() {
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
            let representative_frame_ms = if fi < 0.0 { start_ms } else { round_ms((fi / fps) * 1000.0) };
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
        hasher.update(source.to_string().as_bytes());
        hasher.update(detection.to_string().as_bytes());
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
        "source": source,
        "coveredRange": {"startMs": 0, "endMs": end_ms},
        "page": {"cursor": null, "nextCursor": null, "complete": true},
        "shots": shots
    });
    let mut session_detection = body;
    session_detection.as_object_mut().unwrap().remove("video");
    http_json(h,"POST",&format!("{}/bgx_face_refine/analysis-session",base.trim_end_matches('/')),Some(json!({"analysis":receipt,"detection":session_detection})))?;
    Ok(json!({
        "kind": "completed",
        "vendorTaskId": null,
        "appliedParams": null,
        "outputs": [],
        "data": receipt,
        "usage": null
    }))
}


fn analysis_state(h: &mut dyn Host, base: &str, id: &str, revision: &str, digest: &str) -> Result<Value> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(invalid("invalid analysis session id")); }
    let saved = http_json(h,"GET",&format!("{}/bgx_face_refine/analysis-session/{id}",base.trim_end_matches('/')),None)?;
    if saved["analysis"]["revision"] != revision || saved["analysis"]["digest"] != digest {
        return Err(invalid("analysis session identity changed"));
    }
    Ok(saved)
}
fn apply_detection(wf: &mut Value, select_node: &str, track_node: &str, saved: &Value) {
    for key in ["detector","confidence","cut_detection","cut_threshold"] {
        wf[select_node]["inputs"][key] = saved["detection"][key].clone();
        wf[track_node]["inputs"][key] = saved["detection"][key].clone();
    }
}

fn selection_bindings(selection: &Value, analysis: &Value) -> Result<(String, String)> {
    // Upstream H3FaceSelect manual: confirmed_pick = comma-separated L→R face indices
    // per shot; -1 marks absent. Not shot:idx.
    let mut picks = Vec::new();
    let mut absent = Vec::new();
    if let Some(shots) = selection.get("shots").and_then(Value::as_array) {
        let analyzed = analysis["shots"].as_array().ok_or_else(|| invalid("analysis shots required"))?;
        if shots.len() != analyzed.len() { return Err(invalid("selection must cover every analyzed shot")); }
        for shot in analyzed {
            let selected: Vec<_> = shots.iter().filter(|s| s["shotId"] == shot["shotId"]).collect();
            if selected.len() != 1 { return Err(invalid("selection shot identity is missing or duplicated")); }
            let s = selected[0];
            let sid = s.get("shotId").and_then(Value::as_str).unwrap_or("");
            let kind = s
                .pointer("/choice/kind")
                .or_else(|| s.pointer("/decision/kind"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match kind {
                "absent" => {
                    picks.push("-1".to_string());
                    absent.push(sid.to_string());
                }
                "target" => {
                    let cid = s
                        .pointer("/choice/candidateId")
                        .or_else(|| s.pointer("/decision/candidateId"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !shot["candidates"].as_array().is_some_and(|cs| cs.iter().any(|c| c["candidateId"] == cid)) { return Err(invalid("selection candidate is not in analysis")); }
                    let idx = cid.strip_prefix('c').ok_or_else(|| invalid("candidate identity invalid"))?;
                    let n: i64 = idx.parse().map_err(|_| invalid("candidateId must be cN"))?;
                    picks.push(n.to_string());
                }
                _ => {
                    return Err(invalid(
                        "each shot decision must be target or absent (unset is incomplete)",
                    ));
                }
            }
        }
    }
    Ok((picks.join(","), absent.join(",")))
}


fn load_workflow_template(name: &str) -> Result<Value> {
    let raw = match name {
        "preview" => include_str!("../workflows/preview.track.api.json"),
        "refine" => include_str!("../workflows/refine.manual.api.json"),
        _ => return Err(invalid("unknown workflow template")),
    };
    serde_json::from_str(raw).map_err(|e| invalid(&format!("workflow JSON invalid: {e}")))
}

fn inject_preview_workflow(face_pick: &str, video_name: &str, analysis: &Value, selection: &Value) -> Result<Value> {
    let mut wf = load_workflow_template("preview")?;
    if let Some(obj) = wf.as_object_mut() {
        obj.remove("_meta");
        if let Some(n1) = obj.get_mut("1").and_then(|v| v.as_object_mut()) {
            if let Some(inputs) = n1.get_mut("inputs").and_then(|v| v.as_object_mut()) {
                inputs.insert("video".into(), json!(video_name));
                inputs.insert("confirmed_pick".into(), json!(face_pick));
                inputs.insert("select".into(), json!("manual"));
                inputs.insert("select_index".into(), json!(0));
                inputs.insert("detector".into(), json!("bbox\\face_yolov8m.pt"));
                inputs.insert("cut_detection".into(), json!("none"));
            }
        }
        if let Some(n2) = obj.get_mut("2").and_then(|v| v.as_object_mut()) {
            if let Some(inputs) = n2.get_mut("inputs").and_then(|v| v.as_object_mut()) {
                inputs.insert("detector".into(), json!("bbox\\face_yolov8m.pt"));
                inputs.insert("cut_detection".into(), json!("none"));
                if !inputs.contains_key("crop_factor") { inputs.insert("crop_factor".into(), json!(2.5)); }
                if !inputs.contains_key("canvas_width") { inputs.insert("canvas_width".into(), json!(768)); }
                if !inputs.contains_key("canvas_height") { inputs.insert("canvas_height".into(), json!(768)); }
                if !inputs.contains_key("canvas_mode") { inputs.insert("canvas_mode".into(), json!("manual")); }
                if !inputs.contains_key("smooth_window") { inputs.insert("smooth_window".into(), json!(21)); }
                if !inputs.contains_key("size_smooth_window") { inputs.insert("size_smooth_window".into(), json!(51)); }
                if !inputs.contains_key("smooth_method") { inputs.insert("smooth_method".into(), json!("gaussian")); }
                if !inputs.contains_key("size_mode") { inputs.insert("size_mode".into(), json!("per_frame")); }
                if !inputs.contains_key("preview") { inputs.insert("preview".into(), json!(true)); }
            }
        }
    }
    wf["3"]["inputs"]["fps"] = json!(["1",6]);
    wf["tracking_receipt"] = json!({"class_type":"H3FaceTrackingReceipt","inputs":{"transform":["2",1],"fps":["1",6],"analysis_json":analysis.to_string(),"selection_json":selection.to_string()}});
    Ok(wf)
}

fn inject_refine_workflow(
    face_pick: &str,
    video_name: &str,
    prompt: &str,
    user_params: &Value,
) -> Result<Value> {
    let mut wf = load_workflow_template("refine")?;
    if let Some(obj) = wf.as_object_mut() {
        obj.remove("_meta");
    }
    let denoise = user_params
        .get("denoise")
        .and_then(Value::as_f64)
        .unwrap_or(0.45);
    let small = user_params
        .get("smallFaceStrength")
        .and_then(Value::as_f64)
        .unwrap_or(0.8);
    let large = user_params
        .get("largeFaceStrength")
        .and_then(Value::as_f64)
        .unwrap_or(0.35);
    let scale_mode = user_params
        .get("scaleMode")
        .and_then(Value::as_str)
        .unwrap_or("absolute_px");
    let appearances = user_params.get("appearanceImages").and_then(Value::as_array).ok_or_else(|| invalid("uploaded appearanceImages required"))?;
    let identity = user_params
        .get("identityImage")
        .and_then(Value::as_str);
    if let Some(obj) = wf.as_object_mut() {
        if let Some(sel) = obj.get_mut("select").and_then(|v| v.as_object_mut()) {
            if let Some(inputs) = sel.get_mut("inputs").and_then(|v| v.as_object_mut()) {
                inputs.insert("video".into(), json!(video_name));
                inputs.insert("confirmed_pick".into(), json!(face_pick));
            }
        }
        if let Some(den) = obj.get_mut("denoise").and_then(|v| v.as_object_mut()) {
            if let Some(inputs) = den.get_mut("inputs").and_then(|v| v.as_object_mut()) {
                inputs.insert("denoise_multiplier_small_face".into(), json!(small));
                inputs.insert("denoise_multiplier_large_face".into(), json!(large));
                inputs.insert("scale_mode".into(), json!(scale_mode));
            }
        }
        if let Some(r2v) = obj.get_mut("r2v").and_then(|v| v.as_object_mut()) {
            if let Some(inputs) = r2v.get_mut("inputs").and_then(|v| v.as_object_mut()) {
                inputs.insert("prompt".into(), json!(prompt));
            }
        }
        if let Some(sched) = obj.get_mut("scheduler").and_then(|v| v.as_object_mut()) {
            if let Some(inputs) = sched.get_mut("inputs").and_then(|v| v.as_object_mut()) {
                inputs.insert("denoise".into(), json!(denoise));
            }
        }
        obj.remove("ref0");
        obj.remove("ref1");
        if let Some(inputs) = obj.get_mut("r2v").and_then(|n| n["inputs"].as_object_mut()) {
            inputs.remove("ref_images.ref_image_0"); inputs.remove("ref_images.ref_image_1");
            for (index, _) in appearances.iter().enumerate() {
                inputs.insert(format!("ref_images.ref_image_{index}"), json!([format!("appearance_{index}"),0]));
            }
        }
        for (index, name) in appearances.iter().enumerate() {
            obj.insert(format!("appearance_{index}"), json!({"class_type":"LoadImage","inputs":{"image":name}}));
        }
        if let Some(name) = identity {
            obj.insert("identity_ref".into(), json!({"class_type":"LoadImage","inputs":{"image":name}}));
            obj.get_mut("track").unwrap()["inputs"]["identity_reference"] = json!(["identity_ref",0]);
        }
        obj.get_mut("create").unwrap()["inputs"]["fps"] = json!(["select",6]);
        if user_params.get("samEnabled").and_then(Value::as_bool).unwrap_or(false) {
            let model = user_params.get("samModel").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| invalid("SAM enabled: select an installed samModel"))?;
            obj.insert("sam_loader".into(),json!({"class_type":"SAMLoader","inputs":{"model_name":model,"device_mode":"AUTO"}}));
            obj.insert("sam_mask".into(),json!({"class_type":"H3FaceMaskSAM","inputs":{"crops":["track",0],"sam_model":["sam_loader",0],"transform":["track",1],"threshold":0.93,"dilation":0,"temporal_smooth":5}}));
            obj.get_mut("stitch").unwrap()["inputs"]["masks"] = json!(["sam_mask",0]);
        }
    }
    Ok(wf)
}


mod platform_parameters;
mod submission_rejection;
use submission_rejection::Phase;
fn comfy_submit_prompt(h: &mut dyn Host, base: &str, mut workflow: Value, client_id: &str, phase: &mut Phase) -> Result<String> {
    let info = platform_parameters::definitions(h, base, &workflow)?;
    platform_parameters::prepare(&mut workflow, &info)?;
    let url = format!("{}/prompt", base.trim_end_matches('/'));
    let body = json!({
        "prompt": workflow,
        "client_id": client_id
    });
    *phase = Phase::Unknown;
    let raw = http_json(h, "POST", &url, Some(body)).map_err(|error| {
        if error.service_rejected { *phase = Phase::ServiceRejected; }
        if matches!(error.code, "PERMISSION_DENIED" | "INVALID_INPUT" | "BUDGET_EXCEEDED") { error }
        else { fail("ACCEPTANCE_UNKNOWN", format!("Comfy submission receipt unavailable: {}; do not resubmit", error.message)) }
    })?;
    if let Some(err) = raw.get("error").cloned() {
        return Err(fail(
            "UPSTREAM_ERROR",
            format!("Comfy /prompt rejected: {err}"),
        ));
    }
    raw.get("prompt_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| fail("ACCEPTANCE_UNKNOWN", "Comfy submission receipt missing prompt_id; do not resubmit"))
}

fn comfy_query_prompt(h: &mut dyn Host, base: &str, prompt_id: &str) -> Result<Value> {
    let url = format!(
        "{}/history/{}",
        base.trim_end_matches('/'),
        prompt_id
    );
    let raw = http_json(h, "GET", &url, None)?;
    Ok(raw)
}

fn comfy_cancel(h: &mut dyn Host, base: &str, prompt_id: &str) -> Result<Value> {
    let queue = http_json(h, "GET", &format!("{}/queue", base.trim_end_matches('/')), None)?;
    let pending = queue["queue_pending"].as_array().ok_or_else(|| fail("UPSTREAM_ERROR", "invalid queue response"))?;
    if pending.iter().any(|row| row[1].as_str() == Some(prompt_id)) {
        http_json(h, "POST", &format!("{}/queue", base.trim_end_matches('/')), Some(json!({"delete":[prompt_id]})))?;
        return Ok(json!({"outcome":"remote_requested"}));
    }
    // A global interrupt cannot guarantee cancellation of this prompt only.
    Ok(json!({"outcome":"unsupported"}))
}

mod task_results;

fn run_preview(h: &mut dyn Host, payload: &Value, phase: &mut Phase) -> Result<Value> {
    let configuration = payload.get("configuration").cloned().unwrap_or(json!({}));
    let base = cfg_base(&configuration)?;
    let input = payload.get("input").ok_or_else(|| invalid("missing input"))?;
    let selection = input.get("selection").ok_or_else(|| invalid("preview requires selection"))?;
    let analysis = input.get("analysis").ok_or_else(|| invalid("preview requires analysis"))?;
    let selection_digest = selection["selectionDigest"].as_str().filter(|v| !v.is_empty()).ok_or_else(|| invalid("selectionDigest required"))?;
    let saved = analysis_state(h, base, analysis["analysisId"].as_str().ok_or_else(|| invalid("analysisId required"))?, analysis["revision"].as_str().unwrap_or(""), analysis["digest"].as_str().unwrap_or(""))?;
    if saved["analysis"] != *analysis { return Err(invalid("analysis receipt differs from retained result")); }
    let (face_pick, _absent_shots) = selection_bindings(selection, analysis)?;
    let source = &analysis["source"];
    let source_id = source["mediaId"].as_str().ok_or_else(|| invalid("analysis source.mediaId required"))?;
    let video_name = upload_source(h, payload, base, source_id, "video")?;
    let mut workflow = inject_preview_workflow(&face_pick, &video_name, analysis, selection)?;
    apply_detection(&mut workflow, "1", "2", &saved);
    let client_id = format!("h3-preview-{:.12}", selection_digest);
    let prompt_id = comfy_submit_prompt(h, base, workflow, &client_id, phase)?;
    Ok(json!({
        "kind": "accepted", "vendorTaskId": prompt_id,
        "appliedParams": null,
        "resumeToken": {"operationId":OP_PREVIEW,"promptId":prompt_id,"selectionDigest":selection_digest,"coveredRange":analysis["coveredRange"],"source":source},
        "usage": null
    }))
}

fn run_refine(h: &mut dyn Host, payload: &Value, phase: &mut Phase) -> Result<Value> {
    let configuration = payload.get("configuration").cloned().unwrap_or(json!({}));
    let base = cfg_base(&configuration)?;
    let input = payload.get("input").ok_or_else(|| invalid("missing input"))?;
    let freeze = input.get("freeze").ok_or_else(|| invalid("refine requires freeze"))?;
    let selection = input.get("selection");
    let prompt = freeze.get("prompt").and_then(Value::as_str).unwrap_or("");
    let mut user_params = freeze.get("userParams").cloned().unwrap_or(json!({}));
    let sam_enabled = user_params
        .get("samEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if sam_enabled {
        // Explicit dependency check — never silently fall back to rectangle masks.
        for class in ["SAMLoader", "H3FaceMaskSAM"] {
            let info = http_json(h, "GET", &format!("{}/object_info/{class}", base.trim_end_matches('/')), None)?;
            if info.get(class).is_none() { return Err(fail("RESOURCE_UNAVAILABLE", format!("SAM enabled but {class} is unavailable"))); }
        }
    }
    let selection = selection.ok_or_else(|| invalid("refine requires selection"))?;
    let saved = analysis_state(h, base, selection["analysisId"].as_str().ok_or_else(|| invalid("analysisId required"))?, selection["analysisRevision"].as_str().unwrap_or(""), selection["analysisDigest"].as_str().unwrap_or(""))?;
    if saved["analysis"]["source"] != freeze["source"] || selection["selectionDigest"] != freeze["selectionDigest"] { return Err(invalid("refine source or selection differs from frozen analysis")); }
    let (face_pick, _absent_shots) = selection_bindings(selection, &saved["analysis"])?;
    let source_id = freeze.pointer("/source/mediaId").and_then(Value::as_str).ok_or_else(|| invalid("freeze source.mediaId required"))?;
    let video_name = upload_source(h, payload, base, source_id, "video")?;
    let references = freeze["referenceMediaIds"].as_array().ok_or_else(|| invalid("referenceMediaIds required"))?;
    if references.len() > 9 { return Err(invalid("at most nine appearance references are supported")); }
    let mut images = Vec::new();
    for id in references {
        images.push(upload_source(h, payload, base, id.as_str().ok_or_else(|| invalid("reference mediaId must be string"))?, "image")?);
    }
    let identities = match user_params.get("identityMediaIds") {
        None => Vec::new(),
        Some(v) => v.as_array().ok_or_else(|| invalid("identityMediaIds must be array"))?.clone(),
    };
    if identities.len() > 1 { return Err(invalid("at most one identity reference is supported")); }
    let identity = identities.first().map(|id| id.as_str().ok_or_else(|| invalid("identity mediaId must be string"))).transpose()?;
    let identity_name = identity.map(|id| upload_source(h, payload, base, id, "image")).transpose()?;
    user_params["appearanceImages"] = json!(images);
    user_params["identityImage"] = json!(identity_name);
    let mut workflow = inject_refine_workflow(&face_pick, &video_name, prompt, &user_params)?;
    apply_detection(&mut workflow, "select", "track", &saved);
    let freeze_raw = serde_json::to_string(freeze).unwrap_or_default();
    let freeze_digest: String = Sha256::digest(freeze_raw.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let client_id = format!("h3-refine-{:.12}", freeze_digest);
    let prompt_id = comfy_submit_prompt(h, base, workflow, &client_id, phase)?;
    Ok(json!({
        "kind":"accepted", "vendorTaskId":prompt_id,
        "appliedParams": null,
        "resumeToken":{"operationId":OP_REFINE,"promptId":prompt_id,"freezeDigest":freeze_digest,"selectionDigest":freeze["selectionDigest"],"source":freeze["source"],"coveredRange":freeze["source"]["selectedRange"]},
        "usage":null
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
                    "issues": []
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
                let mut phase = Phase::BeforeSubmission;
                let run = match op_id {
                    OP_ANALYZE => run_analyze(host, &payload, &mut phase),
                    OP_PREVIEW => run_preview(host, &payload, &mut phase),
                    OP_REFINE => run_refine(host, &payload, &mut phase),
                    _ => Err(fail(
                        "UNSUPPORTED_FEATURE",
                        format!("unknown operationId: {op_id}"),
                    )),
                };
                match run {
                    Ok(v) => v,
                    Err(f) => {
                        if let Err(persist) = submission_rejection::record(host, &request.request_id, phase, &f) {
                            return response_fail(&request, persist);
                        }
                        return response_fail(&request, f);
                    }
                }
            }
            "query" => {
                let payload = match submit_payload(&request) { Ok(v) => v, Err(f) => return response_fail(&request, f) };
                match task_results::query(host, &payload) { Ok(v) => v, Err(f) => return response_fail(&request, f) }
            }
            "cancel" => {
                let payload = match submit_payload(&request) {
                    Ok(v) => v,
                    Err(f) => return response_fail(&request, f),
                };
                let configuration = payload.get("configuration").cloned().unwrap_or(json!({}));
                let base = match cfg_base(&configuration) {
                    Ok(b) => b.to_owned(),
                    Err(f) => return response_fail(&request, f),
                };
                let vendor_task_id = payload
                    .get("vendorTaskId")
                    .or_else(|| payload.get("taskId"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if vendor_task_id.is_empty() {
                    return response_fail(&request, invalid("cancel requires vendorTaskId"));
                }
                match comfy_cancel(host, &base, vendor_task_id) {
                    Ok(v) => v,
                    Err(f) => return response_fail(&request, f),
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    struct TestHost { calls: usize }
    impl Host for TestHost {
        fn call(&mut self, call: bgx_adapter_v2_sdk::HostPortCallV2) -> core::result::Result<Value, host::HostError> {
            let call = serde_json::to_value(call).unwrap();
            self.calls += 1;
            if self.calls == 1 {
                assert_eq!(call["method"], "host-media.info");
                return Ok(json!({"kind":"video","mime":"video/mp4","fileName":"clip.mp4","byteLength":"123"}));
            }
            if self.calls == 4 {
                assert_eq!(call["method"], "host-http.execute");
                assert!(call["args"]["plan"]["url"].as_str().unwrap().ends_with("/bgx_face_refine/analysis-session"));
                return Ok(json!({"status":200,"payload":{"kind":"buffered","bytes":serde_json::to_vec(&json!({"saved":true})).unwrap()}}));
            }
            assert_eq!(call["method"], "host-http.execute");
            let plan = &call["args"]["plan"];
            let value = if self.calls == 2 {
                assert_eq!(plan["body"]["parts"][0]["handle"], "managed-clip");
                json!({"name":"uploaded.mp4","subfolder":""})
            } else {
                let bytes: Vec<u8> = serde_json::from_value(plan["body"]["bytes"].clone()).unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(body["video"], "uploaded.mp4");
                json!({"width":640,"height":360,"fps":25,"frames":25,"shots":[{"segment":[0,25],"frame":0,"boxes":[[0.1,0.2,0.3,0.4]]}]})
            };
            Ok(json!({"status":200,"payload":{"kind":"buffered","bytes":serde_json::to_vec(&value).unwrap()}}))
        }
    }
    #[test]
    fn host_budget_failure_is_a_valid_error_response() {
        let request: Invocation = serde_json::from_value(json!({"protocolVersion":2,"operation":"submit","profileId":"test","bindingId":"test","capabilitySlot":"videoFaceRefine","requestId":"test","budgetMs":1000,"payloadJson":"{}","negotiatedVersion":2,"enabledFeatures":[]})).unwrap();
        let response = serde_json::to_value(response_fail(&request, fail("BUDGET_EXCEEDED", "HTTP response buffer exceeds public policy"))).unwrap();
        assert_eq!(response["error"]["code"], "RESOURCE_UNAVAILABLE");
        assert_eq!(response["error"]["message"], "BUDGET_EXCEEDED: HTTP response buffer exceeds public policy");
        assert_eq!(response["error"]["retryable"], false);
    }
    #[test]
    fn submission_transport_failure_is_not_safe_to_resubmit() {
        struct LostReceipt;
        impl Host for LostReceipt {
            fn call(&mut self, call: bgx_adapter_v2_sdk::HostPortCallV2) -> core::result::Result<Value, host::HostError> {
                let v = serde_json::to_value(call).unwrap();
                if v.to_string().contains("object_info") { return Ok(json!({"status":200,"payload":{"bytes":b"{}".to_vec()}})); }
                Err(serde_json::from_value(json!({"code":"TIMEOUT","message":"connection lost"})).unwrap())
            }
        }
        let error = comfy_submit_prompt(&mut LostReceipt,"http://localhost:8188",json!({}),"test", &mut Phase::BeforeSubmission).unwrap_err();
        assert_eq!(error.code,"ACCEPTANCE_UNKNOWN");
        assert!(!error.retryable);
    }
    #[test]
    fn only_explicit_service_validation_receipt_marks_service_rejected() {
        struct Reply { service_response: bool }
        impl Host for Reply {
            fn call(&mut self, call: bgx_adapter_v2_sdk::HostPortCallV2) -> core::result::Result<Value, host::HostError> {
                let v=serde_json::to_value(call).unwrap();
                if v["args"]["plan"]["method"] == "GET" { return Ok(json!({"status":200,"payload":{"bytes":b"{}".to_vec()}})); }
                if !self.service_response { return Err(host::invalid_wire("host input rejected without service receipt")); }
                let bytes=serde_json::to_vec(&json!({"error":{"type":"prompt_outputs_failed_validation"},"node_errors":{}})).unwrap();
                Ok(json!({"status":400,"payload":{"bytes":bytes}}))
            }
        }
        let mut phase=Phase::BeforeSubmission;
        let error=comfy_submit_prompt(&mut Reply{service_response:false},"http://localhost",json!({}),"test",&mut phase).unwrap_err();
        assert!(matches!(phase,Phase::Unknown));
        assert!(!error.service_rejected);
        let error=comfy_submit_prompt(&mut Reply{service_response:true},"http://localhost",json!({}),"test",&mut phase).unwrap_err();
        assert!(matches!(phase,Phase::ServiceRejected));
        assert!(error.service_rejected);
    }
    #[test]
    fn graph_uses_uploaded_refs_identity_tracking_and_actual_fps() {
        let wf = inject_refine_workflow("0", "input.mp4", "原文", &json!({"appearanceImages":["a.png","b.png"],"identityImage":"person.png"})).unwrap();
        assert!(wf.get("ref0").is_none());
        assert_eq!(wf["r2v"]["inputs"]["ref_images.ref_image_1"],json!(["appearance_1",0]));
        assert_eq!(wf["track"]["inputs"]["identity_reference"],json!(["identity_ref",0]));
        assert_eq!(wf["create"]["inputs"]["fps"],json!(["select",6]));
        assert_eq!(wf["r2v"]["inputs"]["prompt"],"原文");
        let empty = inject_refine_workflow("0", "v.mp4", "", &json!({"appearanceImages":[]})).unwrap();
        assert!(empty["r2v"]["inputs"].get("ref_images.ref_image_0").is_none());
    }
    #[test]
    fn preview_has_receipt_output_and_actual_fps() {
        let analysis = json!({"analysisId":"a"}); let selection = json!({"selectionDigest":"d"});
        let wf = inject_preview_workflow("0", "input.mp4", &analysis, &selection).unwrap();
        assert_eq!(wf["tracking_receipt"]["class_type"],"H3FaceTrackingReceipt");
        assert_eq!(wf["3"]["inputs"]["fps"],json!(["1",6]));
        assert_eq!(wf["tracking_receipt"]["inputs"]["analysis_json"],analysis.to_string());
    }
    #[test]
    fn analysis_uses_managed_upload_and_preserves_source_identity() {
        let source = json!({"mediaId":"source","digest":"frozen-source"});
        let payload = json!({"configuration":{"endpointBaseUrl":"http://127.0.0.1:8188"},"input":{"source":source},"media":[{"id":"source","handle":"managed-clip","kind":"video","mime":"video/mp4","fileName":"clip.mp4","byteLength":"123"}]});
        let mut host = TestHost { calls: 0 };
        let result = run_analyze(&mut host, &payload, &mut Phase::BeforeSubmission).unwrap();
        assert_eq!(host.calls, 4);
        assert_eq!(result["data"]["source"], source);
        assert!(result["data"].get("vendor").is_none());
        assert_eq!(result["data"]["shots"][0]["candidates"][0]["box"]["width"], 128.0);
    }
}
