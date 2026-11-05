#![cfg_attr(target_arch = "wasm32", no_std)]
//! One declared H3 T2V profile. Platform graph, frame alignment and HTTP remain in this author package.
//! Uses the native ComfyUI API export; provenance is in SOURCE.json.
//! Platform/node/HTTP semantics stay inside this package. No application source is required.
extern crate alloc;
use alloc::{format, string::{String, ToString}, vec, vec::Vec};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PLUGIN_ID: &str = "local.example.comfy-h3-t2v";
const MODEL_ID: &str = "minimax-h3-t2v";
const OPERATION: &str = "local.example.comfy-h3-t2v/h3-t2v";
/// Frozen native API prompt graph; export hashes are recorded in SOURCE.json.
const GRAPH: &str = include_str!("workflow-api.json");
const PROMPT_NODE: &str = "105:104";
const SEED_NODE: &str = "105:15";
const OUTPUT_NODE: &str = "92";
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
        "schemaVersion":2,"localId":"h3-t2v","capabilityKey":"video.generate","transportSlot":"video","modelIds":[MODEL_ID],
        "configurationSchema":{"id":"config","digest":config["digest"]},
        "operations":[{"id":OPERATION,"version":1,"label":"H3 text to video (864x480, 24fps, 8s, generated audio)",
            "inputSchema":{"id":"input","digest":input["digest"]},"outputSchema":{"id":"output","digest":output["digest"]},
            "resumeSchema":{"id":"resume","digest":resume["digest"]},
            "uiSchema":{"controls":[
                {"fieldPointer":"/prompt","widget":"multiline","label":"Prompt","description":"","group":"Input"},
                {"fieldPointer":"/seed","widget":"number","label":"Seed","description":"","group":"Input"},
                {"fieldPointer":"/width","widget":"number","label":"Width","description":"","group":"Input"},
                {"fieldPointer":"/height","widget":"number","label":"Height","description":"","group":"Input"},
                {"fieldPointer":"/durationSeconds","widget":"number","label":"Duration seconds (192 frames at 24fps)","description":"","group":"Input"},
                {"fieldPointer":"/aspectRatio","widget":"select","label":"Aspect ratio","description":"Native 16:9 rounded to 32 pixels: 864x480","group":"Input"},
                {"fieldPointer":"/fps","widget":"number","label":"FPS","description":"","group":"Input"},
                {"fieldPointer":"/advanced/steps","widget":"number","label":"Steps","description":"","group":"Advanced"}]},
            "mediaSlots":[],
            "execution":{"mode":"asynchronous","streaming":false,"cancellation":"unsupported","resumable":true,"remoteIdempotency":false},
            "outputSlots":[{"role":"video","kind":"video","minCount":1,"maxCount":1}]}],
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
    if obj.keys().any(|k| !["prompt","seed","width","height","durationSeconds","fps","advanced","aspectRatio"].contains(&k.as_str())) { return Err(fail("INVALID_INPUT", "unknown video input")); }
    let prompt = text(input,"prompt").map_err(|_|fail("INVALID_INPUT","prompt is required"))?;
    if prompt.trim().is_empty() || prompt.chars().count()>MAX_PROMPT { return Err(fail("INVALID_INPUT","prompt must be 1..8000 characters")); }
    let seed=number(input,"seed").map_err(|_|fail("INVALID_INPUT","seed is required"))?;
    if seed>MAX_SEED { return Err(fail("INVALID_INPUT","seed exceeds 2^53-1")); }
    if input["aspectRatio"]!="16:9" || number(input,"width")?!=864 || number(input,"height")?!=480 || number(input,"fps")?!=24 || number(input,"durationSeconds")?!=8 {return Err(fail("INVALID_INPUT","This declared profile requires 864x480, 24fps and exactly 8 seconds"));}
    let advanced=input["advanced"].as_object().ok_or_else(||fail("INVALID_INPUT","advanced.steps is required"))?;
    let steps=number(&input["advanced"],"steps")?;
    if advanced.len()!=1 || !(1..=50).contains(&steps) {return Err(fail("INVALID_INPUT","steps must be 1..50 with no undeclared fields"));}
    let frames=number(input,"durationSeconds")?*number(input,"fps")?;
    if frames<5 || (frames-5)%17!=0 {return Err(fail("INVALID_INPUT","Duration does not align to the H3 17k+5 frame grid"));}
    let mut g:Value=serde_json::from_str(GRAPH).expect("native API graph");
    g[PROMPT_NODE]["inputs"]["prompt"]=json!(prompt);
    g[PROMPT_NODE]["inputs"]["width"]=input["width"].clone();
    g[PROMPT_NODE]["inputs"]["height"]=input["height"].clone();
    g[SEED_NODE]["inputs"]["noise_seed"]=json!(seed);
    g["105:9"]["inputs"]["steps"]=json!(steps);
    g["105:111"]["inputs"]["value"]=input["durationSeconds"].clone();
    g["105:91"]["inputs"]["fps"]=input["fps"].clone();
    // Keep the native Math Expression -> length and native generated audio -> CreateVideo intact.
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
fn download(h:&mut dyn Host, base:&str, output:&Value, budget:u64, width:u64, height:u64)->Outcome<Value> {
    if output["type"]!="output" {return Err(fail("UPSTREAM_ERROR","Expected a saved video output"));}
    let name=text(output,"filename")?;let sub=output["subfolder"].as_str().unwrap_or("");
    let r=http(h,"GET",format!("{base}/view?filename={}&subfolder={}&type=output",pct(name),pct(sub)),None,json!({"kind":"media"}),budget)?;
    if number(&r,"status")?!=200 {return Err(fail("UPSTREAM_ERROR","ComfyUI did not return the generated video"));}
    let handle=text(&r["payload"],"handle")?;
    let stat=port(h,"host-media.info",json!({"handle":handle}))?;
    if stat["kind"]!="video" || stat["mime"]!="video/mp4" || stat["byteLength"]=="0" {return Err(fail("UPSTREAM_ERROR","Output must be a non-empty MP4 video"));}
    if stat["width"].as_u64()!=Some(width) || stat["height"].as_u64()!=Some(height) {return Err(fail("UPSTREAM_ERROR","Video dimensions differ from the declared profile"));}
    Ok(json!({"kind":"media","role":"video","media":{"id":stat["id"],"kind":"video","handle":handle,"mime":stat["mime"],"fileName":stat["fileName"],"byteLength":stat["byteLength"]}}))
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
            let images = entry["outputs"][OUTPUT_NODE]["images"].as_array().cloned().unwrap_or_default();
            if images.len() != 1 {
                return Err(Failure { code: "UPSTREAM_ERROR", message: "Prompt completed without exactly one video on the frozen output node", vendor: Some("OUTPUT_MISSING".into()), request: Some(id) });
            }
            let media = download(h, &base, &images[0], budget, number(&token,"width")?, number(&token,"height")?)?;
            Ok(query_result("succeeded", vec![media], format!("ComfyUI prompt {id}: success"), token, None, json!({"promptId":id,"outputNode":OUTPUT_NODE})))
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
pub struct ComfyH3T2v;
impl Adapter for ComfyH3T2v {
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
bgx_adapter_v2_sdk::export_adapter!(ComfyH3T2v);

#[cfg(test)]
mod tests {
    use super::*;
    fn input()->Value {json!({"prompt":"A blue paper boat gently floating on a sunlit pond. Audio: quiet water ripples and soft birdsong.","seed":20261023,"width":864,"height":480,"durationSeconds":8,"fps":24,"aspectRatio":"16:9","advanced":{"steps":20}})}
    #[test]
    fn maps_profile_without_changing_models_audio_or_timing_links(){
        let original:Value=serde_json::from_str(GRAPH).unwrap();let g=build_graph(&input()).ok().unwrap();
        assert_eq!(g[PROMPT_NODE]["inputs"]["prompt"],input()["prompt"]);assert_eq!(g[PROMPT_NODE]["inputs"]["width"],864);assert_eq!(g[PROMPT_NODE]["inputs"]["height"],480);
        assert_eq!(g[SEED_NODE]["inputs"]["noise_seed"],20261023);assert_eq!(g["105:9"]["inputs"]["steps"],20);assert_eq!(g["105:111"]["inputs"]["value"],8);assert_eq!(8*24,17*11+5);
        for k in ["105:6","105:13","105:11","105:24","105:23","105:107",OUTPUT_NODE] {assert_eq!(g[k],original[k]);}
        assert_eq!(g[PROMPT_NODE]["inputs"]["length"],original[PROMPT_NODE]["inputs"]["length"]);assert_eq!(g["105:91"]["inputs"]["audio"],json!(["105:23",0]));
    }
    #[test]
    fn undeclared_duration_dimensions_and_media_fail_before_network(){
        for (key,v) in [("durationSeconds",json!(5)),("fps",json!(25)),("width",json!(768)),("height",json!(512)),("prompt",json!("")),("seed",json!(-1)),("referenceImages",json!([])),("advanced",json!({"steps":20,"hidden":1}))] {let mut x=input();x[key]=v;assert!(build_graph(&x).is_err(),"{key}");}
    }
    #[test]
    fn declared_catalog_profile_and_native_graph_agree(){
        let c=capabilities();let contract=&c["capabilityContracts"][0];assert_eq!(contract["capabilityKey"],"video.generate");assert_eq!(contract["operations"][0]["mediaSlots"],json!([]));assert_eq!(contract["operations"][0]["outputSlots"][0]["kind"],"video");
        let schema:Value=serde_json::from_str(INPUT_SCHEMA).unwrap();for k in ["width","height","durationSeconds","fps"] {assert_eq!(schema["properties"][k]["default"],input()[k]);assert_eq!(schema["properties"][k]["enum"],json!([input()[k]]));}
    }
}
