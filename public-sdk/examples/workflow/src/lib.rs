#![cfg_attr(target_arch = "wasm32", no_std)]
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
        "The fixture service returned an invalid structured response",
    )
}
fn text<'a>(v: &'a Value, key: &str) -> Outcome<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(malformed)
}
fn number(v: &Value, key: &str) -> Outcome<u64> {
    v.get(key).and_then(Value::as_u64).ok_or_else(malformed)
}
fn policy(name: &str) -> u64 {
    let p: Value = serde_json::from_str(include_str!("../../../protocol/schema-policy.json"))
        .expect("published SDK policy");
    p["limits"][name].as_u64().expect("published SDK budget")
}
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
fn shared_input() -> String {
    let examples: Value =
        serde_json::from_str(include_str!("../../capabilities.json")).expect("public examples");
    examples["capabilities"][0]["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "input")
        .unwrap()["schemaJson"]
        .as_str()
        .unwrap()
        .to_string()
}
fn output_schema() -> String {
    let examples: Value =
        serde_json::from_str(include_str!("../../capabilities.json")).expect("public examples");
    examples["capabilities"][0]["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "output")
        .unwrap()["schemaJson"]
        .as_str()
        .unwrap()
        .to_string()
}
fn source_input() -> String {
    let mut input: Value = serde_json::from_str(&shared_input()).unwrap();
    input["properties"]["source"] = json!({"type":"object","properties":{"mediaId":{"type":"string","minLength":1}},"required":["mediaId"],"additionalProperties":false});
    input.to_string()
}
fn contract(
    name: &str,
    key: &str,
    slot: &str,
    kind: Option<&str>,
    asynchronous: bool,
    streaming: bool,
    media_kind: Option<&str>,
    custom_output: Option<&str>,
) -> Value {
    let input = schema(
        "input",
        &if media_kind.is_some() {
            source_input()
        } else {
            shared_input()
        },
    );
    let output = schema(
        "output",
        &custom_output
            .map(str::to_string)
            .unwrap_or_else(output_schema),
    );
    let config = schema("config", &configuration_schema());
    let resume = schema(
        "resume",
        r#"{"type":"object","properties":{"requestId":{"type":"string","minLength":1}},"required":["requestId"],"additionalProperties":false}"#,
    );
    let mut controls = json!([{"fieldPointer":"/prompt","widget":"multiline","label":"Prompt","description":"","group":"Input"},{"fieldPointer":"/strength","widget":"number","label":"Strength","description":"","group":"Input"}]);
    let mut slots = json!([]);
    if let Some(kind) = media_kind {
        controls.as_array_mut().unwrap().push(json!({"fieldPointer":"/source","widget":"media","label":"Source","description":"Explicit broker media reference","group":"Input"}));
        slots = json!([{"role":"source","inputPointer":"/source","mediaKind":kind,"minCount":0,"maxCount":null}]);
    }
    let outputs = match kind {
        Some("text") => json!([{"role":"answer","kind":"text","minCount":1,"maxCount":null}]),
        Some(kind) => json!([{"role":"artifact","kind":kind,"minCount":1,"maxCount":null}]),
        None => json!([]),
    };
    let mut schemas = vec![input.clone(), output.clone(), config.clone()];
    if asynchronous {
        schemas.push(resume.clone());
    }
    json!({"schemaVersion":2,"localId":name,"capabilityKey":key,"transportSlot":slot,"modelIds":["fixture-model"],
        "configurationSchema":{"id":"config","digest":config["digest"]},
        "operations":[{"id":format!("{PLUGIN_ID}/{name}"),"version":1,"label":name,"inputSchema":{"id":"input","digest":input["digest"]},"outputSchema":{"id":"output","digest":output["digest"]},
        "resumeSchema":if asynchronous { json!({"id":"resume","digest":resume["digest"]}) } else { Value::Null },
        "uiSchema":{"controls":controls},"mediaSlots":slots,"outputSlots":outputs,
        "execution":{"mode":if asynchronous {"asynchronous"}else{"synchronous"},"streaming":streaming,"cancellation":if asynchronous {"remote"}else{"unsupported"},"resumable":asynchronous,"remoteIdempotency":false}}],"schemas":schemas})
}
fn plan(
    config: &Value,
    method: &str,
    url: String,
    body: Value,
    headers: Vec<Value>,
    sink: Value,
    budget: u64,
) -> Outcome<Value> {
    let credential = config.get("credentialRef").ok_or_else(|| {
        fail(
            "INVALID_INPUT",
            "Explicit credentialRef (null or reference) is required",
        )
    })?;
    let generated = match credential {
        Value::Null => vec![],
        Value::String(handle) if !handle.is_empty() => {
            vec![json!({"kind":"bearer","name":"Authorization","targetField":"Authorization"})]
        }
        _ => {
            return Err(fail(
                "INVALID_INPUT",
                "credentialRef must be null or a stable reference",
            ))
        }
    };
    Ok(
        json!({"plan":{"method":method,"url":url,"headers":headers,"generatedHeaders":generated,"body":body,"sink":sink,"timeoutMs":budget},"credential":credential}),
    )
}
fn status_error(status: u64, body: &Value) -> Failure {
    Failure {
        code: match status {
            401 | 403 => "UNAUTHORIZED",
            429 => "RATE_LIMITED",
            _ => "UPSTREAM_ERROR",
        },
        message: "The fixture service rejected the operation; no retry was attempted",
        vendor: body["error"]["code"].as_str().map(str::to_string),
        request: None,
    }
}
fn request_json(
    h: &mut dyn Host,
    config: &Value,
    method: &str,
    path: &str,
    body: Option<Value>,
    budget: u64,
    acceptance: bool,
) -> Outcome<Value> {
    let (body, headers) = match body {
        Some(value) => (
            json!({"kind":"bytes","bytes":serde_json::to_vec(&value).map_err(|_|malformed())?}),
            vec![json!({"name":"Content-Type","value":"application/json"})],
        ),
        None => (json!({"kind":"none"}), vec![]),
    };
    let args = plan(
        config,
        method,
        format!("{}{}", text(config, "endpointBaseUrl")?, path),
        body,
        headers,
        json!({"kind":"buffer","maxBytes":policy("maxBufferedHttpBytes")}),
        budget,
    )?;
    let response = port(h, "host-http.execute", args).map_err(|e| {
        if acceptance {
            fail(
                "ACCEPTANCE_UNKNOWN",
                "Submit response was lost or interrupted; do not resubmit",
            )
        } else {
            e
        }
    })?;
    let bytes: Vec<u8> =
        serde_json::from_value(response["payload"]["bytes"].clone()).map_err(|_| malformed())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        if acceptance {
            fail(
                "ACCEPTANCE_UNKNOWN",
                "Submit response is incomplete; do not resubmit",
            )
        } else {
            malformed()
        }
    })?;
    let status = number(&response, "status")?;
    if !(200..300).contains(&status) {
        return Err(status_error(status, &value));
    }
    Ok(value)
}
fn download(
    h: &mut dyn Host,
    config: &Value,
    url: &str,
    kind: &str,
    digest: &str,
    length: u64,
    budget: u64,
) -> Outcome<Value> {
    let response = port(
        h,
        "host-http.execute",
        plan(
            config,
            "GET",
            url.to_string(),
            json!({"kind":"none"}),
            vec![],
            json!({"kind":"media"}),
            budget,
        )?,
    )?;
    let status = number(&response, "status")?;
    if status != 200 {
        return Err(status_error(status, &Value::Null));
    }
    let handle = text(&response["payload"], "handle")?;
    let stat = port(h, "host-media.info", json!({"handle":handle}))?;
    if stat["kind"] != kind || stat["sha256"] != digest || stat["byteLength"] != length.to_string()
    {
        return Err(fail(
            "UPSTREAM_ERROR",
            "Downloaded media differs from the service descriptor or requested kind",
        ));
    }
    Ok(
        json!({"kind":"media","role":"artifact","media":{"id":stat["id"],"kind":stat["kind"],"handle":handle,"mime":stat["mime"],"fileName":stat["fileName"],"byteLength":stat["byteLength"]}}),
    )
}
fn common_probe(h: &mut dyn Host, p: &Value, budget: u64) -> Outcome<Value> {
    match text(p, "kind")? {
        "package_validation" => Ok(
            json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]}),
        ),
        "connectivity" => {
            let health = request_json(
                h,
                &p["configuration"],
                "GET",
                "/health",
                None,
                budget,
                false,
            )?;
            if health["ok"] != true {
                return Err(malformed());
            }
            Ok(
                json!({"packageValid":true,"connection":"ready","authentication":"not_checked","resources":"not_checked","issues":[]}),
            )
        }
        _ => Err(fail("INVALID_INPUT", "Unknown probe kind")),
    }
}
fn finish(request: &Invocation, result: Outcome<Value>) -> Response {
    match result {
        Ok(value)=>guest::completed_response(request,&value).expect("standard serializable result"),
        Err(e)=>serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,"error":{"code":e.code,"message":e.message,"retryable":false,"vendorCode":e.vendor,"vendorRequestId":e.request}})).expect("public error"),
    }
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
    port(h,"host-checkpoint.write",json!({"requestId":request.r#request_id,"expectedRevision":old,"valueJson":checkpoint.to_string()})).map_err(|_|Failure{
        code:"ACCEPTANCE_UNKNOWN",message:"The service accepted the task but checkpoint storage failed; do not resubmit",
        vendor:Some("CHECKPOINT_COMMIT_FAILED".into()),request:accepted["vendorTaskId"].as_str().map(str::to_string)
    })?;
    Ok(())
}
fn upload_media(h: &mut dyn Host, config: &Value, media: &Value, budget: u64) -> Outcome<String> {
    let args = plan(
        config,
        "POST",
        format!(
            "{}/uploads?scenario={}&delayMs={}",
            text(config, "endpointBaseUrl")?,
            text(config, "uploadScenario")?,
            number(config, "delayMs")?
        ),
        json!({"kind":"media","handle":text(media,"handle")?}),
        vec![
            json!({"name":"Content-Type","value":text(media,"mime")?}),
            json!({"name":"X-File-Name","value":text(media,"fileName")?}),
        ],
        json!({"kind":"buffer","maxBytes":policy("maxBufferedHttpBytes")}),
        budget,
    )?;
    let response = port(h, "host-http.execute", args).map_err(|_| {
        fail(
            "ACCEPTANCE_UNKNOWN",
            "Upload acknowledgment was lost; no upload retry was attempted",
        )
    })?;
    let bytes: Vec<u8> =
        serde_json::from_value(response["payload"]["bytes"].clone()).map_err(|_| malformed())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        fail(
            "ACCEPTANCE_UNKNOWN",
            "Upload acknowledgment is incomplete; no upload retry was attempted",
        )
    })?;
    let status = number(&response, "status")?;
    if status != 201 {
        return Err(status_error(status, &value));
    }
    Ok(text(&value, "uploadId")?.to_string())
}
fn explicit_upload(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Value> {
    let reference = upload_media(h, &p["configuration"], &p["media"], request.r#budget_ms)?;
    Ok(json!({"mediaId":p["media"]["id"],"uploadRef":reference}))
}
fn source_upload(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Option<String>> {
    let Some(source) = p["input"].get("source") else {
        return Ok(None);
    };
    let id = text(source, "mediaId")?;
    let media = p["media"]
        .as_array()
        .ok_or_else(malformed)?
        .iter()
        .find(|m| m["id"] == id)
        .ok_or_else(|| {
            fail(
                "INVALID_INPUT",
                "Source ID does not name a supplied media lease",
            )
        })?;
    let uploads = p["uploads"].as_array().ok_or_else(malformed)?;
    if let Some(proof) = uploads.iter().find(|u| u["mediaId"] == id) {
        return Ok(Some(text(proof, "uploadRef")?.to_string()));
    }
    // The absence of a successful pre-upload proof means this submission uploads
    // its explicit media once; it never retries a failed upload.
    Ok(Some(upload_media(
        h,
        &p["configuration"],
        media,
        request.r#budget_ms,
    )?))
}
fn query_path(config: &Value, path: &str) -> Outcome<String> {
    Ok(format!(
        "{path}?scenario={}&delayMs={}",
        text(config, "queryScenario")?,
        number(config, "delayMs")?
    ))
}
fn query_result(
    status: &str,
    outputs: Vec<Value>,
    description: String,
    usage: Value,
    token: Value,
    vendor: Option<String>,
    details: Value,
) -> Value {
    let failure = match status {
        "failed" => {
            json!({"category":"vendorTaskFailed","message":"The fixture task failed and retained its partial output","vendorCode":vendor,"details":details})
        }
        "cancelled" => {
            json!({"category":"vendorTaskCancelled","message":"The fixture task was cancelled remotely","vendorCode":null,"details":details})
        }
        _ => Value::Null,
    };
    json!({"status":status,"outputs":outputs,"data":{"description":description},"usage":usage,
        "progress":if matches!(status,"queued"|"running"){json!({"fraction":null,"message":status})}else{Value::Null},
        "terminalFailure":failure,"resumeToken":token})
}

const PLUGIN_ID: &str = "local.examples.workflow";
fn configuration_schema() -> String {
    json!({"type":"object","properties":{
        "endpointBaseUrl":{"type":"string","minLength":1},"credentialRef":{"anyOf":[{"type":"null"},{"type":"string","minLength":1}]},
        "scenario":{"type":"string","enum":["normal","error","rate_limit","disconnect","slow","duplicate_usage","partial_output","acceptance_unknown"]},
        "queryScenario":{"type":"string","enum":["normal","error","rate_limit","disconnect","slow"]},"uploadScenario":{"type":"string","enum":["normal","error","rate_limit","disconnect","slow","acceptance_unknown"]},
        "pollsUntilReady":{"type":"integer","minimum":0},"delayMs":{"type":"integer","minimum":0}
    },"required":["endpointBaseUrl","credentialRef","scenario","queryScenario","uploadScenario","pollsUntilReady","delayMs"],"additionalProperties":false}).to_string()
}
fn capabilities() -> Value {
    json!({"schemaVersion":2,"pluginId":PLUGIN_ID,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[
        contract("generate","video.generate","video",Some("video"),true,false,None,None),
        contract("speech","video.speech","video",Some("video"),true,false,None,None),
        contract("edit","video.edit","video",Some("video"),true,false,Some("video"),None),
        contract("image","image.generate","image",Some("image"),true,false,None,None)
    ]})
}
fn output(operation: &str) -> Outcome<(&'static str, &'static str)> {
    match operation {
        "local.examples.workflow/generate" => Ok(("video", "VideoNode")),
        "local.examples.workflow/speech" => Ok(("video", "SpeechNode")),
        "local.examples.workflow/edit" => Ok(("video", "EditNode")),
        "local.examples.workflow/image" => Ok(("image", "ImageNode")),
        _ => Err(fail(
            "UNSUPPORTED_FEATURE",
            "Unknown workflow fixture operation",
        )),
    }
}
fn usage(metering: &Value) -> Outcome<Value> {
    if metering.is_null() {
        return Ok(Value::Null);
    }
    Ok(
        json!({"measurements":[{"metric":"local.examples.workflow/units","unit":"local.examples.workflow/units","amount":text(metering,"units")?,"source":"reported"}]}),
    )
}
fn submit(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Value> {
    let (_, class) = output(text(p, "operationId")?)?;
    let config = &p["configuration"];
    // This graph dialect and its node IDs are exclusively this adapter's API.
    let mut nodes = json!({"17":{"class_type":"TextNode","inputs":{"text":p["input"]["prompt"],"strength":p["input"]["strength"]}},"42":{"class_type":class,"inputs":{"prompt":["17",0]}}});
    if let Some(id) = source_upload(h, request, p)? {
        nodes["9"] = json!({"class_type":"UploadedInput","inputs":{"upload_id":id}});
        nodes["42"]["inputs"]["source"] = json!(["9", 0]);
    }
    let remote = request_json(
        h,
        config,
        "POST",
        "/graph/run",
        Some(
            json!({"nodes":nodes,"scenario":text(config,"scenario")?,"pollsUntilReady":number(config,"pollsUntilReady")?,"delayMs":number(config,"delayMs")?,"clientRequestId":request.r#request_id}),
        ),
        request.r#budget_ms,
        true,
    )?;
    if remote["phase"] != "waiting" {
        return Err(malformed());
    }
    let id = text(&remote, "run_token")?;
    if id.is_empty() {
        return Err(malformed());
    }
    let accepted = json!({"kind":"accepted","vendorTaskId":id,"appliedParams":null,"resumeToken":{"requestId":request.r#request_id},"usage":null});
    persist_accepted(h, request, &accepted)?;
    Ok(accepted)
}
fn query(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Value> {
    let config = &p["configuration"];
    let id = text(p, "vendorTaskId")?;
    let (kind, _) = output(text(p, "operationId")?)?;
    let remote = request_json(
        h,
        config,
        "GET",
        &query_path(config, &format!("/graph/history/{id}"))?,
        None,
        request.r#budget_ms,
        false,
    )?;
    if remote["run_token"] != id {
        return Err(malformed());
    }
    let status = match text(&remote, "phase")? {
        "waiting" => "queued",
        "executing" => "running",
        "finished" => "succeeded",
        "errored" => "failed",
        "interrupted" => "cancelled",
        _ => return Err(malformed()),
    };
    let history = &remote["history"];
    let mut outputs = Vec::new();
    for artifact in history["artifacts"].as_array().ok_or_else(malformed)? {
        if artifact["category"] != kind {
            return Err(malformed());
        }
        outputs.push(download(
            h,
            config,
            text(artifact, "download_url")?,
            kind,
            text(artifact, "digest")?,
            number(artifact, "length")?,
            request.r#budget_ms,
        )?);
    }
    let snapshot = if let Some(updates) = history.get("metering_updates") {
        updates
            .as_array()
            .and_then(|s| s.last())
            .ok_or_else(malformed)?
    } else {
        &history["metering"]
    };
    let vendor = if status == "failed" {
        Some(text(&remote["fault"], "tag")?.to_string())
    } else {
        None
    };
    Ok(query_result(
        status,
        outputs,
        format!("Graph {id}: {}", text(&remote, "phase")?),
        usage(snapshot)?,
        p["resumeToken"].clone(),
        vendor,
        json!({"runToken":id,"phase":remote["phase"],"inputDigest":text(&history["analysis"],"input_digest")?,"pollCount":number(&history["analysis"],"poll_count")?}),
    ))
}
fn cancel(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Value> {
    let id = text(p, "vendorTaskId")?;
    let remote = request_json(
        h,
        &p["configuration"],
        "POST",
        &format!("/graph/interrupt/{id}"),
        Some(json!({})),
        request.r#budget_ms,
        false,
    )?;
    if remote["run_token"] != id {
        return Err(malformed());
    }
    let confirmed = remote["interrupted"].as_bool().ok_or_else(malformed)?;
    if confirmed && remote["phase"] != "interrupted" {
        return Err(malformed());
    }
    Ok(json!({"outcome":if confirmed{"remote_confirmed"}else{"remote_requested"}}))
}
pub struct Workflow;
impl Adapter for Workflow {
    fn invoke(request: Invocation, h: &mut dyn Host) -> Response {
        let value = serde_json::to_value(&request).expect("typed invocation");
        let result = (|| {
            let p: Value = serde_json::from_str(&request.r#payload_json)
                .map_err(|_| fail("INVALID_INPUT", "Payload is not JSON"))?;
            match text(&value, "operation")? {
                "capabilities" => Ok(capabilities()),
                "probe" => common_probe(h, &p, request.r#budget_ms),
                "upload" => explicit_upload(h, &request, &p),
                "submit" => submit(h, &request, &p),
                "query" => query(h, &request, &p),
                "cancel" => cancel(h, &request, &p),
                _ => Err(fail("UNSUPPORTED_FEATURE", "Unknown transport operation")),
            }
        })();
        finish(&request, result)
    }
}
bgx_adapter_v2_sdk::export_adapter!(Workflow);
