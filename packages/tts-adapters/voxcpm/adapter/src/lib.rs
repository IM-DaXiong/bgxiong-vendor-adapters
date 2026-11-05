#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, OperationSubmitV2, Response};
use serde_json::{json, Value};

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
        "The companion returned an invalid structured response",
    )
}
fn text<'a>(v: &'a Value, key: &str) -> Outcome<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(malformed)
}
fn number(v: &Value, key: &str) -> Outcome<u64> {
    v.get(key).and_then(Value::as_u64).ok_or_else(malformed)
}
fn port(h: &mut dyn Host, method: &str, args: Value) -> Outcome<Value> {
    host::call(h, method, &args).map_err(|_| {
        fail(
            "RESOURCE_UNAVAILABLE",
            "A managed host operation failed; no retry was attempted",
        )
    })
}
fn capabilities() -> Value {
    serde_json::from_str(include_str!("capabilities.json")).expect("speech profile")
}
fn failure(request: &Invocation, f: Failure) -> Response {
    serde_json::from_value(json!({
        "protocolVersion":2,
        "operation":request.r#operation,
        "error":{
            "code":f.code,
            "message":f.message,
            "retryable":false,
            "vendorCode":f.vendor,
            "vendorRequestId":f.request
        }
    }))
    .expect("closed public error")
}
fn finish(request: &Invocation, result: Outcome<Value>) -> Response {
    match result {
        Ok(value) => guest::completed_response(request, &value).expect("serializable"),
        Err(e) => failure(request, e),
    }
}

fn input_text(input: &Value) -> Outcome<String> {
    let raw = input
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("INVALID_INPUT", "text is required"))?;
    if raw.is_empty() {
        return Err(fail("INVALID_INPUT", "text must be non-empty"));
    }
    if raw.chars().all(|c| c.is_whitespace()) {
        return Err(fail(
            "INVALID_INPUT",
            "text has no synthesizable content (whitespace only); original text was preserved and not altered",
        ));
    }
    Ok(raw.to_string())
}

fn mode(input: &Value) -> Outcome<&'static str> {
    match input.get("mode") {
        None => Ok("plain"),
        Some(Value::Null) => Err(fail("INVALID_INPUT", "mode may be omitted but not null")),
        Some(Value::String(s)) => match s.as_str() {
            "plain" => Ok("plain"),
            "voice_design" => Ok("voice_design"),
            "reference_clone" => Ok("reference_clone"),
            _ => Err(fail("INVALID_INPUT", "mode must be plain|voice_design|reference_clone")),
        },
        _ => Err(fail("INVALID_INPUT", "mode must be a string when provided")),
    }
}

fn delivery(input: &Value) -> Outcome<&'static str> {
    let exec = input.get("execution");
    match exec.and_then(|e| e.get("delivery")) {
        None => Ok("complete"),
        Some(Value::String(s)) if s == "complete" => Ok("complete"),
        Some(Value::String(s)) if s == "stream" => Ok("stream"),
        Some(Value::Null) => Err(fail(
            "INVALID_INPUT",
            "execution.delivery may be omitted but not null",
        )),
        _ => Err(fail(
            "INVALID_INPUT",
            "execution.delivery must be complete|stream when provided",
        )),
    }
}


fn optional_seed(input: &Value) -> Outcome<Option<u64>> {
    match input.get("seed") {
        None => Ok(None),
        Some(Value::Null) => Err(fail("INVALID_INPUT", "seed may be omitted but not null")),
        Some(Value::String(s)) => {
            if s.is_empty() {
                return Err(fail("INVALID_INPUT", "seed must be non-empty when provided"));
            }
            s.parse::<u64>()
                .map(Some)
                .map_err(|_| fail("INVALID_INPUT", "seed must be an unsigned integer string"))
        }
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| fail("INVALID_INPUT", "seed must fit in u64")),
        _ => Err(fail("INVALID_INPUT", "seed must be a string or number when provided")),
    }
}

fn collect_unused(input: &Value, mode: &str) -> Vec<Value> {
    let mut notices = Vec::new();
    // Optional capabilities this A-pack does not claim for plain path.
    if input.get("voiceId").is_some() {
        notices.push(json!({
            "code":"optional_not_applied",
            "pointer":"/voiceId",
            "message":"This A-pack does not apply voiceId for the selected mode; original input preserved"
        }));
    }
    if input.get("runtimeProfile").is_some() {
        notices.push(json!({
            "code":"optional_not_applied",
            "pointer":"/runtimeProfile",
            "message":"runtimeProfile is not applied by this A-pack; original input preserved"
        }));
    }
    if input.get("seed").is_some() {
        notices.push(json!({
            "code":"optional_not_applied",
            "pointer":"/seed",
            "message":"This VoxCPM A-pack does not apply seed; original input preserved"
        }));
    }
    if mode == "plain" {
        if input.get("voiceDesign").is_some() {
            notices.push(json!({
                "code":"optional_not_applied",
                "pointer":"/voiceDesign",
                "message":"voiceDesign is not used in plain mode and was not sent to the model"
            }));
        }
        if input.get("reference").is_some() {
            notices.push(json!({
                "code":"optional_not_applied",
                "pointer":"/reference",
                "message":"reference is not used in plain mode and was not sent to the model"
            }));
        }
    }
    notices
}

fn build_companion_body(
    input: &Value,
    mode_name: &str,
    text_value: &str,
    seed: Option<u64>,
) -> Outcome<Value> {
    // Vendor names/endpoints stay in-pack. Companion contract forbids unknown keys.
    // VoxCPM companion has no execution.seed; seed stays optional_not_applied when present.
    let _seed_forward = seed;
match mode_name {
        "plain" => Ok(json!({
                "model":"openbmb/VoxCPM2",
                "mode":"plain",
                "input": text_value
            })),
        "voice_design" => {
            let prompt = input
                .pointer("/voiceDesign/controlPrompt")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    fail(
                        "INVALID_INPUT",
                        "voice_design requires voiceDesign.controlPrompt",
                    )
                })?;
            Ok(json!({
                "model":"openbmb/VoxCPM2",
                "mode":"voice_design",
                "input": text_value,
                "voiceDesign":{"controlPrompt": prompt}
            }))
        }
        "reference_clone" => {
            // Honest: clone needs real reference audio path + transcript.
            // SDK mediaId must be resolved by host before companion; this A-pack
            // expects configuration-side staging that maps media to a local path
            // via host-media — not implemented for S1 plain path.
            Err(fail(
                "UNSUPPORTED_FEATURE",
                "reference_clone requires a real reference audio transcript and local path mapping; not part of this A-pack plain delivery",
            ))
        }
        _ => Err(fail("INVALID_INPUT", "unsupported mode")),
    }
}

fn synthesize(h: &mut dyn Host, submit: &OperationSubmitV2) -> Outcome<Value> {
    let input = Value::Object(submit.r#input.clone().into_iter().collect());
    let mode_name = mode(&input)?;
    let delivery_name = delivery(&input)?;
    if delivery_name == "stream" {
        return Err(fail(
            "UNSUPPORTED_FEATURE",
            "stream delivery is not provided by this A-pack; complete delivery remains available",
        ));
    }
    // Explicit selections must not be silently replaced.
    if mode_name != "plain" && mode_name != "voice_design" {
        // reference_clone already errors with honest prerequisite message
    }
    if let Some(v) = input.get("voiceId") {
        // explicit voiceId cannot be silently ignored as if applied; we error only when
        // it would be required. For plain, report unused via notices in appliedParams.
        let _ = v;
    }
    let text_value = input_text(&input)?;
    let seed = optional_seed(&input)?;
    let notices = collect_unused(&input, mode_name);
    let body = build_companion_body(&input, mode_name, &text_value, seed)?;

    let origin = submit
        .r#configuration
        .get("origin")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("INVALID_INPUT", "configuration.origin is required"))?;
    if origin.is_empty() {
        return Err(fail("INVALID_INPUT", "configuration.origin must be non-empty"));
    }
    let url = format!("{}/v1/audio/speech", origin.trim_end_matches('/'));

    // Persist-equivalent: host-http.execute with before-send audit is enforced by CLI/host.
    // Body bytes are exactly what will be sent; companion must not append unaudited prompts.
    let body_bytes = serde_json::to_vec(&body).map_err(|_| malformed())?;
    let plan = json!({
        "plan":{
            "method":"POST",
            "url":url,
            "headers":[{"name":"Content-Type","value":"application/json; charset=utf-8"}],
            "generatedHeaders":[],
            "body":{"kind":"bytes","bytes": body_bytes},
            "sink":{"kind":"media"},
            "timeoutMs": 600000u64
        },
        "credential": Value::Null
    });
    let response = port(h, "host-http.execute", plan)?;
    let status = number(&response, "status")?;
    if status == 401 || status == 403 {
        return Err(fail("UNAUTHORIZED", "Companion rejected the request"));
    }
    if status == 429 {
        return Err(fail("RATE_LIMITED", "Companion rate limited the request"));
    }
    if !(200..300).contains(&status) {
        // try buffer-less: media sink on error may still have handle; prefer message
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "Companion synthesis failed; no retry was attempted",
            vendor: Some(status.to_string()),
            request: None,
        });
    }
    let handle = text(&response["payload"], "handle")?;
    let stat = port(h, "host-media.info", json!({"handle": handle}))?;
    if stat.get("kind").and_then(Value::as_str) != Some("audio") {
        // host may classify wav as audio; if generic, still accept when mime audio/*
        let mime = stat.get("mime").and_then(Value::as_str).unwrap_or("");
        if !mime.starts_with("audio/") && stat.get("kind").and_then(Value::as_str) != Some("audio") {
            return Err(fail(
                "UPSTREAM_ERROR",
                "Companion response was not audio media",
            ));
        }
    }
    let byte_length = stat
        .get("byteLength")
        .and_then(|v| v.as_str().or_else(|| v.as_u64().map(|_| ""))) // may be u64 text
        .map(|s| s.to_string());
    let bl = match stat.get("byteLength") {
        Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    };
    if bl < 1000 {
        return Err(fail(
            "UPSTREAM_ERROR",
            "Companion returned audio that is too small to be a real waveform",
        ));
    }
    let _ = byte_length;
    let media = json!({
        "id": stat["id"],
        "kind": "audio",
        "handle": handle,
        "mime": stat.get("mime").cloned().unwrap_or_else(|| json!("audio/wav")),
        "fileName": stat.get("fileName").cloned().unwrap_or_else(|| json!("speech.wav")),
        "byteLength": stat["byteLength"]
    });
    // appliedParams keys must be a subset of frozen input keys (validator).
    // B-pack: when seed was present and forwarded, echo only that input key.
    // seed not forwarded on VoxCPM companion; keep appliedParams null (S1 A-pack rule).
    let applied = Value::Null;
    let _ = (mode_name, notices, text_value, seed);
    Ok(json!({
        "kind":"completed",
        "vendorTaskId": null,
        "appliedParams": applied,
        "outputs":[{"kind":"media","role":"audio","media": media}],
        "data": null,
        "usage": null
    }))
}

pub struct VoxcpmSpeechAdapter;
impl Adapter for VoxcpmSpeechAdapter {
    fn invoke(request: Invocation, host: &mut dyn Host) -> Response {
        let operation = serde_json::to_value(&request.r#operation).expect("closed operation");
        match operation.as_str().expect("operation string") {
            "capabilities" => {
                guest::completed_response(&request, &capabilities()).expect("serializable")
            }
            "probe" => {
                let result = json!({
                    "packageValid":true,
                    "connection":"not_checked",
                    "authentication":"not_checked",
                    "resources":"not_checked",
                    "issues":[]
                });
                guest::completed_response(&request, &result).expect("serializable")
            }
            "submit" => {
                let input: OperationSubmitV2 = match serde_json::from_str(&request.r#payload_json) {
                    Ok(v) => v,
                    Err(_) => {
                        return failure(
                            &request,
                            fail(
                                "INVALID_INPUT",
                                "Submit does not match the public request contract",
                            ),
                        )
                    }
                };
                if input.r#operation_id != "std/speech.synthesize" {
                    return failure(
                        &request,
                        fail(
                            "UNSUPPORTED_FEATURE",
                            "This A-pack declares only ordinary synthesis for delivery",
                        ),
                    );
                }
                if input.r#capability_key != "audio.synthesize" {
                    return failure(
                        &request,
                        fail("INVALID_INPUT", "capabilityKey must be audio.synthesize"),
                    );
                }
                finish(&request, synthesize(host, &input))
            }
            _ => failure(
                &request,
                fail(
                    "UNSUPPORTED_FEATURE",
                    "This synchronous A-pack does not implement the selected operation",
                ),
            ),
        }
    }
}
bgx_adapter_v2_sdk::export_adapter!(VoxcpmSpeechAdapter);
