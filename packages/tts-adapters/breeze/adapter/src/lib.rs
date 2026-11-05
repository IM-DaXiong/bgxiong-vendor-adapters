#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, MediaInputV2, OperationSubmitV2, Response};
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

const SEED_MAX: u64 = 4_294_967_295;
const MODEL_ID: &str = "BreezeBlue/Breeze-TTS-2";

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
            _ => Err(fail(
                "INVALID_INPUT",
                "mode must be plain|voice_design|reference_clone",
            )),
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
            let n = s
                .parse::<u64>()
                .map_err(|_| fail("INVALID_INPUT", "seed must be an unsigned integer string"))?;
            if n > SEED_MAX {
                return Err(fail(
                    "INVALID_INPUT",
                    "seed exceeds Breeze 32-bit range (0..4294967295); value was not truncated",
                ));
            }
            Ok(Some(n))
        }
        Some(Value::Number(n)) => {
            let n = n
                .as_u64()
                .ok_or_else(|| fail("INVALID_INPUT", "seed must fit in u64"))?;
            if n > SEED_MAX {
                return Err(fail(
                    "INVALID_INPUT",
                    "seed exceeds Breeze 32-bit range (0..4294967295); value was not truncated",
                ));
            }
            Ok(Some(n))
        }
        _ => Err(fail(
            "INVALID_INPUT",
            "seed must be a string or number when provided",
        )),
    }
}

fn nonempty_str<'a>(v: &'a Value, pointer: &str) -> Outcome<&'a str> {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| fail("INVALID_INPUT", "required string field is missing or empty"))
}

fn join_direction(style: Option<&str>, emotion_text: Option<&str>) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(s) = style {
        if !s.trim().is_empty() {
            parts.push(s);
        }
    }
    if let Some(s) = emotion_text {
        if !s.trim().is_empty() {
            parts.push(s);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("，"))
    }
}

fn emotion_text_for_instruction(input: &Value) -> Outcome<Option<String>> {
    let Some(ec) = input.get("emotionControl") else {
        return Ok(None);
    };
    if ec.is_null() {
        return Err(fail(
            "INVALID_INPUT",
            "emotionControl may be omitted but not null",
        ));
    }
    let mode = ec
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("INVALID_INPUT", "emotionControl.mode is required when emotionControl is present"))?;
    match mode {
        "none" | "inherit_speaker" => Ok(None),
        "text" => {
            let t = ec
                .get("text")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    fail(
                        "INVALID_INPUT",
                        "emotionControl.mode=text requires non-empty emotionControl.text",
                    )
                })?;
            Ok(Some(t.to_string()))
        }
        "audio_reference" => Err(fail(
            "UNSUPPORTED_FEATURE",
            "Breeze does not support emotionControl.mode=audio_reference; use text direction or none",
        )),
        "vector" => Err(fail(
            "UNSUPPORTED_FEATURE",
            "Breeze does not support emotionControl.mode=vector; use text direction or none",
        )),
        _ => Err(fail(
            "INVALID_INPUT",
            "emotionControl.mode must be none|inherit_speaker|text|audio_reference|vector",
        )),
    }
}

fn reject_pretend_native(input: &Value) -> Outcome<()> {
    if input.get("language").is_some() {
        return Err(fail(
            "UNSUPPORTED_FEATURE",
            "Breeze does not accept a language selector parameter; write Chinese/English in the text itself",
        ));
    }
    if input.get("rate").is_some() {
        return Err(fail(
            "UNSUPPORTED_FEATURE",
            "Breeze does not accept numeric rate/durationFactor; use natural-language pace in direction/controlPrompt",
        ));
    }
    // Top-level durationFactor is not in speech-v1 input, but reject if somehow present.
    if input.get("durationFactor").is_some() {
        return Err(fail(
            "UNSUPPORTED_FEATURE",
            "Breeze does not accept durationFactor; use natural-language pace instead",
        ));
    }
    if input.get("lang").is_some() {
        return Err(fail(
            "UNSUPPORTED_FEATURE",
            "Breeze does not accept lang; write language in the spoken text",
        ));
    }
    Ok(())
}

struct Receipt {
    applied: Vec<Value>,
    not_applied: Vec<Value>,
    final_instruction: Option<String>,
    style_original: Option<String>,
    emotion_original: Option<String>,
    control_prompt_original: Option<String>,
    prompt_text_original: Option<String>,
    temp_id: Option<String>,
}

impl Receipt {
    fn new() -> Self {
        Self {
            applied: Vec::new(),
            not_applied: Vec::new(),
            final_instruction: None,
            style_original: None,
            emotion_original: None,
            control_prompt_original: None,
            prompt_text_original: None,
            temp_id: None,
        }
    }
    fn apply(&mut self, pointer: &str, value: Value) {
        self.applied.push(json!({"pointer": pointer, "value": value}));
    }
    fn skip(&mut self, pointer: &str, reason: &str, message: &str) {
        self.not_applied.push(json!({
            "pointer": pointer,
            "reason": reason,
            "message": message
        }));
    }
}

fn collect_optional_not_applied(input: &Value, mode_name: &str, receipt: &mut Receipt) {
    if input.get("voiceId").is_some() {
        receipt.skip(
            "/voiceId",
            "unsupported",
            "Breeze has no vendor voiceId catalog; original input preserved and not sent",
        );
    }
    if input.get("runtimeProfile").is_some() {
        receipt.skip(
            "/runtimeProfile",
            "not_applicable",
            "runtimeProfile is negotiated by the managed companion profile, not applied as synthesize input",
        );
    }
    if mode_name == "plain" {
        if input.get("voiceDesign").is_some() {
            receipt.skip(
                "/voiceDesign",
                "not_applicable",
                "voiceDesign is not used in plain mode and was not sent to the model",
            );
        }
        if input.get("reference").is_some() {
            receipt.skip(
                "/reference",
                "not_applicable",
                "reference is not used in plain mode and was not sent to the model",
            );
        }
        if input.get("instruction").is_some() {
            receipt.skip(
                "/instruction",
                "not_applicable",
                "top-level instruction is not used in plain mode for Breeze",
            );
        }
    }
    if let Some(vd) = input.get("voiceDesign") {
        if mode_name == "voice_design" {
            for key in [
                "description",
                "genderHint",
                "ageHint",
                "emotion",
                "speakingStyle",
                "speedHint",
            ] {
                if vd.get(key).is_some() {
                    receipt.skip(
                        &format!("/voiceDesign/{key}"),
                        "unsupported",
                        "Breeze voice_design consumes only voiceDesign.controlPrompt; put design intent there",
                    );
                }
            }
        }
    }
    if let Some(ec) = input.get("emotionControl") {
        if let Some(mode) = ec.get("mode").and_then(Value::as_str) {
            if matches!(mode, "none" | "inherit_speaker") {
                receipt.skip(
                    "/emotionControl",
                    "not_applicable",
                    "emotionControl.mode does not add instruction text for this request",
                );
            }
        }
        if ec.get("alpha").is_some() {
            receipt.skip(
                "/emotionControl/alpha",
                "unsupported",
                "Breeze has no numeric emotion alpha",
            );
        }
        if ec.get("audio").is_some() {
            // hard-fail path covers audio_reference mode; leftover audio object still noted
            receipt.skip(
                "/emotionControl/audio",
                "unsupported",
                "Breeze does not consume emotion audio references",
            );
        }
        if ec.get("vector").is_some() {
            receipt.skip(
                "/emotionControl/vector",
                "unsupported",
                "Breeze does not consume emotion vectors",
            );
        }
    }
}

fn find_media<'a>(
    submit: &'a OperationSubmitV2,
    media_id: &str,
) -> Outcome<&'a MediaInputV2> {
    submit
        .r#media
        .iter()
        .find(|m| m.r#id == media_id)
        .ok_or_else(|| {
            fail(
                "INVALID_INPUT",
                "reference.audio.mediaId is not present in the authorized submit.media set",
            )
        })
}

fn upload_reference_temp(
    h: &mut dyn Host,
    origin: &str,
    media: &MediaInputV2,
) -> Outcome<(String, String)> {
    // Verify the authorized handle before multipart upload.
    let stat = port(h, "host-media.info", json!({"handle": media.r#handle}))?;
    let mime = stat
        .get("mime")
        .and_then(Value::as_str)
        .unwrap_or(media.r#mime.as_str());
    let kind = stat.get("kind").and_then(Value::as_str);
    if kind != Some("audio") && !mime.starts_with("audio/") {
        return Err(fail(
            "INVALID_INPUT",
            "reference media handle is not audio",
        ));
    }
    let bl = match stat.get("byteLength") {
        Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => media.r#byte_length.parse::<u64>().unwrap_or(0),
    };
    if bl < 64 {
        return Err(fail(
            "INVALID_INPUT",
            "reference audio is too small to be a real waveform",
        ));
    }
    // Prove bytes are readable via host-media (multipart streams the same lease).
    let _probe = port(
        h,
        "host-media.read",
        json!({"handle": media.r#handle, "offset": "0", "maxBytes": 64u32}),
    )?;

    let url = format!("{}/v1/audio/reference-temp", origin.trim_end_matches('/'));
    let plan = json!({
        "plan":{
            "method":"POST",
            "url":url,
            "headers":[],
            "generatedHeaders":[],
            "body":{
                "kind":"multipart",
                "parts":[{
                    "kind":"file",
                    "name":"file",
                    "fileName": media.r#file_name,
                    "handle": media.r#handle,
                    "mime": mime
                }]
            },
            "sink":{"kind":"buffer","maxBytes": 65536u32},
            "timeoutMs": 120000u64
        },
        "credential": Value::Null
    });
    let response = port(h, "host-http.execute", plan)?;
    let status = number(&response, "status")?;
    if status == 401 || status == 403 {
        return Err(fail("UNAUTHORIZED", "Companion rejected the reference upload"));
    }
    if !(200..300).contains(&status) {
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "Companion reference upload failed; no retry was attempted",
            vendor: Some(status.to_string()),
            request: None,
        });
    }
    let payload = &response["payload"];
    let bytes = payload
        .get("bytes")
        .and_then(Value::as_array)
        .ok_or_else(malformed)?;
    let raw: Vec<u8> = bytes
        .iter()
        .map(|v| v.as_u64().unwrap_or(0) as u8)
        .collect();
    let parsed: Value = serde_json::from_slice(&raw).map_err(|_| malformed())?;
    let audio_path = parsed
        .get("audioPath")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(malformed)?
        .to_string();
    let temp_id = parsed
        .get("tempId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(malformed)?
        .to_string();
    Ok((audio_path, temp_id))
}

fn release_temp(h: &mut dyn Host, origin: &str, temp_id: &str) {
    let url = format!(
        "{}/v1/audio/reference-temp/{}",
        origin.trim_end_matches('/'),
        temp_id
    );
    let plan = json!({
        "plan":{
            "method":"DELETE",
            "url":url,
            "headers":[],
            "generatedHeaders":[],
            "body":{"kind":"none"},
            "sink":{"kind":"buffer","maxBytes": 4096u32},
            "timeoutMs": 30000u64
        },
        "credential": Value::Null
    });
    let _ = port(h, "host-http.execute", plan);
}

fn build_companion_body(
    h: &mut dyn Host,
    submit: &OperationSubmitV2,
    input: &Value,
    mode_name: &str,
    text_value: &str,
    seed: Option<u64>,
    origin: &str,
    receipt: &mut Receipt,
) -> Outcome<Value> {
    let execution = seed.map(|s| json!({"seed": s}));
    let emotion = emotion_text_for_instruction(input)?;
    receipt.emotion_original = emotion.clone();

    match mode_name {
        "plain" => {
            if emotion.is_some() {
                receipt.skip(
                    "/emotionControl",
                    "not_applicable",
                    "plain mode does not send emotionControl text to Breeze; use voice_design or reference_clone direction",
                );
            }
            receipt.apply("/mode", json!("plain"));
            receipt.apply("/text", json!(text_value));
            if let Some(s) = seed {
                if let Some(v) = input.get("seed") {
                    receipt.apply("/seed", v.clone());
                } else {
                    receipt.apply("/seed", json!(s.to_string()));
                }
            }
            let mut body = json!({
                "model": MODEL_ID,
                "mode":"plain",
                "input": text_value
            });
            if let Some(exec) = execution {
                body["execution"] = exec;
            }
            Ok(body)
        }
        "voice_design" => {
            let prompt = nonempty_str(input, "/voiceDesign/controlPrompt").map_err(|_| {
                fail(
                    "INVALID_INPUT",
                    "voice_design requires voiceDesign.controlPrompt",
                )
            })?;
            receipt.control_prompt_original = Some(prompt.to_string());
            let instruction = join_direction(Some(prompt), emotion.as_deref())
                .unwrap_or_else(|| prompt.to_string());
            receipt.final_instruction = Some(instruction.clone());
            receipt.apply("/mode", json!("voice_design"));
            receipt.apply("/text", json!(text_value));
            receipt.apply("/voiceDesign/controlPrompt", json!(prompt));
            if let Some(ref e) = emotion {
                receipt.apply("/emotionControl/text", json!(e));
            }
            if let Some(s) = seed {
                if let Some(v) = input.get("seed") {
                    receipt.apply("/seed", v.clone());
                } else {
                    receipt.apply("/seed", json!(s.to_string()));
                }
            }
            let mut body = json!({
                "model": MODEL_ID,
                "mode":"voice_design",
                "input": text_value,
                "voiceDesign":{"controlPrompt": instruction}
            });
            if let Some(exec) = execution {
                body["execution"] = exec;
            }
            Ok(body)
        }
        "reference_clone" => {
            let media_id = nonempty_str(input, "/reference/audio/mediaId").map_err(|_| {
                fail(
                    "INVALID_INPUT",
                    "reference_clone requires reference.audio.mediaId",
                )
            })?;
            let prompt_text = nonempty_str(input, "/reference/promptText").map_err(|_| {
                fail(
                    "INVALID_INPUT",
                    "reference_clone requires non-empty reference.promptText (accurate transcript); no ASR fill-in",
                )
            })?;
            receipt.prompt_text_original = Some(prompt_text.to_string());
            let style = input
                .pointer("/reference/styleControl")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            receipt.style_original = style.clone();
            let instruction = join_direction(style.as_deref(), emotion.as_deref());
            receipt.final_instruction = instruction.clone();

            let media = find_media(submit, media_id)?;
            let (audio_path, temp_id) = upload_reference_temp(h, origin, media)?;
            receipt.temp_id = Some(temp_id);

            receipt.apply("/mode", json!("reference_clone"));
            receipt.apply("/text", json!(text_value));
            receipt.apply("/reference/audio/mediaId", json!(media_id));
            receipt.apply("/reference/promptText", json!(prompt_text));
            if let Some(ref s) = style {
                receipt.apply("/reference/styleControl", json!(s));
            }
            if let Some(ref e) = emotion {
                receipt.apply("/emotionControl/text", json!(e));
            }
            if let Some(s) = seed {
                if let Some(v) = input.get("seed") {
                    receipt.apply("/seed", v.clone());
                } else {
                    receipt.apply("/seed", json!(s.to_string()));
                }
            }

            let mut reference = json!({
                "audioPath": audio_path,
                "promptText": prompt_text
            });
            if let Some(instr) = instruction {
                reference["styleControl"] = json!(instr);
            }
            let mut body = json!({
                "model": MODEL_ID,
                "mode":"reference_clone",
                "input": text_value,
                "reference": reference
            });
            if let Some(exec) = execution {
                body["execution"] = exec;
            }
            Ok(body)
        }
        _ => Err(fail("INVALID_INPUT", "unsupported mode")),
    }
}

fn build_data_attachment(receipt: &Receipt) -> Value {
    let mut payload = json!({});
    if !receipt.applied.is_empty() {
        payload["applied"] = Value::Array(receipt.applied.clone());
    }
    if !receipt.not_applied.is_empty() {
        payload["notApplied"] = Value::Array(receipt.not_applied.clone());
    }
    let mut attachments = vec![json!({
        "kind": "parameter_receipt",
        "payload": payload
    })];
    if receipt.final_instruction.is_some()
        || receipt.control_prompt_original.is_some()
        || receipt.style_original.is_some()
        || receipt.emotion_original.is_some()
        || receipt.prompt_text_original.is_some()
    {
        attachments.push(json!({
            "kind": "diagnostic",
            "payload": {
                "code": "breeze_instruction_audit",
                "message": "Original control fields and final companion instruction",
                "details": {
                    "finalInstruction": receipt.final_instruction,
                    "controlPromptOriginal": receipt.control_prompt_original,
                    "styleControlOriginal": receipt.style_original,
                    "emotionTextOriginal": receipt.emotion_original,
                    "promptTextOriginal": receipt.prompt_text_original
                }
            }
        }));
    }
    json!({
        "schemaVersion": 1,
        "attachments": attachments
    })
}

fn applied_params_echo(input: &Value, receipt: &Receipt) -> Value {
    // appliedParams keys must be a subset of frozen input keys.
    let mut obj = serde_json::Map::new();
    if input.get("seed").is_some() {
        obj.insert("seed".into(), input.get("seed").cloned().unwrap());
    }
    if input.get("mode").is_some() {
        obj.insert("mode".into(), input.get("mode").cloned().unwrap());
    }
    if !obj.is_empty() {
        Value::Object(obj)
    } else if receipt.applied.is_empty() {
        Value::Null
    } else {
        // Keep null when only derived fields were applied; receipt carries detail.
        Value::Null
    }
}

fn synthesize(h: &mut dyn Host, submit: &OperationSubmitV2) -> Outcome<Value> {
    let input = Value::Object(submit.r#input.clone().into_iter().collect());
    let mode_name = mode(&input)?;
    let delivery_name = delivery(&input)?;
    if delivery_name == "stream" {
        return Err(fail(
            "UNSUPPORTED_FEATURE",
            "stream delivery is not provided by this pack; complete delivery remains available",
        ));
    }
    reject_pretend_native(&input)?;
    let text_value = input_text(&input)?;
    let seed = optional_seed(&input)?;
    let mut receipt = Receipt::new();
    collect_optional_not_applied(&input, mode_name, &mut receipt);

    let origin = submit
        .r#configuration
        .get("origin")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("INVALID_INPUT", "configuration.origin is required"))?;
    if origin.is_empty() {
        return Err(fail(
            "INVALID_INPUT",
            "configuration.origin must be non-empty",
        ));
    }

    let body = build_companion_body(
        h, submit, &input, mode_name, &text_value, seed, origin, &mut receipt,
    )?;
    let temp_id = receipt.temp_id.clone();

    let url = format!("{}/v1/audio/speech", origin.trim_end_matches('/'));
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
    let response = match port(h, "host-http.execute", plan) {
        Ok(v) => v,
        Err(e) => {
            if let Some(tid) = temp_id.as_deref() {
                release_temp(h, origin, tid);
            }
            return Err(e);
        }
    };
    let status = match number(&response, "status") {
        Ok(s) => s,
        Err(e) => {
            if let Some(tid) = temp_id.as_deref() {
                release_temp(h, origin, tid);
            }
            return Err(e);
        }
    };
    if let Some(tid) = temp_id.as_deref() {
        release_temp(h, origin, tid);
    }
    if status == 401 || status == 403 {
        return Err(fail("UNAUTHORIZED", "Companion rejected the request"));
    }
    if status == 429 {
        return Err(fail("RATE_LIMITED", "Companion rate limited the request"));
    }
    if !(200..300).contains(&status) {
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
        let mime = stat.get("mime").and_then(Value::as_str).unwrap_or("");
        if !mime.starts_with("audio/") && stat.get("kind").and_then(Value::as_str) != Some("audio")
        {
            return Err(fail(
                "UPSTREAM_ERROR",
                "Companion response was not audio media",
            ));
        }
    }
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
    let media = json!({
        "id": stat["id"],
        "kind": "audio",
        "handle": handle,
        "mime": stat.get("mime").cloned().unwrap_or_else(|| json!("audio/wav")),
        "fileName": stat.get("fileName").cloned().unwrap_or_else(|| json!("speech.wav")),
        "byteLength": stat["byteLength"]
    });
    Ok(json!({
        "kind":"completed",
        "vendorTaskId": null,
        "appliedParams": applied_params_echo(&input, &receipt),
        "outputs":[{"kind":"media","role":"audio","media": media}],
        "data": build_data_attachment(&receipt),
        "usage": null
    }))
}

pub struct BreezeSpeechAdapter;
impl Adapter for BreezeSpeechAdapter {
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
                            "This pack declares only ordinary synthesis for delivery",
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
                    "This synchronous pack does not implement the selected operation",
                ),
            ),
        }
    }
}
bgx_adapter_v2_sdk::export_adapter!(BreezeSpeechAdapter);
