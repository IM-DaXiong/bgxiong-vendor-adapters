#![cfg_attr(target_arch = "wasm32", no_std)]
//! local.example.comfy-qwen-two-output: one frozen ComfyUI Qwen-Image-2.1 text-to-image operation producing two images.
//! Uses the native ComfyUI Export(API) graph; provenance is in SOURCE.json.
//! Platform/node/HTTP semantics stay inside this package. No application source is required.
extern crate alloc;
use alloc::{format, string::{String, ToString}, vec, vec::Vec};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PLUGIN_ID: &str = "local.example.comfy-qwen-two-output";
const MODEL_ID: &str = "qwen-image-2.1-two-output";
const OPERATION: &str = "local.example.comfy-qwen-two-output/qwen-two-output";
/// Frozen API prompt graph (native Export(API) SHA-256 recorded in SOURCE.json).
const GRAPH: &str = include_str!("workflow-api.json");
const PROMPT_NODE: &str = "459:474"; // ComfySwitchNode(switch=false).on_false feeds the text encoder
const PROMPT_KEY: &str = "on_false";
const REWRITE_NODE: &str = "459:471"; // TextGenerate prompt kept identical (disabled branch)
const SEED_NODE: &str = "459:458"; // KSampler
const OUTPUT_NODE: &str = "461"; // SaveImageAdvanced
const MAX_PROMPT: usize = 8000;
const MAX_SEED: u64 = 9007199254740991;
const INPUT_SCHEMA: &str = include_str!("../schemas/input.json");
const OUTPUT_SCHEMA: &str = r#"{"type":"object","properties":{"description":{"type":"string"}},"required":["description"],"additionalProperties":false}"#;
const CONFIG_SCHEMA: &str = r#"{"type":"object","properties":{"endpointBaseUrl":{"type":"string","minLength":1}},"required":["endpointBaseUrl"],"additionalProperties":false}"#;
const RESUME_SCHEMA: &str = include_str!("../schemas/resume.json");

type Outcome<T> = Result<T, Failure>;
struct Failure { code: &'static str, message: &'static str, vendor: Option<String>, request: Option<String> }
fn fail(code: &'static str, message: &'static str) -> Failure { Failure { code, message, vendor: None, request: None } }
fn malformed() -> Failure { fail("UPSTREAM_ERROR", "ComfyUI returned an unexpected structured response") }
fn text<'a>(v: &'a Value, key: &str) -> Outcome<&'a str> { v.get(key).and_then(Value::as_str).ok_or_else(malformed) }
fn number(v: &Value, key: &str) -> Outcome<u64> { v.get(key).and_then(Value::as_u64).ok_or_else(malformed) }
fn policy(_name: &str) -> u64 { 1_048_576 } // protocol/schema-policy.json limits.maxBufferedHttpBytes
fn port(h: &mut dyn Host, method: &str, args: Value) -> Outcome<Value> {
    host::call(h, method, &args).map_err(|e| {
        let v = serde_json::to_value(&e).expect("public host error");
        let code = match v["code"].as_str() {
            Some("TIMEOUT") => "TIMEOUT",
            Some("INVALID_INPUT") => "INVALID_INPUT",
            Some("PERMISSION_DENIED") => "PERMISSION_DENIED",
            _ => "RESOURCE_UNAVAILABLE",
        };
        fail(code, "A managed host operation failed; no retry was attempted")
    })
}
fn schema(id: &str, source: &str) -> Value {
    let digest: String = Sha256::digest(source.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    json!({"id":id,"digest":digest,"schemaJson":source})
}
fn capabilities() -> Value {
    let input = schema("input", INPUT_SCHEMA);
    let output = schema("output", OUTPUT_SCHEMA);
    let config = schema("config", CONFIG_SCHEMA);
    let resume = schema("resume", RESUME_SCHEMA);
    json!({"schemaVersion":2,"pluginId":PLUGIN_ID,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[{
        "schemaVersion":2,"localId":"qwen-two-output","capabilityKey":"image.generate","transportSlot":"image","modelIds":[MODEL_ID],
        "configurationSchema":{"id":"config","digest":config["digest"]},
        "operations":[{"id":OPERATION,"version":1,"label":"Qwen Image 2.1 two images",
            "inputSchema":{"id":"input","digest":input["digest"]},"outputSchema":{"id":"output","digest":output["digest"]},
            "resumeSchema":{"id":"resume","digest":resume["digest"]},
            "uiSchema":{"controls":[
                {"fieldPointer":"/prompt","widget":"multiline","label":"Prompt","description":"","group":"Input"},
                {"fieldPointer":"/seed","widget":"number","label":"Seed","description":"","group":"Input"},
                {"fieldPointer":"/width","widget":"number","label":"Width","description":"","group":"Input"},
                {"fieldPointer":"/height","widget":"number","label":"Height","description":"","group":"Input"},
                {"fieldPointer":"/negativePrompt","widget":"multiline","label":"Negative prompt","description":"","group":"Input"},
                {"fieldPointer":"/advanced/steps","widget":"number","label":"Steps","description":"","group":"Advanced"}]},
            "mediaSlots":[],
            "execution":{"mode":"asynchronous","streaming":false,"cancellation":"unsupported","resumable":true,"remoteIdempotency":false},
            "outputSlots":[{"role":"artifact","kind":"image","minCount":2,"maxCount":2}]}],
        "schemas":[input,output,config,resume]
    }]})
}
fn base(config: &Value) -> Outcome<String> {
    let b = text(config, "endpointBaseUrl").map_err(|_| fail("INVALID_INPUT", "configuration.endpointBaseUrl is required"))?;
    if !(b.starts_with("http://") || b.starts_with("https://")) || b.ends_with('/') {
        return Err(fail("INVALID_INPUT", "endpointBaseUrl must be an absolute http(s) origin without trailing slash"));
    }
    Ok(b.to_string())
}
fn http(h: &mut dyn Host, method: &str, url: String, body: Option<&Value>, sink: Value, budget: u64) -> Outcome<Value> {
    let (body, headers) = match body {
        Some(v) => (json!({"kind":"bytes","bytes":serde_json::to_vec(v).map_err(|_| malformed())?}), vec![json!({"name":"Content-Type","value":"application/json"})]),
        None => (json!({"kind":"none"}), vec![]),
    };
    port(h, "host-http.execute", json!({"plan":{"method":method,"url":url,"headers":headers,"generatedHeaders":[],"body":body,"sink":sink,"timeoutMs":budget},"credential":null}))
}
fn buffered_json(response: &Value) -> Option<Value> {
    let bytes: Vec<u8> = serde_json::from_value(response["payload"]["bytes"].clone()).ok()?;
    serde_json::from_slice(&bytes).ok()
}
fn get_json(h: &mut dyn Host, url: String, budget: u64) -> Outcome<Value> {
    let r = http(h, "GET", url, None, json!({"kind":"buffer","maxBytes":policy("maxBufferedHttpBytes")}), budget)?;
    let status = number(&r, "status")?;
    let v = buffered_json(&r).ok_or_else(malformed)?;
    if status != 200 {
        return Err(Failure { code: "UPSTREAM_ERROR", message: "ComfyUI rejected the read request", vendor: Some(format!("HTTP_{status}")), request: None });
    }
    Ok(v)
}
fn probe(h: &mut dyn Host, p: &Value, budget: u64) -> Outcome<Value> {
    match text(p, "kind")? {
        "package_validation" => Ok(json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]})),
        "connectivity" => {
            let stats = get_json(h, format!("{}/system_stats", base(&p["configuration"])?), budget)?;
            if !stats["system"]["comfyui_version"].is_string() { return Err(malformed()); }
            Ok(json!({"packageValid":true,"connection":"ready","authentication":"not_checked","resources":"not_checked","issues":[]}))
        }
        _ => Err(fail("INVALID_INPUT", "Unknown probe kind")),
    }
}
/// Validates all mapped inputs before any network effect.
fn build_graph(input: &Value) -> Outcome<Value> {
    let obj = input.as_object().ok_or_else(|| fail("INVALID_INPUT", "input must be an object"))?;
    if obj.keys().any(|k| !["prompt","seed","width","height","negativePrompt","advanced"].contains(&k.as_str())) { return Err(fail("INVALID_INPUT", "unknown image input")); }
    let prompt = obj.get("prompt").and_then(Value::as_str).ok_or_else(|| fail("INVALID_INPUT", "prompt must be a string"))?;
    if prompt.trim().is_empty() || prompt.chars().count() > MAX_PROMPT { return Err(fail("INVALID_INPUT", "prompt must be 1..8000 characters")); }
    let seed = obj.get("seed").and_then(Value::as_u64).ok_or_else(|| fail("INVALID_INPUT", "seed must be a non-negative integer"))?;
    if seed > MAX_SEED { return Err(fail("INVALID_INPUT", "seed exceeds 2^53-1")); }
    let width = number(input, "width").map_err(|_| fail("INVALID_INPUT", "width is required"))?;
    let height = number(input, "height").map_err(|_| fail("INVALID_INPUT", "height is required"))?;
    if [width,height].iter().any(|v| *v < 256 || *v > 4096 || *v % 8 != 0) { return Err(fail("INVALID_INPUT", "dimensions must be 256..4096, multiple of 8")); }
    let advanced = input["advanced"].as_object().ok_or_else(|| fail("INVALID_INPUT", "advanced.steps is required"))?;
    let steps = advanced.get("steps").and_then(Value::as_u64).ok_or_else(|| fail("INVALID_INPUT", "steps must be an integer"))?;
    if advanced.len() != 1 || !(1..=50).contains(&steps) { return Err(fail("INVALID_INPUT", "advanced must contain steps in 1..50")); }
    let negative = match input.get("negativePrompt") { None => "", Some(v) => v.as_str().ok_or_else(|| fail("INVALID_INPUT", "negativePrompt must be a string"))? };
    if negative.chars().count() > MAX_PROMPT { return Err(fail("INVALID_INPUT", "negativePrompt exceeds 8000 characters")); }
    let mut g: Value = serde_json::from_str(GRAPH).expect("frozen graph");
    for (node, key) in [(PROMPT_NODE, PROMPT_KEY), (REWRITE_NODE, "prompt"), (SEED_NODE, "seed")] {
        if g[node]["inputs"].get(key).is_none() { return Err(fail("INTERNAL_ERROR", "frozen graph is missing a mapped input")); }
    }
    g[PROMPT_NODE]["inputs"][PROMPT_KEY] = json!(prompt);
    g[REWRITE_NODE]["inputs"]["prompt"] = json!(prompt);
    g[SEED_NODE]["inputs"]["seed"] = json!(seed);
    g["459:456"]["inputs"]["width"] = json!(width);
    g["459:456"]["inputs"]["height"] = json!(height);
    g[SEED_NODE]["inputs"]["steps"] = json!(steps);
    g["459:452"]["inputs"]["negative_prompt"] = json!(negative);
    Ok(g)
}
fn persist_accepted(h: &mut dyn Host, request: &Invocation, accepted: &Value) -> Outcome<()> {
    let read = port(h, "host-checkpoint.read", json!({"requestId":request.r#request_id}))?;
    let mut checkpoint: Value = serde_json::from_str(text(&read, "valueJson")?).map_err(|_| malformed())?;
    let old = text(&read, "revision")?.to_string();
    let revision = old.parse::<u64>().ok().and_then(|n| n.checked_add(1)).ok_or_else(malformed)?;
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
    if text(p, "operationId")? != OPERATION || text(p, "modelId")? != MODEL_ID { return Err(fail("INVALID_INPUT", "Unknown operation or model")); }
    let graph = build_graph(&p["input"])?;
    let url = format!("{}/prompt", base(&p["configuration"])?);
    let body = json!({"prompt":graph,"client_id":request.r#request_id});
    // Exactly one POST; any transport loss or unreadable reply is ACCEPTANCE_UNKNOWN (never resubmitted).
    let r = http(h, "POST", url, Some(&body), json!({"kind":"buffer","maxBytes":policy("maxBufferedHttpBytes")}), request.r#budget_ms)
        .map_err(|_| fail("ACCEPTANCE_UNKNOWN", "Submit response was lost or interrupted; do not resubmit"))?;
    let status = number(&r, "status").map_err(|_| fail("ACCEPTANCE_UNKNOWN", "Submit response is incomplete; do not resubmit"))?;
    let v = buffered_json(&r).ok_or_else(|| fail("ACCEPTANCE_UNKNOWN", "Submit response is not JSON; do not resubmit"))?;
    if status != 200 {
        let vendor = v["error"]["type"].as_str().map(str::to_string);
        return Err(Failure { code: if (400..500).contains(&status) { "INVALID_INPUT" } else { "UPSTREAM_ERROR" }, message: "ComfyUI rejected the prompt", vendor, request: None });
    }
    let id = v.get("prompt_id").and_then(Value::as_str).filter(|s| !s.is_empty())
        .ok_or_else(|| fail("ACCEPTANCE_UNKNOWN", "Submit reply lacks prompt_id; do not resubmit"))?;
    if v.get("node_errors").and_then(Value::as_object).is_some_and(|o| !o.is_empty()) { return Err(malformed()); }
    let accepted = json!({"kind":"accepted","vendorTaskId":id,"appliedParams":null,"resumeToken":{"requestId":request.r#request_id,"width":p["input"]["width"],"height":p["input"]["height"]},"usage":null});
    persist_accepted(h, request, &accepted)?;
    Ok(accepted)
}
fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { o.push(b as char) } else { o.push_str(&format!("%{b:02X}")) }
    }
    o
}
fn query_result(status: &str, outputs: Vec<Value>, description: String, token: Value, vendor: Option<String>, details: Value) -> Value {
    let failure = match status {
        "failed" => json!({"category":"vendorTaskFailed","message":"The ComfyUI prompt failed","vendorCode":vendor,"details":details}),
        _ => Value::Null,
    };
    json!({"status":status,"outputs":outputs,"data":{"description":description},"usage":null,
        "progress":if matches!(status,"queued"|"running"){json!({"fraction":null,"message":status})}else{Value::Null},
        "terminalFailure":failure,"resumeToken":token})
}
fn in_queue(list: &Value, id: &str) -> bool {
    list.as_array().is_some_and(|a| a.iter().any(|e| e.get(1).and_then(Value::as_str) == Some(id)))
}
/// PNG signature + IHDR width/height from the real downloaded bytes (not from the file name).
fn png_dims(bytes: &[u8]) -> Option<(u64, u64)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIG || &bytes[12..16] != b"IHDR" { return None; }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as u64;
    let hgt = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as u64;
    Some((w, hgt))
}
fn download(h: &mut dyn Host, base: &str, image: &Value, budget: u64, width: u64, height: u64) -> Outcome<Value> {
    if image["type"] != "output" { return Err(fail("UPSTREAM_ERROR", "Selected output is not a saved output image")); }
    let name = text(image, "filename")?;
    let sub = image.get("subfolder").and_then(Value::as_str).unwrap_or("");
    let url = format!("{base}/view?filename={}&subfolder={}&type=output", pct(name), pct(sub));
    let r = http(h, "GET", url, None, json!({"kind":"media"}), budget)?;
    let status = number(&r, "status")?;
    if status != 200 {
        return Err(Failure { code: "UPSTREAM_ERROR", message: "ComfyUI did not return the output image", vendor: Some(format!("HTTP_{status}")), request: None });
    }
    let handle = text(&r["payload"], "handle")?.to_string();
    let stat = port(h, "host-media.info", json!({"handle":handle}))?;
    let head: Vec<u8> = serde_json::from_value(port(h, "host-media.read", json!({"handle":handle,"offset":"0","maxBytes":32}))?).map_err(|_| malformed())?;
    let Some((w, hgt)) = png_dims(&head) else { return Err(fail("UPSTREAM_ERROR", "Downloaded output is not a PNG image")); };
    if (w, hgt) != (width, height) { return Err(fail("UPSTREAM_ERROR", "Downloaded image size differs from the accepted input dimensions")); }
    if stat["kind"] != "image" || stat["byteLength"] == "0" { return Err(fail("UPSTREAM_ERROR", "Downloaded media is not a non-empty image")); }
    Ok(json!({"kind":"media","role":"artifact","media":{"id":stat["id"],"kind":stat["kind"],"handle":handle,"mime":stat["mime"],"fileName":stat["fileName"],"byteLength":stat["byteLength"]}}))
}
fn output_images(entry: &Value) -> Outcome<Vec<Value>> {
    let images = entry["outputs"][OUTPUT_NODE]["images"].as_array().cloned().unwrap_or_default();
    if images.len() != 2 { return Err(fail("UPSTREAM_ERROR", "Expected exactly two saved images")); }
    let identity = |v: &Value| -> Outcome<(String,String)> {
        if v["type"] != "output" { return Err(malformed()); }
        let name=text(v,"filename")?;
        if name.is_empty() { return Err(malformed()); }
        Ok((name.to_string(),v["subfolder"].as_str().unwrap_or("").to_string()))
    };
    if identity(&images[0])? == identity(&images[1])? {
        return Err(fail("UPSTREAM_ERROR", "Output images must have distinct saved identities"));
    }
    Ok(images)
}
fn query(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Value> {
    if text(p, "operationId")? != OPERATION { return Err(fail("INVALID_INPUT", "Unknown operation")); }
    let base = base(&p["configuration"])?;
    let id = text(p, "vendorTaskId")?.to_string();
    let budget = request.r#budget_ms;
    let history = get_json(h, format!("{base}/history/{}", pct(&id)), budget)?;
    let token = p["resumeToken"].clone();
    let Some(entry) = history.get(&id) else {
        let q = get_json(h, format!("{base}/queue"), budget)?;
        let status = if in_queue(&q["queue_running"], &id) { "running" } else if in_queue(&q["queue_pending"], &id) { "queued" } else {
            return Err(Failure { code: "UPSTREAM_ERROR", message: "Prompt is neither in history nor in the queue", vendor: Some("PROMPT_NOT_FOUND".into()), request: Some(id) });
        };
        return Ok(query_result(status, vec![], format!("ComfyUI prompt {id}: {status}"), token, None, json!({"promptId":id})));
    };
    let st = text(&entry["status"], "status_str")?;
    match st {
        "success" => {
            if entry["status"]["completed"] != true { return Err(malformed()); }
            let images = output_images(entry)?;
            let media = images.iter().map(|image| download(h, &base, image, budget, number(&token,"width")?, number(&token,"height")?)).collect::<Outcome<Vec<_>>>()?;
            Ok(query_result("succeeded", media, format!("ComfyUI prompt {id}: success"), token, None, json!({"promptId":id,"outputNode":OUTPUT_NODE})))
        }
        "error" => {
            let vendor = entry["status"]["messages"].as_array().and_then(|m| m.iter().find(|e| e.get(0) == Some(&json!("execution_error"))))
                .and_then(|e| e[1]["exception_type"].as_str()).map(str::to_string).or(Some("execution_error".to_string()));
            Ok(query_result("failed", vec![], format!("ComfyUI prompt {id}: error"), token, vendor, json!({"promptId":id})))
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
pub struct ComfyT2i;
impl Adapter for ComfyT2i {
    fn invoke(request: Invocation, h: &mut dyn Host) -> Response {
        let value = serde_json::to_value(&request).expect("typed invocation");
        let result = (|| {
            let p: Value = serde_json::from_str(&request.r#payload_json).map_err(|_| fail("INVALID_INPUT", "Payload is not JSON"))?;
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
bgx_adapter_v2_sdk::export_adapter!(ComfyT2i);

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Value { json!({"prompt":"a red apple","seed":7,"width":768,"height":512,"negativePrompt":"blurry","advanced":{"steps":12}}) }
    #[test]
    fn graph_maps_confirmed_image_inputs() {
        let original: Value=serde_json::from_str(GRAPH).unwrap(); let g=build_graph(&input()).ok().unwrap();
        assert_eq!(g[PROMPT_NODE]["inputs"][PROMPT_KEY],"a red apple");
        assert_eq!(g[SEED_NODE]["inputs"]["seed"],7); assert_eq!(g[SEED_NODE]["inputs"]["steps"],12);
        assert_eq!(g["459:456"]["inputs"]["width"],768); assert_eq!(g["459:456"]["inputs"]["height"],512);
        assert_eq!(g["459:452"]["inputs"]["negative_prompt"],"blurry");
        assert_eq!(g["459:456"]["inputs"]["batch_size"],2);
        for node in ["459:451","459:453","459:454",OUTPUT_NODE] { assert_eq!(g[node],original[node]); }
    }
    #[test]
    fn invalid_inputs_rejected_before_network() {
        for (key,value) in [("prompt",json!("")),("seed",json!(-1)),("width",json!(513)),("height",json!(0)),("advanced",json!({"steps":51})),("advanced",json!({"steps":12,"hidden":1}))] {
            let mut bad=input();bad[key]=value;assert!(build_graph(&bad).is_err(),"{key}");
        }
        let mut bad=input();bad["unknown"]=json!(1);assert!(build_graph(&bad).is_err());
    }
    #[test]
    fn two_outputs_preserve_order_and_reject_partial_extra_or_duplicate_results() {
        let image=|n:&str| json!({"filename":n,"subfolder":"","type":"output"});
        let entry=|images:Vec<Value>| json!({"outputs":{OUTPUT_NODE:{"images":images}}});
        let ordered=vec![image("second.png"),image("first.png")];
        assert_eq!(output_images(&entry(ordered.clone())).ok().unwrap(),ordered);
        for images in [vec![],vec![image("a.png")],vec![image("a.png"),image("b.png"),image("c.png")],vec![image("a.png"),image("a.png")]] {
            assert!(output_images(&entry(images)).is_err());
        }
    }
    #[test]
    fn png_header_matches_real_dimensions() {
        let mut b=vec![0x89,b'P',b'N',b'G',0x0D,0x0A,0x1A,0x0A,0,0,0,13];
        b.extend_from_slice(b"IHDR");b.extend_from_slice(&768u32.to_be_bytes());b.extend_from_slice(&512u32.to_be_bytes());
        assert_eq!(png_dims(&b),Some((768,512)));assert_eq!(png_dims(&[0xFF,0xD8,0xFF]),None);
    }
}
