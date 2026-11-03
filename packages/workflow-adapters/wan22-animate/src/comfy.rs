//! ComfyUI HTTP exchange through the SDK managed ports only (no own polling loop,
//! no task database). Endpoints: /upload/image, /prompt, /queue, /history, /view.
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use bgx_adapter_v2_sdk::{host, Host};
use serde_json::{json, Value};

use crate::Failure;

pub struct Ctx<'a> {
    pub config: &'a Value,
    pub budget_ms: u64,
}

fn malformed() -> Failure {
    Failure::new("UPSTREAM_ERROR", "ComfyUI returned an unexpected response")
}
pub fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, Failure> {
    v.get(key).and_then(Value::as_str).ok_or_else(malformed)
}
fn max_buffer() -> u64 {
    policy_limit("maxBufferedHttpBytes")
}
fn policy_limit(key: &str) -> u64 {
    let p: Value = serde_json::from_str(include_str!(
        "L:/00Dev/bgxiong-ai-story/packages/vendor-adapter-contract/v2/protocol/schema-policy.json"
    ))
    .expect("published SDK policy");
    p["limits"][key].as_u64().expect("published SDK budget")
}

pub fn port(h: &mut dyn Host, method: &str, args: Value) -> Result<Value, Failure> {
    host::call(h, method, &args).map_err(|e| {
        let v = serde_json::to_value(&e).expect("public host error");
        let code = match v["code"].as_str() {
            Some("TIMEOUT") => "TIMEOUT",
            Some("INVALID_INPUT") => "INVALID_INPUT",
            Some("PERMISSION_DENIED") => "PERMISSION_DENIED",
            _ => "RESOURCE_UNAVAILABLE",
        };
        Failure::new(
            code,
            "A managed host operation failed; no retry was attempted",
        )
    })
}

fn plan(
    ctx: &Ctx,
    method: &str,
    path: &str,
    body: Value,
    headers: Vec<Value>,
    sink: Value,
) -> Result<Value, Failure> {
    let credential = ctx
        .config
        .get("credentialRef")
        .ok_or_else(|| Failure::new("INVALID_INPUT", "credentialRef is required"))?;
    let generated = match credential {
        Value::Null => vec![],
        Value::String(s) if !s.is_empty() => {
            vec![json!({"kind": "bearer", "name": "Authorization", "targetField": "Authorization"})]
        }
        _ => {
            return Err(Failure::new(
                "INVALID_INPUT",
                "credentialRef must be null or a stable reference",
            ))
        }
    };
    let base = text(ctx.config, "endpointBaseUrl")
        .map_err(|_| Failure::new("INVALID_INPUT", "endpointBaseUrl is required"))?;
    Ok(
        json!({"plan": {"method": method, "url": format!("{}{}", base.trim_end_matches('/'), path), "headers": headers,
        "generatedHeaders": generated, "body": body, "sink": sink, "timeoutMs": ctx.budget_ms}, "credential": credential}),
    )
}

fn status_error(status: u64, body: &Value) -> Failure {
    let mut f = Failure::new(
        match status {
            401 | 403 => "UNAUTHORIZED",
            429 => "RATE_LIMITED",
            _ => "UPSTREAM_ERROR",
        },
        "ComfyUI rejected the request; no retry was attempted",
    );
    f.vendor = body["error"]["type"].as_str().map(str::to_string);
    f
}

/// Buffered JSON exchange. `acceptance` marks submit-like calls whose lost
/// response must not be resubmitted.
pub fn exchange_public(
    h: &mut dyn Host,
    ctx: &Ctx,
    method: &str,
    path: &str,
    body: Value,
    headers: Vec<Value>,
    acceptance: bool,
) -> Result<Value, Failure> {
    exchange(h, ctx, method, path, body, headers, acceptance)
}

fn exchange(
    h: &mut dyn Host,
    ctx: &Ctx,
    method: &str,
    path: &str,
    body: Value,
    headers: Vec<Value>,
    acceptance: bool,
) -> Result<Value, Failure> {
    let unknown = || {
        Failure::new(
            "ACCEPTANCE_UNKNOWN",
            "The acknowledgment was lost or incomplete; do not resubmit",
        )
    };
    let args = plan(
        ctx,
        method,
        path,
        body,
        headers,
        json!({"kind": "buffer", "maxBytes": max_buffer()}),
    )?;
    let response =
        port(h, "host-http.execute", args).map_err(|e| if acceptance { unknown() } else { e })?;
    let bytes: Vec<u8> =
        serde_json::from_value(response["payload"]["bytes"].clone()).map_err(|_| malformed())?;
    let value: Value =
        serde_json::from_slice(&bytes)
            .map_err(|_| if acceptance { unknown() } else { malformed() })?;
    let status = response["status"].as_u64().ok_or_else(malformed)?;
    if !(200..300).contains(&status) {
        return Err(status_error(status, &value));
    }
    Ok(value)
}

/// Uploads one leased media file; returns the server file reference used by
/// LoadImage/LoadVideo (`subfolder/name` when the server places it in a subfolder).
pub fn upload(h: &mut dyn Host, ctx: &Ctx, media: &Value) -> Result<String, Failure> {
    let parts = json!([
        {"kind": "file", "name": "image", "fileName": text(media, "fileName")?, "handle": text(media, "handle")?, "mime": text(media, "mime")?},
        {"kind": "field", "name": "type", "value": "input"}
    ]);
    let v = exchange(
        h,
        ctx,
        "POST",
        "/upload/image",
        json!({"kind": "multipart", "parts": parts}),
        vec![],
        true,
    )?;
    let name = text(&v, "name")?;
    Ok(match v.get("subfolder").and_then(Value::as_str) {
        Some(sub) if !sub.is_empty() => format!("{sub}/{name}"),
        _ => name.to_string(),
    })
}

pub fn submit(
    h: &mut dyn Host,
    ctx: &Ctx,
    graph: &Value,
    client_id: &str,
) -> Result<String, Failure> {
    let body = serde_json::to_vec(&json!({"prompt": graph, "client_id": client_id}))
        .map_err(|_| malformed())?;
    let v = exchange(
        h,
        ctx,
        "POST",
        "/prompt",
        json!({"kind": "bytes", "bytes": body}),
        vec![json!({"name": "Content-Type", "value": "application/json"})],
        true,
    )?;
    let id = text(&v, "prompt_id").map_err(|_| {
        Failure::new(
            "ACCEPTANCE_UNKNOWN",
            "Submit response has no prompt_id; do not resubmit",
        )
    })?;
    if id.is_empty() {
        return Err(Failure::new(
            "ACCEPTANCE_UNKNOWN",
            "Submit response has an empty prompt_id; do not resubmit",
        ));
    }
    Ok(id.to_string())
}

const REFERENCE_PATH: &str = "/bgx/wan22-animate/reference";
const REFERENCE_POLICY: &str = "contain-gray-v1";

#[cfg(test)]
mod reference_tests {
    use super::*;
    use bgx_adapter_v2_sdk::{HostPortCallV2, host::HostError};

    struct ReferenceHost { response: Value, calls: Vec<Value> }
    impl Host for ReferenceHost {
        fn call(&mut self, call: HostPortCallV2) -> Result<Value, HostError> {
            self.calls.push(serde_json::to_value(call).unwrap());
            Ok(json!({"status": 200, "payload": {"bytes": serde_json::to_vec(&self.response).unwrap()}}))
        }
    }
    fn config() -> Value { json!({"endpointBaseUrl": "http://127.0.0.1:8188", "credentialRef": null}) }
    fn host() -> ReferenceHost { ReferenceHost { response: json!({"policy": REFERENCE_POLICY,
        "image": "bgx-wan22-reference/reference-1.png", "width": 1280, "height": 720}), calls: vec![] } }

    #[test]
    fn reference_preparation_uses_frozen_dimensions_and_derived_file() {
        let config = config();
        let ctx = Ctx { config: &config, budget_ms: 10000 };
        let mut h = host();
        ensure_reference_preparation(&mut h, &ctx).unwrap();
        assert_eq!(prepare_reference(&mut h, &ctx, "original-portrait.png", 1280, 720).unwrap(), "bgx-wan22-reference/reference-1.png");
        assert_eq!(h.calls.len(), 2);
        assert_eq!(h.calls[0]["args"]["plan"]["method"], "GET");
        let args = &h.calls[1]["args"]["plan"];
        assert_eq!(args["method"], "POST");
        assert_eq!(args["url"], "http://127.0.0.1:8188/bgx/wan22-animate/reference");
        let bytes: Vec<u8> = serde_json::from_value(args["body"]["bytes"].clone()).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!({"image": "original-portrait.png", "width": 1280, "height": 720}));
    }

    #[test]
    fn unprepared_or_mismatched_reference_is_never_used() {
        let config = config();
        let ctx = Ctx { config: &config, budget_ms: 10000 };
        for (key, value) in [("policy", json!("old")), ("width", json!(720)),
            ("image", json!("../original.png")), ("image", json!("original.png"))] {
            let mut h = host(); h.response[key] = value;
            assert!(prepare_reference(&mut h, &ctx, "original.png", 1280, 720).is_err());
            assert_eq!(h.calls.len(), 1);
        }
        let mut h = host(); h.response = json!({});
        assert!(ensure_reference_preparation(&mut h, &ctx).is_err());
    }
}

/// Fail before uploads when Comfy still has an older companion loaded.
pub fn ensure_reference_preparation(h: &mut dyn Host, ctx: &Ctx) -> Result<(), Failure> {
    let result = exchange(h, ctx, "GET", REFERENCE_PATH, json!({"kind": "none"}), vec![], false)
        .map_err(|_| Failure::new("RESOURCE_UNAVAILABLE",
            "Reference preparation is unavailable; install this adapter's companion and restart ComfyUI"))?;
    if result["policy"] != REFERENCE_POLICY {
        return Err(Failure::new("RESOURCE_UNAVAILABLE", "Reference preparation policy does not match this adapter"));
    }
    Ok(())
}

/// The unchanged workflow receives one complete, correctly sized reference image.
pub fn prepare_reference(h: &mut dyn Host, ctx: &Ctx, image: &str, width: u64, height: u64) -> Result<String, Failure> {
    let bytes = serde_json::to_vec(&json!({"image": image, "width": width, "height": height}))
        .map_err(|_| malformed())?;
    let result = exchange(h, ctx, "POST", REFERENCE_PATH,
        json!({"kind": "bytes", "bytes": bytes}),
        vec![json!({"name": "Content-Type", "value": "application/json"})], false)?;
    let image = text(&result, "image")?;
    if result["policy"] != REFERENCE_POLICY || result["width"] != width || result["height"] != height
        || !bgx_adapter_v2_sdk::guest::valid_package_path(image)
        || !image.starts_with("bgx-wan22-reference/") || !image.ends_with(".png") {
        return Err(Failure::new("UPSTREAM_ERROR", "Prepared reference identity or dimensions do not match; generation was not submitted"));
    }
    Ok(image.to_string())
}

pub enum Remote {
    Queued,
    Running,
    /// (filename, subfolder, type) of the declared output node.
    Succeeded(Vec<(String, String, String)>),
    Failed(Option<String>),
    Lost,
}

fn listed(queue: &Value, key: &str, id: &str) -> bool {
    queue[key]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item[1] == id))
}

/// GET whose JSON can exceed the buffered HTTP limit: `/history` carries every UI output
/// (DWPreprocessor writes per-frame openpose JSON, ~16 KB/frame). Received into managed
/// media, read back in policy-sized chunks, then released.
fn large_json(h: &mut dyn Host, ctx: &Ctx, path: &str) -> Result<Value, Failure> {
    const MAX_BYTES: usize = 48 * 1024 * 1024;
    let response = port(
        h,
        "host-http.execute",
        plan(ctx, "GET", path, json!({"kind": "none"}), vec![], json!({"kind": "media"}))?,
    )?;
    let status = response["status"].as_u64().ok_or_else(malformed)?;
    let handle = text(&response["payload"], "handle")?.to_string();
    let chunk = policy_limit("maxHttpChunkBytes");
    let mut bytes: Vec<u8> = Vec::new();
    let read = (|| loop {
        let part: Vec<u8> = serde_json::from_value(port(
            h,
            "host-media.read",
            json!({"handle": handle, "offset": alloc::format!("{}", bytes.len()), "maxBytes": chunk}),
        )?)
        .map_err(|_| malformed())?;
        if part.is_empty() {
            return Ok(());
        }
        bytes.extend_from_slice(&part);
        if bytes.len() > MAX_BYTES {
            return Err(Failure::new("UPSTREAM_ERROR", "ComfyUI response is too large"));
        }
    })();
    let released = port(h, "host-media.release", json!({"handle": handle}));
    read?;
    released?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| malformed())?;
    if !(200..300).contains(&status) {
        return Err(status_error(status, &value));
    }
    Ok(value)
}

/// Queue is read before history so a prompt finishing in between is seen as done.
pub fn status(h: &mut dyn Host, ctx: &Ctx, id: &str, output_node: &str) -> Result<Remote, Failure> {
    let queue = exchange(
        h,
        ctx,
        "GET",
        "/queue",
        json!({"kind": "none"}),
        vec![],
        false,
    )?;
    let history = large_json(h, ctx, &format!("/history/{}", encode(id)))?;
    let Some(entry) = history.get(id) else {
        return Ok(if listed(&queue, "queue_running", id) {
            Remote::Running
        } else if listed(&queue, "queue_pending", id) {
            Remote::Queued
        } else {
            Remote::Lost
        });
    };
    match entry["status"]["status_str"].as_str() {
        Some("success") => {}
        Some("error") => {
            let node = entry["status"]["messages"].as_array().and_then(|m| {
                m.iter()
                    .find(|x| x[0] == "execution_error")
                    .and_then(|x| x[1]["exception_type"].as_str())
                    .map(str::to_string)
            });
            return Ok(Remote::Failed(node));
        }
        _ => return Ok(Remote::Running),
    }
    let files = entry["outputs"][output_node]["images"]
        .as_array()
        .ok_or_else(malformed)?;
    let mut out = Vec::new();
    for f in files {
        out.push((
            text(f, "filename")?.to_string(),
            f.get("subfolder")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            text(f, "type")?.to_string(),
        ));
    }
    if out.len() != 1 {
        return Err(malformed());
    }
    Ok(Remote::Succeeded(out))
}

/// Downloads an output into managed media and returns the standard artifact.
pub fn download(
    h: &mut dyn Host,
    ctx: &Ctx,
    file: &(String, String, String),
) -> Result<Value, Failure> {
    let path = format!(
        "/view?filename={}&subfolder={}&type={}",
        encode(&file.0),
        encode(&file.1),
        encode(&file.2)
    );
    let response = port(
        h,
        "host-http.execute",
        plan(
            ctx,
            "GET",
            &path,
            json!({"kind": "none"}),
            vec![],
            json!({"kind": "media"}),
        )?,
    )?;
    let status = response["status"].as_u64().ok_or_else(malformed)?;
    if status != 200 {
        return Err(status_error(status, &Value::Null));
    }
    let handle = text(&response["payload"], "handle")?;
    let stat = port(h, "host-media.info", json!({"handle": handle}))?;
    if stat["kind"] != "video" {
        return Err(Failure::new(
            "UPSTREAM_ERROR",
            "ComfyUI output is not a video",
        ));
    }
    Ok(
        json!({"kind": "media", "role": "video", "media": {"id": stat["id"], "kind": "video", "handle": handle,
        "mime": stat["mime"], "fileName": stat["fileName"], "byteLength": stat["byteLength"]}}),
    )
}

/// Standard media output for one managed handle.
pub fn media_output(role: &str, handle: &str, stat: &Value) -> Value {
    json!({"kind": "media", "role": role, "media": {"id": stat["id"], "kind": stat["kind"], "handle": handle,
        "mime": stat["mime"], "fileName": stat["fileName"], "byteLength": stat["byteLength"]}})
}

/// Downloads one ComfyUI output image into managed media. Returns (handle, stat).
pub fn download_image(
    h: &mut dyn Host,
    ctx: &Ctx,
    filename: &str,
    subfolder: &str,
    kind: &str,
) -> Result<(String, Value), Failure> {
    let path = format!(
        "/view?filename={}&subfolder={}&type={}",
        encode(filename),
        encode(subfolder),
        encode(kind)
    );
    let response = port(
        h,
        "host-http.execute",
        plan(ctx, "GET", &path, json!({"kind": "none"}), vec![], json!({"kind": "media"}))?,
    )?;
    let status = response["status"].as_u64().ok_or_else(malformed)?;
    if status != 200 {
        return Err(status_error(status, &Value::Null));
    }
    let handle = text(&response["payload"], "handle")?.to_string();
    let stat = port(h, "host-media.info", json!({"handle": handle}))?;
    if stat["kind"] != "image" {
        return Err(Failure::new("UPSTREAM_ERROR", "ComfyUI mask frame is not an image"));
    }
    Ok((handle, stat))
}

/// Writes package-built bytes as one managed file. Returns (handle, stat).
pub fn create_file(h: &mut dyn Host, mime: &str, bytes: &[u8]) -> Result<(String, Value), Failure> {
    let handle = port(h, "host-media.create", json!({"kind": "file", "mimeHint": mime}))?
        .as_str()
        .ok_or_else(malformed)?
        .to_string();
    let chunk = policy_limit("maxMediaChunkBytes") as usize;
    for part in bytes.chunks(chunk.max(1)) {
        port(h, "host-media.write", json!({"handle": handle, "bytes": part}))?;
    }
    port(h, "host-media.finish", json!({"handle": handle}))?;
    let stat = port(h, "host-media.info", json!({"handle": handle}))?;
    Ok((handle, stat))
}

/// Connectivity probe: `/system_stats` answers without side effects.
pub fn status_probe(h: &mut dyn Host, ctx: &Ctx) -> Result<(), Failure> {
    let v = exchange(
        h,
        ctx,
        "GET",
        "/system_stats",
        json!({"kind": "none"}),
        vec![],
        false,
    )?;
    v.get("system").map(|_| ()).ok_or_else(malformed)
}

/// RFC 3986 percent-encoding of a query/path component.

/// Connectivity-time inventory. `status` is ready|failed|not_checked.
/// Generation model gaps are listed as issues; tracking may still run when SAM2 companions exist.
pub struct ResourceReport {
    pub status: &'static str,
    pub issues: alloc::vec::Vec<Value>,
}

pub fn resource_inventory(h: &mut dyn Host, ctx: &Ctx) -> ResourceReport {
    let mut issues = alloc::vec::Vec::new();
    let mut tracking_ok = true;
    match companion_loaded_identity(h, ctx) {
        Ok(identity) => {
            let version = identity.get("version").and_then(Value::as_str).unwrap_or("");
            if !version.is_empty() && version != env!("CARGO_PKG_VERSION") {
                tracking_ok = false;
                issues.push(json!({
                    "code": "COMPANION_CODE_MISMATCH",
                    "message": "已加载的伴随节点版本与安装包不一致，请重启 ComfyUI 后再试"
                }));
            }
        }
        Err(err) => {
            // Route missing usually means old process or nodes not loaded.
            tracking_ok = false;
            issues.push(json!({"code": err.code, "message": err.message}));
        }
    }
    if let Err(err) = ensure_companion_nodes_tracking(h, ctx) {
        tracking_ok = false;
        issues.push(json!({"code": err.code, "message": err.message}));
    }
    if let Err(err) = ensure_generation_models_readable(h, ctx) {
        issues.push(json!({"code": err.code, "message": err.message}));
    }
    let status = if issues.is_empty() {
        "ready"
    } else {
        "failed"
    };
    let _ = tracking_ok;
    ResourceReport { status, issues }
}

fn companion_loaded_identity(h: &mut dyn Host, ctx: &Ctx) -> Result<Value, Failure> {
    exchange(
        h,
        ctx,
        "GET",
        "/bgx/wan22-animate/companion/identity",
        json!({"kind": "none"}),
        vec![],
        false,
    )
}

fn ensure_generation_models_readable(h: &mut dyn Host, ctx: &Ctx) -> Result<(), Failure> {
    // Presence of the UNet loader node class — weights may still be missing at submit time.
    let path = format!("/object_info/{}", encode("UNETLoader"));
    match exchange(h, ctx, "GET", &path, json!({"kind": "none"}), vec![], false) {
        Ok(info) if info.get("UNETLoader").is_some() => Ok(()),
        Ok(_) | Err(_) => Err(Failure::new(
            "RESOURCE_UNAVAILABLE",
            "生成模型加载节点未就绪；人物替换生成不可用，但 SAM2 跟踪在伴随节点齐全时可继续。请在设置中补齐模型资源",
        )),
    }
}
/// Minimal Mix-mode dependency check: required companion nodes must exist with expected
/// inputs/outputs. Missing companion fails before any upload or GPU work; never falls back
/// to legacy Sam2Segmentation.
pub fn ensure_companion_nodes(h: &mut dyn Host, ctx: &Ctx) -> Result<(), Failure> {
    const REQUIRED: &[(&str, &[&str], &[&str])] = &[
        (
            "BGXAnimateFrameSequence",
            &["images", "source_fps", "target_fps", "frame_count"],
            &["IMAGE"],
        ),
        (
            "BGXAnimateBidirectionalMask",
            &[
                "sam2_model",
                "image",
                "coordinates_positive",
                "frame_time_ms",
                "target_fps",
                "frame_count",
            ],
            &["MASK"],
        ),
    ];
    for (name, required_inputs, required_outputs) in REQUIRED {
        let path = format!("/object_info/{}", encode(name));
        let info = exchange(h, ctx, "GET", &path, json!({"kind": "none"}), vec![], false).map_err(
            |_| {
                Failure::new(
                    "RESOURCE_UNAVAILABLE",
                    "Required companion node is missing from ComfyUI; install BGXAnimateTracking and restart",
                )
            },
        )?;
        let node = info
            .get(*name)
            .ok_or_else(|| {
                Failure::new(
                    "RESOURCE_UNAVAILABLE",
                    "Required companion node is missing from ComfyUI; install BGXAnimateTracking and restart",
                )
            })?;
        let input_names: alloc::collections::BTreeSet<&str> = node
            .pointer("/input/required")
            .and_then(Value::as_object)
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        for key in *required_inputs {
            if !input_names.contains(key) {
                return Err(Failure::new(
                    "RESOURCE_UNAVAILABLE",
                    "Companion node input contract mismatch; install the matching BGXAnimateTracking package",
                ));
            }
        }
        let outputs = node
            .get("output")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Failure::new(
                    "RESOURCE_UNAVAILABLE",
                    "Companion node output contract mismatch; install the matching BGXAnimateTracking package",
                )
            })?;
        for expected in *required_outputs {
            if !outputs.iter().any(|v| v.as_str() == Some(*expected)) {
                return Err(Failure::new(
                    "RESOURCE_UNAVAILABLE",
                    "Companion node output contract mismatch; install the matching BGXAnimateTracking package",
                ));
            }
        }
        // Explicitly refuse legacy-only environments that still only expose Sam2Segmentation.
        let _ = node; // checked above
    }
    Ok(())
}


/// Tracking-preview companion nodes (multi-keyframe + artifact export/load).
pub fn ensure_companion_nodes_tracking(h: &mut dyn Host, ctx: &Ctx) -> Result<(), Failure> {
    const REQUIRED: &[(&str, &[&str], &[&str])] = &[
        (
            "BGXAnimateFrameSequence",
            &["images", "source_fps", "target_fps", "frame_count"],
            &["IMAGE"],
        ),
        (
            "BGXAnimateMultiKeyframeMask",
            &[
                "sam2_model",
                "image",
                "keyframes_json",
                "direction",
                "frame_count",
                "request_id",
                "selection_digest",
                "frame_times_json",
                "span_ms",
            ],
            &["MASK"],
        ),
        (
            "BGXAnimateMaskArtifactExport",
            &["masks", "request_id", "selection_digest", "direction", "frame_times_json"],
            &["STRING"],
        ),
        (
            "BGXAnimateMaskArtifactLoad",
            &["artifact_id", "digest", "expected_frame_count"],
            &["MASK"],
        ),
    ];
    for (name, required_inputs, required_outputs) in REQUIRED {
        let path = format!("/object_info/{}", encode(name));
        let info = exchange(h, ctx, "GET", &path, json!({"kind": "none"}), vec![], false).map_err(
            |_| {
                Failure::new(
                    "RESOURCE_UNAVAILABLE",
                    "Required tracking companion node is missing; install BGXAnimateTracking 0.2.1 and restart",
                )
            },
        )?;
        let node = info.get(*name).ok_or_else(|| {
            Failure::new(
                "RESOURCE_UNAVAILABLE",
                "Required tracking companion node is missing; install BGXAnimateTracking 0.2.1 and restart",
            )
        })?;
        let input_names: alloc::collections::BTreeSet<&str> = node
            .pointer("/input/required")
            .and_then(Value::as_object)
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        for key in *required_inputs {
            if !input_names.contains(key) {
                return Err(Failure::new(
                    "RESOURCE_UNAVAILABLE",
                    "Tracking companion node input contract mismatch; install matching 0.2.1 package",
                ));
            }
        }
        let outputs = node.get("output").and_then(Value::as_array).ok_or_else(|| {
            Failure::new(
                "RESOURCE_UNAVAILABLE",
                "Tracking companion node output contract mismatch; install matching 0.2.1 package",
            )
        })?;
        for expected in *required_outputs {
            if !outputs.iter().any(|v| v.as_str() == Some(*expected)) {
                return Err(Failure::new(
                    "RESOURCE_UNAVAILABLE",
                    "Tracking companion node output contract mismatch; install matching 0.2.1 package",
                ));
            }
        }
        let _ = node;
    }
    Ok(())
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn percent_encoding() {
        assert_eq!(super::encode("video/a b.mp4"), "video%2Fa%20b.mp4");
    }
}
