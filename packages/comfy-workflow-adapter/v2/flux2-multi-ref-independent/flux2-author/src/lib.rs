#![cfg_attr(target_arch = "wasm32", no_std)]
//! Independent FLUX.2 ordered two-reference image.generate package.
//! Native API export, source provenance and explicit parameter transformations are in SOURCE.json.
//! Platform/node/HTTP semantics stay inside this package. No application source is required.
extern crate alloc;
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PLUGIN_ID: &str = "local.independent.flux2-ordered-reference";
const MODEL_ID: &str = "flux2-dev-ordered-two-reference";
const OPERATION: &str = "local.independent.flux2-ordered-reference/flux2-multi-ref";
/// Native ComfyUI Export (API) graph; exact file digest is recorded in SOURCE.json.
const GRAPH: &str = include_str!("workflow-api.json");
const PROMPT_NODE: &str = "62:6";
const SEED_NODE: &str = "62:25";
const SCHEDULER_NODE: &str = "62:48";
const LATENT_NODE: &str = "62:47";
const GUIDANCE_NODE: &str = "62:26";
const TURBO_NODE: &str = "62:95";
const OUTPUT_NODE: &str = "9";
const REFERENCE_NODES: [&str;2] = ["42", "46"];
const MAX_PROMPT: usize = 8000;
const MAX_SEED: u64 = 9007199254740991;
const INPUT_SCHEMA: &str = include_str!("../schemas/input.json");
const OUTPUT_SCHEMA: &str = r#"{"type":"object","properties":{"description":{"type":"string"}},"required":["description"],"additionalProperties":false}"#;
const CONFIG_SCHEMA: &str = r#"{"type":"object","properties":{"endpointBaseUrl":{"type":"string","minLength":1}},"required":["endpointBaseUrl"],"additionalProperties":false}"#;
const RESUME_SCHEMA: &str = include_str!("../schemas/resume.json");

type Outcome<T> = Result<T, Failure>;
struct Failure {
    code: &'static str,
    message: &'static str,
    vendor: Option<String>,
    request: Option<String>,
}
fn fail(code: &'static str, message: &'static str) -> Failure {
    Failure {
        code,
        message,
        vendor: None,
        request: None,
    }
}
fn malformed() -> Failure {
    fail(
        "UPSTREAM_ERROR",
        "ComfyUI returned an unexpected structured response",
    )
}
fn text<'a>(v: &'a Value, key: &str) -> Outcome<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(malformed)
}
fn number(v: &Value, key: &str) -> Outcome<u64> {
    v.get(key).and_then(Value::as_u64).ok_or_else(malformed)
}
fn policy(_name: &str) -> u64 {
    1_048_576
} // protocol/schema-policy.json limits.maxBufferedHttpBytes
fn port(h: &mut dyn Host, method: &str, args: Value) -> Outcome<Value> {
    host::call(h, method, &args).map_err(|e| {
        let v = serde_json::to_value(&e).expect("public host error");
        let code = match v["code"].as_str() {
            Some("TIMEOUT") => "TIMEOUT",
            Some("INVALID_INPUT") => "INVALID_INPUT",
            Some("PERMISSION_DENIED") => "PERMISSION_DENIED",
            _ => "RESOURCE_UNAVAILABLE",
        };
        fail(
            code,
            "A managed host operation failed; no retry was attempted",
        )
    })
}
fn schema(id: &str, source: &str) -> Value {
    let digest: String = Sha256::digest(source.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({"id":id,"digest":digest,"schemaJson":source})
}
fn capabilities() -> Value {
    let input = schema("input", INPUT_SCHEMA);
    let output = schema("output", OUTPUT_SCHEMA);
    let config = schema("config", CONFIG_SCHEMA);
    let resume = schema("resume", RESUME_SCHEMA);
    json!({"schemaVersion":2,"pluginId":PLUGIN_ID,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[{
        "schemaVersion":2,"localId":"flux2-multi-ref","capabilityKey":"image.generate","transportSlot":"image","modelIds":[MODEL_ID],
        "configurationSchema":{"id":"config","digest":config["digest"]},
        "operations":[{"id":OPERATION,"version":1,"label":"FLUX.2 ordered two-image reference generation",
            "inputSchema":{"id":"input","digest":input["digest"]},"outputSchema":{"id":"output","digest":output["digest"]},
            "resumeSchema":{"id":"resume","digest":resume["digest"]},
            "uiSchema":{"controls":[
                {"fieldPointer":"/referenceImages","widget":"repeat","label":"Ordered references","description":"Exactly two originals, bound in the original workflow image_1/image_2 order","group":"Input"},
                {"fieldPointer":"/prompt","widget":"multiline","label":"Prompt","description":"","group":"Input"},
                {"fieldPointer":"/seed","widget":"number","label":"Seed","description":"","group":"Input"},
                {"fieldPointer":"/width","widget":"number","label":"Width","description":"","group":"Input"},
                {"fieldPointer":"/height","widget":"number","label":"Height","description":"","group":"Input"},
                {"fieldPointer":"/advanced/steps","widget":"integer","label":"采样步数","description":"Confirmed explicit step count for either model branch","group":"Advanced"},
                {"fieldPointer":"/advanced/guidance","widget":"number","label":"FLUX引导强度","description":"Native FluxGuidance value","group":"Advanced"},
                {"fieldPointer":"/advanced/enableTurbo","widget":"toggle","label":"启用Turbo LoRA","description":"Native model switch; does not silently change steps","group":"Advanced"}]},
            "mediaSlots":[{"role":"referenceImages","inputPointer":"/referenceImages","mediaKind":"image","minCount":2,"maxCount":2}],
            "execution":{"mode":"asynchronous","streaming":false,"cancellation":"unsupported","resumable":true,"remoteIdempotency":false},
            "outputSlots":[{"role":"artifact","kind":"image","minCount":1,"maxCount":1}]}],
        "schemas":[input,output,config,resume]
    }]})
}
fn base(config: &Value) -> Outcome<String> {
    let b = text(config, "endpointBaseUrl")
        .map_err(|_| fail("INVALID_INPUT", "configuration.endpointBaseUrl is required"))?;
    if !(b.starts_with("http://") || b.starts_with("https://")) || b.ends_with('/') {
        return Err(fail(
            "INVALID_INPUT",
            "endpointBaseUrl must be an absolute http(s) origin without trailing slash",
        ));
    }
    Ok(b.to_string())
}
fn http(
    h: &mut dyn Host,
    method: &str,
    url: String,
    body: Option<&Value>,
    sink: Value,
    budget: u64,
) -> Outcome<Value> {
    let (body, headers) = match body {
        Some(v) => (
            json!({"kind":"bytes","bytes":serde_json::to_vec(v).map_err(|_| malformed())?}),
            vec![json!({"name":"Content-Type","value":"application/json"})],
        ),
        None => (json!({"kind":"none"}), vec![]),
    };
    port(
        h,
        "host-http.execute",
        json!({"plan":{"method":method,"url":url,"headers":headers,"generatedHeaders":[],"body":body,"sink":sink,"timeoutMs":budget},"credential":null}),
    )
}
fn buffered_json(response: &Value) -> Option<Value> {
    let bytes: Vec<u8> = serde_json::from_value(response["payload"]["bytes"].clone()).ok()?;
    serde_json::from_slice(&bytes).ok()
}
fn get_json(h: &mut dyn Host, url: String, budget: u64) -> Outcome<Value> {
    let r = http(
        h,
        "GET",
        url,
        None,
        json!({"kind":"buffer","maxBytes":policy("maxBufferedHttpBytes")}),
        budget,
    )?;
    let status = number(&r, "status")?;
    let v = buffered_json(&r).ok_or_else(malformed)?;
    if status != 200 {
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "ComfyUI rejected the read request",
            vendor: Some(format!("HTTP_{status}")),
            request: None,
        });
    }
    Ok(v)
}
fn probe(h: &mut dyn Host, p: &Value, budget: u64) -> Outcome<Value> {
    match text(p, "kind")? {
        "package_validation" => Ok(
            json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]}),
        ),
        "connectivity" => {
            let stats = get_json(
                h,
                format!("{}/system_stats", base(&p["configuration"])?),
                budget,
            )?;
            if !stats["system"]["comfyui_version"].is_string() {
                return Err(malformed());
            }
            Ok(
                json!({"packageValid":true,"connection":"ready","authentication":"not_checked","resources":"not_checked","issues":[]}),
            )
        }
        _ => Err(fail("INVALID_INPUT", "Unknown probe kind")),
    }
}
/// Validates confirmed values before any network effect; prompt bytes are not rewritten.
fn build_graph(input: &Value, reference_names: &[&str]) -> Outcome<Value> {
    reference_ids(input)?;
    if reference_names.len() != 2 || reference_names.iter().any(|name| name.is_empty()) {
        return Err(fail("INVALID_INPUT", "Both uploaded reference names are required"));
    }
    let obj = input.as_object().ok_or_else(|| fail("INVALID_INPUT", "input must be an object"))?;
    if obj.keys().any(|k| !["prompt", "seed", "width", "height", "advanced", "referenceImages"].contains(&k.as_str())) {
        return Err(fail("INVALID_INPUT", "Unknown image input"));
    }
    let prompt = obj.get("prompt").and_then(Value::as_str)
        .ok_or_else(|| fail("INVALID_INPUT", "prompt must be a string"))?;
    if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT {
        return Err(fail("INVALID_INPUT", "prompt must be 1..8000 characters"));
    }
    let seed = obj.get("seed").and_then(Value::as_u64)
        .ok_or_else(|| fail("INVALID_INPUT", "seed must be a non-negative integer"))?;
    if seed > MAX_SEED { return Err(fail("INVALID_INPUT", "seed exceeds 2^53-1")); }
    let width = number(input, "width").map_err(|_| fail("INVALID_INPUT", "width is required"))?;
    let height = number(input, "height").map_err(|_| fail("INVALID_INPUT", "height is required"))?;
    if [width,height].iter().any(|v| *v < 256 || *v > 4096 || *v % 16 != 0) {
        return Err(fail("INVALID_INPUT", "dimensions must be 256..4096, multiple of 16"));
    }
    let advanced = input["advanced"].as_object().ok_or_else(|| fail("INVALID_INPUT", "advanced is required"))?;
    if advanced.len() != 3 || advanced.keys().any(|k| !["steps","guidance","enableTurbo"].contains(&k.as_str())) {
        return Err(fail("INVALID_INPUT", "advanced must contain exactly steps, guidance and enableTurbo"));
    }
    let steps = advanced.get("steps").and_then(Value::as_u64).filter(|n| (1..=50).contains(n))
        .ok_or_else(|| fail("INVALID_INPUT", "steps must be integer 1..50"))?;
    let guidance = advanced.get("guidance").and_then(Value::as_f64).filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
        .ok_or_else(|| fail("INVALID_INPUT", "guidance must be number 0..100"))?;
    let turbo = advanced.get("enableTurbo").and_then(Value::as_bool)
        .ok_or_else(|| fail("INVALID_INPUT", "enableTurbo must be boolean"))?;
    let mut g: Value = serde_json::from_str(GRAPH).expect("frozen native graph");
    g[PROMPT_NODE]["inputs"]["text"] = json!(prompt);
    g[SEED_NODE]["inputs"]["noise_seed"] = json!(seed);
    g[SCHEDULER_NODE]["inputs"]["steps"] = json!(steps);
    for node in [SCHEDULER_NODE, LATENT_NODE] {
        g[node]["inputs"]["width"] = json!(width);
        g[node]["inputs"]["height"] = json!(height);
    }
    g[GUIDANCE_NODE]["inputs"]["guidance"] = json!(guidance);
    g[TURBO_NODE]["inputs"]["value"] = json!(turbo);
    for (node,name) in REFERENCE_NODES.iter().zip(reference_names) {
        g[*node]["inputs"]["image"] = json!(name);
    }
    Ok(g)
}
fn reference_ids(input: &Value) -> Outcome<Vec<&str>> {
    let refs = input["referenceImages"].as_array().filter(|r| r.len() == 2)
        .ok_or_else(|| fail("INVALID_INPUT", "Exactly two ordered original images are required"))?;
    let mut ids = Vec::new();
    for reference in refs {
        let object = reference.as_object().filter(|r| r.len() == 1)
            .ok_or_else(|| fail("INVALID_INPUT", "Reference must contain only mediaId"))?;
        let id = object.get("mediaId").and_then(Value::as_str).filter(|s| !s.is_empty())
            .ok_or_else(|| fail("INVALID_INPUT", "Reference mediaId is required"))?;
        if ids.contains(&id) { return Err(fail("INVALID_INPUT", "Each reference needs its own media identity")); }
        ids.push(id);
    }
    Ok(ids)
}
/// Verify the complete lease set before the first upload. Input order alone determines roles.
fn reference_media<'a>(p: &'a Value, ids: &[&str]) -> Outcome<Vec<&'a Value>> {
    let media = p["media"].as_array().filter(|m| m.len() == ids.len())
        .ok_or_else(|| fail("INVALID_INPUT", "Both original image leases are required"))?;
    ids.iter().map(|id| {
        let matching: Vec<_> = media.iter().filter(|m| m["id"] == *id && m["kind"] == "image").collect();
        if matching.len() != 1 { return Err(fail("INVALID_INPUT", "An original image has no unique matching lease")); }
        let item = matching[0];
        for key in ["fileName", "handle", "mime"] {
            if text(item,key)?.is_empty() { return Err(fail("INVALID_INPUT", "Original lease metadata is incomplete")); }
        }
        Ok(item)
    }).collect()
}
fn upload_reference(h: &mut dyn Host, configuration: &Value, media: &Value, budget: u64) -> Outcome<String> {
    let body = json!({"kind":"multipart","parts":[
        {"kind":"file","name":"image","fileName":text(media,"fileName")?,"handle":text(media,"handle")?,"mime":text(media,"mime")?},
        {"kind":"field","name":"type","value":"input"}]});
    let args = json!({"plan":{"method":"POST","url":format!("{}/upload/image",base(configuration)?),
        "headers":[],"generatedHeaders":[],"body":body,"sink":{"kind":"buffer","maxBytes":policy("maxBufferedHttpBytes")},"timeoutMs":budget},"credential":null});
    let response = port(h, "host-http.execute", args).map_err(|_| {
        fail(
            "ACCEPTANCE_UNKNOWN",
            "Upload response was lost; no retry was attempted",
        )
    })?;
    let value = buffered_json(&response).ok_or_else(malformed)?;
    if number(&response, "status")? != 200 {
        return Err(fail(
            "UPSTREAM_ERROR",
            "ComfyUI rejected the original image upload",
        ));
    }
    let name = text(&value, "name")?;
    if name.is_empty() {
        return Err(malformed());
    }
    Ok(match value.get("subfolder").and_then(Value::as_str) {
        Some(sub) if !sub.is_empty() => format!("{sub}/{name}"),
        _ => name.to_string(),
    })
}
fn persist_accepted(h: &mut dyn Host, request: &Invocation, accepted: &Value) -> Outcome<()> {
    let read = port(
        h,
        "host-checkpoint.read",
        json!({"requestId":request.r#request_id}),
    )?;
    let mut checkpoint: Value =
        serde_json::from_str(text(&read, "valueJson")?).map_err(|_| malformed())?;
    let old = text(&read, "revision")?.to_string();
    let revision = old
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(malformed)?;
    checkpoint["revision"] = json!(revision.to_string());
    checkpoint["state"] = json!("accepted");
    checkpoint["vendorTaskId"] = accepted["vendorTaskId"].clone();
    checkpoint["resumeToken"] = accepted["resumeToken"].clone();
    checkpoint["result"] = accepted.clone();
    port(h, "host-checkpoint.write", json!({"requestId":request.r#request_id,"expectedRevision":old,"valueJson":checkpoint.to_string()})).map_err(|_| Failure {
        code: "ACCEPTANCE_UNKNOWN", message: "ComfyUI accepted the prompt but checkpoint storage failed; do not resubmit",
        vendor: Some("CHECKPOINT_COMMIT_FAILED".into()), request: accepted["vendorTaskId"].as_str().map(str::to_string),
    })?;
    Ok(())
}
fn submit(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Value> {
    if text(p, "operationId")? != OPERATION || text(p, "modelId")? != MODEL_ID {
        return Err(fail("INVALID_INPUT", "Unknown operation or model"));
    }
    // Validate confirmed values and both leases before network effects; preserve role order.
    let mut graph = build_graph(&p["input"], &["pending-1", "pending-2"])?;
    let ids = reference_ids(&p["input"])?;
    let media = reference_media(p, &ids)?;
    for (node, original) in REFERENCE_NODES.iter().zip(media) {
        graph[*node]["inputs"]["image"] = json!(upload_reference(h, &p["configuration"], original, request.r#budget_ms)?);
    }
    let url = format!("{}/prompt", base(&p["configuration"])?);
    let body = json!({"prompt":graph,"client_id":request.r#request_id});
    // Exactly one POST; any transport loss or unreadable reply is ACCEPTANCE_UNKNOWN (never resubmitted).
    let r = http(
        h,
        "POST",
        url,
        Some(&body),
        json!({"kind":"buffer","maxBytes":policy("maxBufferedHttpBytes")}),
        request.r#budget_ms,
    )
    .map_err(|_| {
        fail(
            "ACCEPTANCE_UNKNOWN",
            "Submit response was lost or interrupted; do not resubmit",
        )
    })?;
    let status = number(&r, "status").map_err(|_| {
        fail(
            "ACCEPTANCE_UNKNOWN",
            "Submit response is incomplete; do not resubmit",
        )
    })?;
    let v = buffered_json(&r).ok_or_else(|| {
        fail(
            "ACCEPTANCE_UNKNOWN",
            "Submit response is not JSON; do not resubmit",
        )
    })?;
    if status != 200 {
        let vendor = v["error"]["type"].as_str().map(str::to_string);
        return Err(Failure {
            code: if (400..500).contains(&status) {
                "INVALID_INPUT"
            } else {
                "UPSTREAM_ERROR"
            },
            message: "ComfyUI rejected the prompt",
            vendor,
            request: None,
        });
    }
    let id = v
        .get("prompt_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            fail(
                "ACCEPTANCE_UNKNOWN",
                "Submit reply lacks prompt_id; do not resubmit",
            )
        })?;
    if v.get("node_errors")
        .and_then(Value::as_object)
        .is_some_and(|o| !o.is_empty())
    {
        return Err(malformed());
    }
    let accepted = json!({"kind":"accepted","vendorTaskId":id,"appliedParams":null,"resumeToken":{"requestId":request.r#request_id,"width":p["input"]["width"],"height":p["input"]["height"]},"usage":null});
    persist_accepted(h, request, &accepted)?;
    Ok(accepted)
}
fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char)
        } else {
            o.push_str(&format!("%{b:02X}"))
        }
    }
    o
}
fn query_result(
    status: &str,
    outputs: Vec<Value>,
    description: String,
    token: Value,
    vendor: Option<String>,
    details: Value,
) -> Value {
    let failure = match status {
        "failed" => {
            json!({"category":"vendorTaskFailed","message":"The ComfyUI prompt failed","vendorCode":vendor,"details":details})
        }
        _ => Value::Null,
    };
    json!({"status":status,"outputs":outputs,"data":{"description":description},"usage":null,
        "progress":if matches!(status,"queued"|"running"){json!({"fraction":null,"message":status})}else{Value::Null},
        "terminalFailure":failure,"resumeToken":token})
}
fn in_queue(list: &Value, id: &str) -> bool {
    list.as_array().is_some_and(|a| {
        a.iter()
            .any(|e| e.get(1).and_then(Value::as_str) == Some(id))
    })
}
/// PNG signature + IHDR width/height from the real downloaded bytes (not from the file name).
fn png_dims(bytes: &[u8]) -> Option<(u64, u64)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as u64;
    let hgt = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as u64;
    Some((w, hgt))
}
fn download(
    h: &mut dyn Host,
    base: &str,
    image: &Value,
    budget: u64,
    width: u64,
    height: u64,
) -> Outcome<Value> {
    if image["type"] != "output" {
        return Err(fail(
            "UPSTREAM_ERROR",
            "Selected output is not a saved output image",
        ));
    }
    let name = text(image, "filename")?;
    let sub = image.get("subfolder").and_then(Value::as_str).unwrap_or("");
    let url = format!(
        "{base}/view?filename={}&subfolder={}&type=output",
        pct(name),
        pct(sub)
    );
    let r = http(h, "GET", url, None, json!({"kind":"media"}), budget)?;
    let status = number(&r, "status")?;
    if status != 200 {
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "ComfyUI did not return the output image",
            vendor: Some(format!("HTTP_{status}")),
            request: None,
        });
    }
    let handle = text(&r["payload"], "handle")?.to_string();
    let stat = port(h, "host-media.info", json!({"handle":handle}))?;
    let head: Vec<u8> = serde_json::from_value(port(
        h,
        "host-media.read",
        json!({"handle":handle,"offset":"0","maxBytes":32}),
    )?)
    .map_err(|_| malformed())?;
    let Some((w, hgt)) = png_dims(&head) else {
        return Err(fail(
            "UPSTREAM_ERROR",
            "Downloaded output is not a PNG image",
        ));
    };
    if (w, hgt) != (width, height) {
        return Err(fail(
            "UPSTREAM_ERROR",
            "Downloaded image size differs from the accepted input dimensions",
        ));
    }
    if stat["kind"] != "image" || stat["byteLength"] == "0" {
        return Err(fail(
            "UPSTREAM_ERROR",
            "Downloaded media is not a non-empty image",
        ));
    }
    Ok(
        json!({"kind":"media","role":"artifact","media":{"id":stat["id"],"kind":stat["kind"],"handle":handle,"mime":stat["mime"],"fileName":stat["fileName"],"byteLength":stat["byteLength"]}}),
    )
}
fn query(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Value> {
    if text(p, "operationId")? != OPERATION {
        return Err(fail("INVALID_INPUT", "Unknown operation"));
    }
    let base = base(&p["configuration"])?;
    let id = text(p, "vendorTaskId")?.to_string();
    let budget = request.r#budget_ms;
    let history = get_json(h, format!("{base}/history/{}", pct(&id)), budget)?;
    let token = p["resumeToken"].clone();
    let Some(entry) = history.get(&id) else {
        let q = get_json(h, format!("{base}/queue"), budget)?;
        let status = if in_queue(&q["queue_running"], &id) {
            "running"
        } else if in_queue(&q["queue_pending"], &id) {
            "queued"
        } else {
            return Err(Failure {
                code: "UPSTREAM_ERROR",
                message: "Prompt is neither in history nor in the queue",
                vendor: Some("PROMPT_NOT_FOUND".into()),
                request: Some(id),
            });
        };
        return Ok(query_result(
            status,
            vec![],
            format!("ComfyUI prompt {id}: {status}"),
            token,
            None,
            json!({"promptId":id}),
        ));
    };
    let st = text(&entry["status"], "status_str")?;
    match st {
        "success" => {
            if entry["status"]["completed"] != true {
                return Err(malformed());
            }
            let images = entry["outputs"][OUTPUT_NODE]["images"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if images.len() != 1 {
                return Err(Failure {
                    code: "UPSTREAM_ERROR",
                    message: "Prompt completed without exactly one image on the frozen output node",
                    vendor: Some("OUTPUT_MISSING".into()),
                    request: Some(id),
                });
            }
            let media = download(
                h,
                &base,
                &images[0],
                budget,
                number(&token, "width")?,
                number(&token, "height")?,
            )?;
            Ok(query_result(
                "succeeded",
                vec![media],
                format!("ComfyUI prompt {id}: success"),
                token,
                None,
                json!({"promptId":id,"outputNode":OUTPUT_NODE}),
            ))
        }
        "error" => {
            let vendor = entry["status"]["messages"]
                .as_array()
                .and_then(|m| {
                    m.iter()
                        .find(|e| e.get(0) == Some(&json!("execution_error")))
                })
                .and_then(|e| e[1]["exception_type"].as_str())
                .map(str::to_string)
                .or(Some("execution_error".to_string()));
            Ok(query_result(
                "failed",
                vec![],
                format!("ComfyUI prompt {id}: error"),
                token,
                vendor,
                json!({"promptId":id}),
            ))
        }
        _ => Err(malformed()),
    }
}
fn finish(request: &Invocation, result: Outcome<Value>) -> Response {
    match result {
        Ok(value) => guest::completed_response(request, &value).expect("standard serializable result"),
        Err(e) => serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,"error":{"code":e.code,"message":e.message,"retryable":false,"vendorCode":e.vendor,"vendorRequestId":e.request}})).expect("public error"),
    }
}
pub struct Flux2OrderedReferences;
impl Adapter for Flux2OrderedReferences {
    fn invoke(request: Invocation, h: &mut dyn Host) -> Response {
        let value = serde_json::to_value(&request).expect("typed invocation");
        let result = (|| {
            let p: Value = serde_json::from_str(&request.r#payload_json)
                .map_err(|_| fail("INVALID_INPUT", "Payload is not JSON"))?;
            match text(&value, "operation")? {
                "capabilities" => Ok(capabilities()),
                "probe" => probe(h, &p, request.r#budget_ms),
                "submit" => submit(h, &request, &p),
                "query" => query(h, &request, &p),
                _ => Err(fail("UNSUPPORTED_FEATURE", "Only capabilities, probe, submit and query are implemented; remote cancel is unsupported")),
            }
        })();
        finish(&request, result)
    }
}
bgx_adapter_v2_sdk::export_adapter!(Flux2OrderedReferences);

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Value {
        json!({"prompt":"Keep both reference characters together.\n原文完整保留。","seed":7,"width":768,"height":512,"advanced":{"steps":12,"guidance":3.7,"enableTurbo":false},"referenceImages":[{"mediaId":"original-1"},{"mediaId":"original-2"}]})
    }
    #[test]
    fn graph_maps_confirmed_inputs_and_preserves_native_reference_conditioning() {
        let original: Value = serde_json::from_str(GRAPH).unwrap();
        let g = build_graph(&input(), &["first.png", "second.png"]).ok().unwrap();
        assert_eq!(g[PROMPT_NODE]["inputs"]["text"], input()["prompt"]);
        assert_eq!(g[SEED_NODE]["inputs"]["noise_seed"], 7);
        assert_eq!(g[SCHEDULER_NODE]["inputs"]["steps"], 12);
        assert_eq!(g[SCHEDULER_NODE]["inputs"]["width"], 768);
        assert_eq!(g[LATENT_NODE]["inputs"]["height"], 512);
        assert_eq!(g[GUIDANCE_NODE]["inputs"]["guidance"], 3.7);
        assert_eq!(g[TURBO_NODE]["inputs"]["value"], false);
        assert_eq!(g[REFERENCE_NODES[0]]["inputs"]["image"], "first.png");
        assert_eq!(g[REFERENCE_NODES[1]]["inputs"]["image"], "second.png");
        for (node,value) in original.as_object().unwrap() {
            if ![PROMPT_NODE, SEED_NODE, SCHEDULER_NODE, LATENT_NODE, GUIDANCE_NODE, TURBO_NODE, REFERENCE_NODES[0], REFERENCE_NODES[1]].contains(&node.as_str()) {
                assert_eq!(&g[node],value,"Native node {node} changed");
            }
        }
    }
    #[test]
    fn invalid_inputs_rejected_before_network() {
        for (key, value) in [
            ("referenceImages", json!([])),
            ("referenceImages", json!([{"mediaId":"a"}])),
            ("referenceImages", json!([{"mediaId":"a"},{"mediaId":"b"},{"mediaId":"c"}])),
            ("referenceImages", json!([{"mediaId":"a"},{"mediaId":"a"}])),
            ("prompt", json!("")),
            ("seed", json!(-1)),
            ("width", json!(513)),
            ("height", json!(0)),
            ("advanced", json!({"steps":51})),
            ("advanced", json!({"steps":12,"guidance":4,"enableTurbo":true,"hidden":1})),
            ("advanced", json!({"steps":12,"guidance":101,"enableTurbo":true})),
            ("advanced", json!({"steps":12,"guidance":4,"enableTurbo":"true"})),
            ("negativePrompt", json!("unsupported")),
        ] {
            let mut bad = input();
            bad[key] = value;
            assert!(build_graph(&bad, &["scene.png", "reference.png"]).is_err(), "{key}");
        }
        let mut bad = input();
        bad["unknown"] = json!(1);
        assert!(build_graph(&bad, &["scene.png", "reference.png"]).is_err());
    }
    #[test]
    fn missing_original_never_becomes_single_image_or_text_to_image() {
        let mut value = input();
        value.as_object_mut().unwrap().remove("referenceImages");
        assert!(build_graph(&value, &["scene.png", "reference.png"]).is_err());
    }
    #[test]
    fn reference_order_and_complete_lease_set_are_preserved() {
        let mut value = input();
        let media = json!([
            {"id":"original-1","kind":"image","fileName":"scene.png","handle":"lease1","mime":"image/png"},
            {"id":"original-2","kind":"image","fileName":"reference.png","handle":"lease2","mime":"image/png"}]);
        let ids = reference_ids(&value).ok().unwrap();
        let p = json!({"media":media});
        let leases = reference_media(&p,&ids).ok().unwrap();
        assert_eq!(leases[0]["handle"], "lease1");
        assert_eq!(leases[1]["handle"], "lease2");
        value["referenceImages"].as_array_mut().unwrap().swap(0,1);
        let ids = reference_ids(&value).ok().unwrap();
        let leases = reference_media(&p,&ids).ok().unwrap();
        assert_eq!(leases[0]["handle"], "lease2");
        assert_eq!(leases[1]["handle"], "lease1");
        assert!(reference_media(&json!({"media":[p["media"][0]]}),&ids).is_err());
        assert!(reference_media(&json!({"media":[p["media"][0],p["media"][0]]}),&ids).is_err());
    }
    #[test]
    fn png_header_matches_real_dimensions() {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&768u32.to_be_bytes());
        b.extend_from_slice(&512u32.to_be_bytes());
        assert_eq!(png_dims(&b), Some((768, 512)));
        assert_eq!(png_dims(&[0xFF, 0xD8, 0xFF]), None);
    }
}
