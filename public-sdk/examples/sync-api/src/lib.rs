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

const PLUGIN_ID: &str = "local.examples.sync";
fn configuration_schema() -> String {
    json!({"type":"object","properties":{
        "endpointBaseUrl":{"type":"string","minLength":1},"credentialRef":{"anyOf":[{"type":"null"},{"type":"string","minLength":1}]},
        "scenario":{"type":"string","enum":["normal","error","rate_limit","disconnect","slow","duplicate_usage","partial_output","acceptance_unknown"]},
        "delayMs":{"type":"integer","minimum":0},"chunkDelayMs":{"type":"integer","minimum":0},"streamFormat":{"type":"string","enum":["sse","ndjson"]}
    },"required":["endpointBaseUrl","credentialRef","scenario","delayMs","chunkDelayMs","streamFormat"],"additionalProperties":false}).to_string()
}
fn capabilities() -> Value {
    let analysis = r#"{"type":"object","properties":{"description":{"type":"string"},"items":{"type":"array","items":{"type":"integer"}},"echoPrompt":{"type":"string"}},"required":["description","items","echoPrompt"],"additionalProperties":false}"#;
    json!({"schemaVersion":2,"pluginId":PLUGIN_ID,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[
        contract("generate","video.generate","video",Some("video"),false,false,None,None),
        contract("image","image.generate","image",Some("image"),false,false,None,None),
        contract("audio","audio.synthesize","speech",Some("audio"),false,false,None,None),
        contract("analyze","local.examples.sync/analyze","generic",None,false,false,None,Some(analysis)),
        contract("stream","text.generate","text",Some("text"),false,true,None,None)
    ]})
}
fn usage(snapshot: &Value) -> Outcome<Value> {
    if snapshot.is_null() {
        return Ok(Value::Null);
    }
    let mut measurements = Vec::new();
    for metric in ["input_tokens", "output_tokens", "total_tokens"] {
        measurements.push(json!({"metric":metric,"unit":"tokens","amount":number(snapshot,metric)?.to_string(),"source":"reported"}));
    }
    measurements.push(json!({"metric":"credits","unit":"credits","amount":text(snapshot,"credits")?,"source":"reported"}));
    Ok(json!({"measurements":measurements}))
}
fn emit(
    h: &mut dyn Host,
    request: &Invocation,
    sequence: &mut u64,
    kind: &str,
    payload: Value,
) -> Outcome<()> {
    let next = sequence.checked_add(1).ok_or_else(malformed)?;
    port(
        h,
        "host-stream.emit-event",
        json!({"event":{"requestId":request.r#request_id,"sequence":next,"type":kind,"payload":payload}}),
    )?;
    *sequence = next;
    Ok(())
}
struct Streaming {
    remote_sequence: u64,
    sequence: u64,
    usage: Value,
    done: Option<String>,
}
impl Streaming {
    fn event(&mut self, h: &mut dyn Host, request: &Invocation, event: Value) -> Outcome<()> {
        if self.done.is_some()
            || number(&event, "sequence")?
                != self.remote_sequence.checked_add(1).ok_or_else(malformed)?
        {
            return Err(malformed());
        }
        self.remote_sequence += 1;
        match text(&event, "type")? {
            "text" => emit(
                h,
                request,
                &mut self.sequence,
                "text_delta",
                json!({"channel":"answer","text":text(&event,"delta")?}),
            )?,
            "usage" => {
                self.usage = usage(&event["snapshot"])?;
                emit(h, request, &mut self.sequence, "usage", self.usage.clone())?;
            }
            "done" => self.done = Some(text(&event, "text")?.to_string()),
            "failure" => {
                return Err(Failure {
                    code: "UPSTREAM_ERROR",
                    message: "The service stream explicitly failed; partial output is not complete",
                    vendor: Some(text(&event["error"], "code")?.to_string()),
                    request: None,
                })
            }
            _ => return Err(fail("UPSTREAM_ERROR", "Unexpected fixture stream event")),
        }
        Ok(())
    }
}
fn stream_frame(bytes: &[u8], format: &str) -> Outcome<Value> {
    let frame = core::str::from_utf8(bytes).map_err(|_| malformed())?;
    if format == "ndjson" {
        return serde_json::from_str(frame).map_err(|_| malformed());
    }
    let mut data = String::new();
    let mut event_kind = None;
    let mut identifier = None;
    for line in frame.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
        } else if let Some(value) = line.strip_prefix("event:") {
            event_kind = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("id:") {
            identifier = Some(value.trim());
        } else if line.is_empty() || line.starts_with(':') {
        } else {
            return Err(malformed());
        }
    }
    let event: Value = serde_json::from_str(&data).map_err(|_| malformed())?;
    if event_kind != event["type"].as_str()
        || identifier != Some(number(&event, "sequence")?.to_string().as_str())
    {
        return Err(malformed());
    }
    Ok(event)
}
fn stream(h: &mut dyn Host, request: &Invocation, p: &Value, body: Value) -> Outcome<Value> {
    let config = &p["configuration"];
    let format = text(config, "streamFormat")?;
    let opened = port(
        h,
        "host-http-stream.open",
        plan(
            config,
            "POST",
            format!(
                "{}/stream?format={format}",
                text(config, "endpointBaseUrl")?
            ),
            json!({"kind":"bytes","bytes":serde_json::to_vec(&body).map_err(|_|malformed())?}),
            vec![json!({"name":"Content-Type","value":"application/json"})],
            json!({"kind":"buffer","maxBytes":policy("maxBufferedHttpBytes")}),
            request.r#budget_ms,
        )?,
    )?;
    let stream_id = text(&opened, "streamId")?.to_string();
    let status = number(&opened, "status")?;
    let result = (|| {
        let mut state = Streaming {
            remote_sequence: 0,
            sequence: 0,
            usage: Value::Null,
            done: None,
        };
        let mut pending = Vec::<u8>::new();
        let max = policy("maxBufferedHttpBytes") as usize;
        let chunk = policy("maxHttpChunkBytes");
        loop {
            let part = port(
                h,
                "host-http-stream.read",
                json!({"streamId":stream_id,"maxBytes":chunk}),
            )?;
            let bytes: Vec<u8> =
                serde_json::from_value(part["bytes"].clone()).map_err(|_| malformed())?;
            if pending.len().saturating_add(bytes.len()) > max {
                return Err(fail(
                    "UPSTREAM_ERROR",
                    "Fixture stream frame exceeds the explicit SDK buffer budget",
                ));
            }
            pending.extend(bytes);
            if status >= 200 && status < 300 {
                loop {
                    let end = if format == "sse" {
                        pending
                            .windows(2)
                            .position(|b| b == b"\n\n")
                            .map(|i| (i, 2))
                    } else {
                        pending.iter().position(|b| *b == b'\n').map(|i| (i, 1))
                    };
                    let Some((end, delimiter)) = end else {
                        break;
                    };
                    let event = stream_frame(&pending[..end], format)?;
                    pending.drain(..end + delimiter);
                    state.event(h, request, event)?;
                }
            }
            let eof = part["eof"].as_bool().ok_or_else(malformed)?;
            if eof {
                if !(200..300).contains(&status) {
                    let error: Value = serde_json::from_slice(&pending).map_err(|_| malformed())?;
                    return Err(status_error(status, &error));
                }
                if !pending.is_empty() {
                    return Err(fail(
                        "UPSTREAM_ERROR",
                        "Fixture stream ended with an incomplete frame",
                    ));
                }
                let answer = state.done.ok_or_else(|| {
                    fail(
                        "UPSTREAM_ERROR",
                        "Fixture stream ended without an explicit done event",
                    )
                })?;
                return Ok(
                    json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[{"kind":"text","role":"answer","text":answer}],"data":{"description":answer},"usage":state.usage}),
                );
            }
        }
    })();
    let closed = port(h, "host-http-stream.close", json!({"streamId":stream_id}));
    match result {
        Ok(value) => {
            closed?;
            Ok(value)
        }
        // The managed read may already revoke the stream. Cleanup is still
        // attempted above, but its secondary failure cannot replace the cause.
        Err(primary) => Err(primary),
    }
}
fn submit(h: &mut dyn Host, request: &Invocation, p: &Value) -> Outcome<Value> {
    let operation = text(p, "operationId")?;
    let config = &p["configuration"];
    let kind = match operation {
        "local.examples.sync/generate" => Some("video"),
        "local.examples.sync/image" => Some("image"),
        "local.examples.sync/audio" => Some("audio"),
        "local.examples.sync/analyze" | "local.examples.sync/stream" => None,
        _ => return Err(fail("UNSUPPORTED_FEATURE", "Unknown fixture operation")),
    };
    let kinds: Vec<&str> = kind.into_iter().collect();
    let body = json!({"input":{"prompt":p["input"]["prompt"],"strength":p["input"]["strength"],"outputKinds":kinds},"scenario":text(config,"scenario")?,"delayMs":number(config,"delayMs")?,"chunkDelayMs":number(config,"chunkDelayMs")?});
    if operation == "local.examples.sync/stream" {
        return stream(h, request, p, body);
    }
    let remote = request_json(
        h,
        config,
        "POST",
        "/generate",
        Some(body),
        request.r#budget_ms,
        false,
    )?;
    if remote["partial"].as_bool().ok_or_else(malformed)? {
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "The service reported partial output; generation is incomplete",
            vendor: Some(text(&remote["error"], "code")?.to_string()),
            request: None,
        });
    }
    let snapshot = if let Some(snapshots) = remote.get("usageSnapshots") {
        snapshots
            .as_array()
            .and_then(|s| s.last())
            .ok_or_else(malformed)?
    } else {
        &remote["usage"]
    };
    let measured = usage(snapshot)?;
    let mut outputs = Vec::new();
    for descriptor in remote["outputs"].as_array().ok_or_else(malformed)? {
        outputs.push(download(
            h,
            config,
            text(descriptor, "url")?,
            kind.ok_or_else(malformed)?,
            text(descriptor, "sha256")?,
            number(descriptor, "bytes")?,
            request.r#budget_ms,
        )?);
    }
    let data = if operation == "local.examples.sync/analyze" {
        let items = remote["data"]["items"].as_array().ok_or_else(malformed)?;
        if items.iter().any(|v| v.as_i64().is_none()) {
            return Err(malformed());
        }
        json!({"description":text(&remote,"text")?,"items":items,"echoPrompt":text(&remote["data"]["echo"],"prompt")?})
    } else {
        json!({"description":text(&remote,"text")?})
    };
    Ok(
        json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":outputs,"data":data,"usage":measured}),
    )
}
pub struct SyncApi;
impl Adapter for SyncApi {
    fn invoke(request: Invocation, h: &mut dyn Host) -> Response {
        let value = serde_json::to_value(&request).expect("typed invocation");
        let result = (|| {
            let payload: Value = serde_json::from_str(&request.r#payload_json)
                .map_err(|_| fail("INVALID_INPUT", "Payload is not JSON"))?;
            match text(&value, "operation")? {
                "capabilities" => Ok(capabilities()),
                "probe" => common_probe(h, &payload, request.r#budget_ms),
                "submit" => submit(h, &request, &payload),
                _ => Err(fail(
                    "UNSUPPORTED_FEATURE",
                    "This synchronous example has no asynchronous task or upload lifecycle",
                )),
            }
        })();
        finish(&request, result)
    }
}
bgx_adapter_v2_sdk::export_adapter!(SyncApi);
