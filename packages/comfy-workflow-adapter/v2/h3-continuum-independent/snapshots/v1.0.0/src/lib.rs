#![cfg_attr(target_arch = "wasm32", no_std)]
//! One declared H3 two-reference plus original soundtrack profile. Platform graph, frame alignment and HTTP remain in this author package.
//! Uses the native ComfyUI API export; provenance is in SOURCE.json.
//! Platform/node/HTTP semantics stay inside this package. No application source is required.
extern crate alloc;
use alloc::{format, string::{String, ToString}, vec, vec::Vec};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PLUGIN_ID: &str = "local.independent.comfy-h3-continuum";
const MODEL_ID: &str = "h3-continuum-driving-audio";
const OPERATION: &str = "local.independent.comfy-h3-continuum/continuum-driving-audio";
/// Frozen native API prompt graph; export hashes are recorded in SOURCE.json.
const GRAPH: &str = include_str!("workflow-api.json");
const PROMPT_NODE: &str = "5";
const SEED_NODE: &str = "15";
const OUTPUT_NODE: &str = "20";
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
        "schemaVersion":2,"localId":"continuum-driving-audio","capabilityKey":"video.generate","transportSlot":"video","modelIds":[MODEL_ID],
        "configurationSchema":{"id":"config","digest":config["digest"]},
        "operations":[{"id":OPERATION,"version":1,"label":"H3 Continuum image + driving audio (512x512,24fps,one 5second chunk)",
            "inputSchema":{"id":"input","digest":input["digest"]},"outputSchema":{"id":"output","digest":output["digest"]},
            "resumeSchema":{"id":"resume","digest":resume["digest"]},
            "uiSchema":{"controls":[
                {"fieldPointer":"/referenceImages","widget":"media","label":"One original used as first frame AND identity reference","description":"Same original feeds native first_frame and reference_image_1","group":"Input"},
                {"fieldPointer":"/referenceAudios","widget":"media","label":"Required complete5second WAV (driving condition AND soundtrack)","description":"Whole original from0, no trim or substitute","group":"Input"},
                {"fieldPointer":"/advanced/audioUsage","widget":"select","label":"原音参与生成条件并复用为成片音轨","description":"从0秒，完整5秒，原生重编码","group":"Advanced"},
                {"fieldPointer":"/prompt","widget":"multiline","label":"Prompt","description":"","group":"Input"},
                {"fieldPointer":"/seed","widget":"number","label":"Seed","description":"","group":"Input"},
                {"fieldPointer":"/width","widget":"number","label":"Width","description":"","group":"Input"},
                {"fieldPointer":"/height","widget":"number","label":"Height","description":"","group":"Input"},
                {"fieldPointer":"/durationSeconds","widget":"number","label":"Duration seconds (one5second chunk at24fps)","description":"","group":"Input"},
                {"fieldPointer":"/aspectRatio","widget":"select","label":"Aspect ratio","description":"1:1,512x512","group":"Input"},
                {"fieldPointer":"/fps","widget":"number","label":"FPS","description":"","group":"Input"},
                {"fieldPointer":"/advanced/chunks","widget":"number","label":"段数（本包固定1）","description":"Only one chunk validated","group":"Advanced"},
                {"fieldPointer":"/advanced/chunkSeconds","widget":"number","label":"每段秒数（本包固定5）","description":"No audio trim or loop","group":"Advanced"},
                {"fieldPointer":"/advanced/runName","widget":"text","label":"原生运行存储名称（新生成使用新名称）","description":"Explicit native Save + Auto Resume namespace","group":"Advanced"},
                {"fieldPointer":"/advanced/steps","widget":"number","label":"采样步数（Continuum单段，5秒成片）","description":"","group":"Advanced"}]},
            "mediaSlots":[{"role":"referenceImages","inputPointer":"/referenceImages","mediaKind":"image","minCount":1,"maxCount":1},{"role":"referenceAudios","inputPointer":"/referenceAudios","mediaKind":"audio","minCount":1,"maxCount":1}],
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
fn build_graph(input: &Value, references: &[&str], audio: &str) -> Outcome<Value> {
    let obj=input.as_object().ok_or_else(||fail("INVALID_INPUT","input must be object"))?;
    if obj.keys().any(|k| !["prompt","seed","width","height","durationSeconds","fps","advanced","aspectRatio","referenceImages","referenceAudios"].contains(&k.as_str())) {return Err(fail("INVALID_INPUT","unknown video input"));}
    let prompt=text(input,"prompt")?;
    if prompt.trim().is_empty() || prompt.chars().count()>MAX_PROMPT {return Err(fail("INVALID_INPUT","prompt must be1..8000 characters"));}
    let seed=number(input,"seed")?;if seed>MAX_SEED{return Err(fail("INVALID_INPUT","seed exceeds2^53-1"));}
    if input["aspectRatio"]!="1:1" || number(input,"width")?!=512 || number(input,"height")?!=512 || number(input,"fps")?!=24 || number(input,"durationSeconds")?!=5 {return Err(fail("INVALID_INPUT","Declared profile requires512x512,24fps,5seconds"));}
    reference_ids(input)?; audio_id(input)?;
    if audio.is_empty() || references.len()!=1 || references[0].is_empty() { return Err(fail("INVALID_INPUT","Exactly one uploaded image and one full audio required")); }
    let a=input["advanced"].as_object().ok_or_else(||fail("INVALID_INPUT","advanced required"))?;
    let steps=number(&input["advanced"],"steps")?;
    let chunks=number(&input["advanced"],"chunks")?;
    let seconds=number(&input["advanced"],"chunkSeconds")?;
    let run=text(&input["advanced"],"runName")?;
    if a.len()!=5 || a.get("audioUsage").and_then(Value::as_str)!=Some("soundtrack_copy") || steps!=6 || chunks!=1 || seconds!=5 || run.trim().is_empty() || run.chars().count()>128 {return Err(fail("INVALID_INPUT","Explicit profile requires6steps,one5second chunk,soundtrack_copy and runName1..128chars"));}
    if chunks.checked_mul(seconds)!=Some(number(input,"durationSeconds")?) {return Err(fail("INVALID_INPUT","Duration must equal chunks multiplied by chunkSeconds"));}
    let mut g:Value=serde_json::from_str(GRAPH).expect("native API graph");
    g[PROMPT_NODE]["inputs"]["value"]=json!(prompt);
    g["6"]["inputs"]["value"]=input["width"].clone();g["7"]["inputs"]["value"]=input["height"].clone();
    g[SEED_NODE]["inputs"]["base_seed"]=json!(seed);
    g["15"]["inputs"]["chunks"]=json!(chunks);g["15"]["inputs"]["chunk_seconds"]=json!(seconds);
    g["15"]["inputs"]["run_name"]=json!(run);
    g["14"]["inputs"]["steps"]=json!(steps);
    g["19"]["inputs"]["fps"]=input["fps"].clone();
    g["3"]["inputs"]["image"]=json!(references[0]);g["4"]["inputs"]["audio"]=json!(audio);
    // No native condition/decoder/assembler edges are rewritten.
    Ok(g)
}
fn reference_ids(input:&Value)->Outcome<Vec<&str>> {
    let refs=input["referenceImages"].as_array().filter(|r|r.len()==1).ok_or_else(||fail("INVALID_INPUT","Exactly one original image required"))?;
    refs.iter().map(|r|{
        let o=r.as_object().filter(|o|o.len()==1).ok_or_else(||fail("INVALID_INPUT","Reference must contain only mediaId"))?;
        o.get("mediaId").and_then(Value::as_str).filter(|s|!s.is_empty()).ok_or_else(||fail("INVALID_INPUT","Reference mediaId required"))
    }).collect()
}
fn audio_id(input:&Value)->Outcome<&str> {
    input["referenceAudios"].as_array().filter(|a|a.len()==1).and_then(|a|a[0].as_object())
        .filter(|a|a.len()==1).and_then(|a|a.get("mediaId")).and_then(Value::as_str).filter(|s|!s.is_empty())
        .ok_or_else(||fail("INVALID_INPUT","Exactly one original soundtrack mediaId required"))
}
fn validate_audio_stat(stat:&Value)->Outcome<()> {
    if stat["kind"]!="audio" || stat["mime"]!="audio/x-wav" || stat["durationMs"].as_str()!=Some("5000") {
        return Err(fail("INVALID_INPUT","This profile requires one complete5second WAV from0; no clipping, looping or replacement"));
    }
    Ok(())
}
fn upload_reference(h: &mut dyn Host, p: &Value, id: &str, kind: &str, budget: u64) -> Outcome<String> {
    let media = p["media"]
        .as_array()
        .filter(|m| m.len() == 2)
        .and_then(|m| m.iter().find(|m| m["id"] == id && m["kind"] == kind))
        .ok_or_else(|| {
            fail(
                "INVALID_INPUT",
                "The referenced original has no matching media lease",
            )
        })?;
    let body = json!({"kind":"multipart","parts":[
        {"kind":"file","name":"image","fileName":text(media,"fileName")?,"handle":text(media,"handle")?,"mime":text(media,"mime")?},
        {"kind":"field","name":"type","value":"input"}]});
    let args = json!({"plan":{"method":"POST","url":format!("{}/upload/image",base(&p["configuration"])?),
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
            "ComfyUI rejected the original media upload",
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
    let mut graph = build_graph(&p["input"], &["pending-1"], "pending-audio")?;
    let audio_id=audio_id(&p["input"])?;
    let audio=p["media"].as_array().and_then(|m|m.iter().find(|m|m["id"]==audio_id && m["kind"]=="audio"))
        .ok_or_else(||fail("INVALID_INPUT","Required original audio lease is missing or has wrong kind"))?;
    let stat=port(h,"host-media.info",json!({"handle":text(audio,"handle")?}))?;
    validate_audio_stat(&stat)?;
    let uploaded_audio=upload_reference(h,p,audio_id,"audio",request.r#budget_ms)?;
    graph["4"]["inputs"]["audio"]=json!(uploaded_audio);
    let ids=reference_ids(&p["input"])?;
    for id in &ids{
        let uploaded=upload_reference(h,p,id,"image",request.r#budget_ms)?;
        graph["3"]["inputs"]["image"]=json!(uploaded);
    }
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
pub struct ComfyContinuumDrivingAudio;
impl Adapter for ComfyContinuumDrivingAudio {
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
bgx_adapter_v2_sdk::export_adapter!(ComfyContinuumDrivingAudio);


#[cfg(test)]
mod tests;
