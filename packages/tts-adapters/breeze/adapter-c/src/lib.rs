#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{
    format,
    string::{String, ToString},
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
    Failure { code, message, vendor: None, request: None }
}
fn malformed() -> Failure {
    fail("UPSTREAM_ERROR", "The companion returned an invalid structured response")
}
fn text<'a>(v: &'a Value, key: &str) -> Outcome<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(malformed)
}
fn number(v: &Value, key: &str) -> Outcome<u64> {
    v.get(key).and_then(Value::as_u64).ok_or_else(malformed)
}
fn port(h: &mut dyn Host, method: &str, args: Value) -> Outcome<Value> {
    host::call(h, method, &args).map_err(|e| {
        let msg = host::diagnostic(&e);
        Failure {
            code: "RESOURCE_UNAVAILABLE",
            message: "A managed host operation failed; no retry was attempted",
            vendor: Some(format!("{method}: {msg}")),
            request: None,
        }
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
    let raw = input.get("text").and_then(Value::as_str)
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
    match input.get("execution").and_then(|e| e.get("delivery")) {
        None => Ok("complete"),
        Some(Value::String(s)) if s == "complete" => Ok("complete"),
        Some(Value::String(s)) if s == "stream" => Ok("stream"),
        Some(Value::Null) => Err(fail("INVALID_INPUT", "execution.delivery may be omitted but not null")),
        _ => Err(fail("INVALID_INPUT", "execution.delivery must be complete|stream when provided")),
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
            let n = s.parse::<u64>().map_err(|_| fail("INVALID_INPUT", "seed must be an unsigned integer string"))?;
            if n > SEED_MAX {
                return Err(fail("INVALID_INPUT", "seed exceeds Breeze 32-bit range (0..4294967295); value was not truncated"));
            }
            Ok(Some(n))
        }
        Some(Value::Number(n)) => {
            let n = n.as_u64().ok_or_else(|| fail("INVALID_INPUT", "seed must fit in u64"))?;
            if n > SEED_MAX {
                return Err(fail("INVALID_INPUT", "seed exceeds Breeze 32-bit range (0..4294967295); value was not truncated"));
            }
            Ok(Some(n))
        }
        _ => Err(fail("INVALID_INPUT", "seed must be a string or number when provided")),
    }
}


const SEED_MAX: u64 = 4_294_967_295;

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
        return Err(fail("INVALID_INPUT", "emotionControl may be omitted but not null"));
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
                .ok_or_else(|| fail("INVALID_INPUT", "emotionControl.mode=text requires non-empty emotionControl.text"))?;
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

fn upload_reference_temp(h: &mut dyn Host, origin: &str, media: &MediaInputV2) -> Outcome<(String, String)> {
    let stat = port(h, "host-media.info", json!({"handle": media.r#handle}))?;
    let mime = stat.get("mime").and_then(Value::as_str).unwrap_or(media.r#mime.as_str());
    let kind = stat.get("kind").and_then(Value::as_str);
    if kind != Some("audio") && !mime.starts_with("audio/") {
        return Err(fail("INVALID_INPUT", "reference media handle is not audio"));
    }
    let _ = port(
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
    if !(200..300).contains(&status) {
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "Companion reference upload failed; no retry was attempted",
            vendor: Some(status.to_string()),
            request: None,
        });
    }
    let bytes = response["payload"].get("bytes").and_then(Value::as_array).ok_or_else(malformed)?;
    let raw: Vec<u8> = bytes.iter().map(|v| v.as_u64().unwrap_or(0) as u8).collect();
    let parsed: Value = serde_json::from_slice(&raw).map_err(|_| malformed())?;
    let audio_path = parsed.get("audioPath").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(malformed)?.to_string();
    let temp_id = parsed.get("tempId").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(malformed)?.to_string();
    Ok((audio_path, temp_id))
}

fn release_temp(h: &mut dyn Host, origin: &str, temp_id: &str) {
    let url = format!("{}/v1/audio/reference-temp/{}", origin.trim_end_matches('/'), temp_id);
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


fn origin_of(submit: &OperationSubmitV2) -> Outcome<&str> {
    let origin = submit.r#configuration.get("origin").and_then(Value::as_str)
        .ok_or_else(|| fail("INVALID_INPUT", "configuration.origin is required"))?;
    if origin.is_empty() {
        return Err(fail("INVALID_INPUT", "configuration.origin must be non-empty"));
    }
    Ok(origin)
}

fn build_companion_body(h: &mut dyn Host, submit: &OperationSubmitV2, input: &Value, mode_name: &str, text_value: &str, seed: Option<u64>, delivery_name: &str) -> Outcome<(Value, Option<String>)> {
    let mut execution = json!({});
    if let Some(s) = seed {
        execution["seed"] = json!(s);
    }
    if delivery_name == "stream" {
        execution["delivery"] = json!("stream");
    }
    let execution = if execution.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
        Some(execution)
    } else {
        None
    };
    match mode_name {
        "plain" => {
            let mut body = json!({"model":"BreezeBlue/Breeze-TTS-2","mode":"plain","input": text_value});
            if let Some(exec) = execution { body["execution"] = exec; }
            Ok((body, None))
        }
        "voice_design" => {
            let prompt = input.pointer("/voiceDesign/controlPrompt").and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| fail("INVALID_INPUT", "voice_design requires voiceDesign.controlPrompt"))?;
            let emotion = emotion_text_for_instruction(input)?;
            let instruction = join_direction(Some(prompt), emotion.as_deref()).unwrap_or_else(|| prompt.to_string());
            let mut body = json!({
                "model":"BreezeBlue/Breeze-TTS-2",
                "mode":"voice_design",
                "input": text_value,
                "voiceDesign":{"controlPrompt": instruction}
            });
            if let Some(exec) = execution { body["execution"] = exec; }
            Ok((body, None))
        }
        "reference_clone" => {
            let media_id = input
                .pointer("/reference/audio/mediaId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| fail("INVALID_INPUT", "reference_clone requires reference.audio.mediaId"))?;
            let prompt_text = input
                .pointer("/reference/promptText")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| fail(
                    "INVALID_INPUT",
                    "reference_clone requires non-empty reference.promptText (accurate transcript); no ASR fill-in",
                ))?;
            let style = input.pointer("/reference/styleControl").and_then(Value::as_str);
            let emotion = emotion_text_for_instruction(input)?;
            let instruction = join_direction(style, emotion.as_deref());
            let media = submit
                .r#media
                .iter()
                .find(|m| m.r#id == media_id)
                .ok_or_else(|| fail(
                    "INVALID_INPUT",
                    "reference.audio.mediaId is not present in the authorized submit.media set",
                ))?;
            let origin = origin_of(submit)?;
            let (audio_path, temp_id) = upload_reference_temp(h, origin, media)?;
            let mut reference = json!({"audioPath": audio_path, "promptText": prompt_text});
            if let Some(instr) = instruction {
                reference["styleControl"] = json!(instr);
            }
            let mut body = json!({
                "model":"BreezeBlue/Breeze-TTS-2",
                "mode":"reference_clone",
                "input": text_value,
                "reference": reference
            });
            if let Some(exec) = execution { body["execution"] = exec; }
            Ok((body, Some(temp_id)))
        }
        _ => Err(fail("INVALID_INPUT", "unsupported mode")),
    }
}

fn media_from_handle(h: &mut dyn Host, handle: &str) -> Outcome<Value> {
    let stat = port(h, "host-media.info", json!({"handle": handle}))?;
    let mime = stat.get("mime").and_then(Value::as_str).unwrap_or("");
    let kind = stat.get("kind").and_then(Value::as_str);
    if kind != Some("audio") && !mime.starts_with("audio/") {
        return Err(fail("UPSTREAM_ERROR", "Companion response was not audio media"));
    }
    let bl = match stat.get("byteLength") {
        Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    };
    if bl < 64 {
        return Err(fail("UPSTREAM_ERROR", "Companion returned audio that is too small to be a real waveform"));
    }
    Ok(json!({
        "id": stat["id"],
        "kind": "audio",
        "handle": handle,
        "mime": stat.get("mime").cloned().unwrap_or_else(|| json!("audio/wav")),
        "fileName": stat.get("fileName").cloned().unwrap_or_else(|| json!("speech.wav")),
        "byteLength": stat["byteLength"]
    }))
}

fn header_value(headers: &Value, name: &str) -> Option<String> {
    let want = name.to_ascii_lowercase();
    headers.as_array()?.iter().find_map(|h| {
        let n = h.get("name").and_then(Value::as_str)?;
        if n.to_ascii_lowercase() == want {
            h.get("value").and_then(Value::as_str).map(|s| s.to_string())
        } else {
            None
        }
    })
}

fn emit_progress(h: &mut dyn Host, request_id: &str, sequence: u64, message: &str) -> Outcome<()> {
    let event = json!({
        "requestId": request_id,
        "sequence": sequence,
        "type": "progress",
        "payload": { "fraction": Value::Null, "message": message }
    });
    port(h, "host-stream.emit-event", json!({"event": event}))?;
    Ok(())
}

fn emit_media_chunk(h: &mut dyn Host, request_id: &str, sequence: u64, media: &Value) -> Outcome<()> {
    let event = json!({
        "requestId": request_id,
        "sequence": sequence,
        "type": "media_chunk",
        "payload": { "role": "audio", "media": media }
    });
    port(h, "host-stream.emit-event", json!({"event": event}))?;
    Ok(())
}

fn complete_http(h: &mut dyn Host, url: &str, body: &Value) -> Outcome<String> {
    let body_bytes = serde_json::to_vec(body).map_err(|_| malformed())?;
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
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "Companion synthesis failed; no retry was attempted",
            vendor: Some(status.to_string()),
            request: None,
        });
    }
    Ok(text(&response["payload"], "handle")?.to_string())
}

fn synthesize_complete(h: &mut dyn Host, submit: &OperationSubmitV2, input: &Value, mode_name: &str, text_value: &str, seed: Option<u64>) -> Outcome<Value> {
    let origin = origin_of(submit)?;
    let (body, temp_id) = build_companion_body(h, submit, input, mode_name, text_value, seed, "complete")?;
    let url = format!("{}/v1/audio/speech", origin.trim_end_matches('/'));
    let handle = match complete_http(h, &url, &body) {
        Ok(v) => { if let Some(tid) = temp_id.as_deref() { release_temp(h, origin, tid); } v }
        Err(e) => { if let Some(tid) = temp_id.as_deref() { release_temp(h, origin, tid); } return Err(e); }
    };
    let media = media_from_handle(h, &handle)?;
    let applied = if let Some(seed_val) = input.get("seed") {
        json!({ "seed": seed_val.clone() })
    } else {
        Value::Null
    };
    Ok(json!({
        "kind":"completed",
        "vendorTaskId": null,
        "appliedParams": applied,
        "outputs":[{"kind":"media","role":"audio","media": media}],
        "data": null,
        "usage": null
    }))
}


fn wav_wrap_pcm_s16le(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let file_len = 36u32 + data_len;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&file_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * 2;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

fn bytes_from_value(v: &Value) -> Outcome<Vec<u8>> {
    let arr = v.as_array().ok_or_else(malformed)?;
    let mut out = Vec::with_capacity(arr.len());
    for b in arr {
        let n = b.as_u64().ok_or_else(malformed)?;
        if n > 255 {
            return Err(malformed());
        }
        out.push(n as u8);
    }
    Ok(out)
}

fn synthesize_stream(h: &mut dyn Host, submit: &OperationSubmitV2, request_id: &str, input: &Value, mode_name: &str, text_value: &str, seed: Option<u64>) -> Outcome<Value> {
    let origin = origin_of(submit)?;
    let (body, temp_id) = build_companion_body(h, submit, input, mode_name, text_value, seed, "stream")?;
    let url = format!("{}/v1/audio/speech", origin.trim_end_matches('/'));
    let body_bytes = serde_json::to_vec(&body).map_err(|_| malformed())?;
    let plan = json!({
        "plan":{
            "method":"POST",
            "url":url,
            "headers":[{"name":"Content-Type","value":"application/json; charset=utf-8"}],
            "generatedHeaders":[],
            "body":{"kind":"bytes","bytes": body_bytes},
            "sink":{"kind":"buffer","maxBytes": 1048576u64},
            "timeoutMs": 600000u64
        },
        "credential": Value::Null
    });
    let opened = match port(h, "host-http-stream.open", plan) {
        Ok(v) => v,
        Err(e) => {
            if let Some(tid) = temp_id.as_deref() { release_temp(h, origin, tid); }
            return Err(e);
        }
    };
    let status = number(&opened, "status")?;
    if !(200..300).contains(&status) {
        let _ = port(h, "host-http-stream.close", json!({"streamId": opened.get("streamId").cloned().unwrap_or(json!(""))}));
        if let Some(tid) = temp_id.as_deref() { release_temp(h, origin, tid); }
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "Companion stream open failed; no retry was attempted",
            vendor: Some(status.to_string()),
            request: None,
        });
    }
    let stream_id = text(&opened, "streamId")?.to_string();
    let sample_rate = header_value(&opened["headers"], "X-Sample-Rate")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(24000);
    let create = port(h, "host-media.create", json!({"kind":"audio","mimeHint":"audio/wav"}))?;
    let handle = match &create {
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get("handle")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .ok_or_else(malformed)?,
        _ => return Err(malformed()),
    };
    if handle.is_empty() {
        return Err(malformed());
    }

    let mut sequence: u64 = 1;
    let mut total: u64 = 0;
    let mut pcm: Vec<u8> = Vec::new();
    loop {
        let chunk = port(h, "host-http-stream.read", json!({"streamId": stream_id, "maxBytes": 65536u32}))?;
        let eof = chunk.get("eof").and_then(Value::as_bool).unwrap_or(false);
        let raw = bytes_from_value(&chunk["bytes"])?;
        if !raw.is_empty() {
            total = total.saturating_add(raw.len() as u64);
            pcm.extend_from_slice(&raw);
            // Emit sparsely to keep guest fuel/memory within public limits.
            if sequence == 1 || eof || total % 65536 < raw.len() as u64 {
                emit_progress(h, request_id, sequence, &format!("streamed {total} pcm bytes"))?;
                sequence += 1;
            }
        }
        if eof {
            break;
        }
    }
    if let Some(tid) = temp_id.as_deref() { release_temp(h, origin, tid); }
    let _ = port(h, "host-http-stream.close", json!({"streamId": stream_id}));
    let wav = wav_wrap_pcm_s16le(&pcm, sample_rate);
    // Single media write after EOF (JSON byte arrays are expensive in wasm).
    const WRITE_CHUNK: usize = 16384;
    let mut offset = 0usize;
    while offset < wav.len() {
        let end = (offset + WRITE_CHUNK).min(wav.len());
        let bytes_json: Vec<Value> = wav[offset..end].iter().map(|b| json!(*b)).collect();
        port(h, "host-media.write", json!({"handle": &handle, "bytes": bytes_json}))?;
        offset = end;
    }
    port(h, "host-media.finish", json!({"handle": handle}))?;
    let media = media_from_handle(h, &handle)?;
    emit_media_chunk(h, request_id, sequence, &media)?;
    let applied = if let Some(seed_val) = input.get("seed") {
        json!({ "seed": seed_val.clone() })
    } else {
        Value::Null
    };
    Ok(json!({
        "kind":"completed",
        "vendorTaskId": null,
        "appliedParams": applied,
        "outputs":[{"kind":"media","role":"audio","media": media}],
        "data": null,
        "usage": null
    }))
}

fn synthesize(h: &mut dyn Host, submit: &OperationSubmitV2, request_id: &str) -> Outcome<Value> {
    let input = Value::Object(submit.r#input.clone().into_iter().collect());
    let mode_name = mode(&input)?;
    let delivery_name = delivery(&input)?;
    let text_value = input_text(&input)?;
    let seed = optional_seed(&input)?;
    if delivery_name == "stream" {
        synthesize_stream(h, submit, request_id, &input, mode_name, &text_value, seed)
    } else {
        synthesize_complete(h, submit, &input, mode_name, &text_value, seed)
    }
}

fn batch(h: &mut dyn Host, submit: &OperationSubmitV2) -> Outcome<Value> {
    let input = Value::Object(submit.r#input.clone().into_iter().collect());
    let items = input.get("items").and_then(Value::as_array)
        .ok_or_else(|| fail("INVALID_INPUT", "items is required"))?;
    if items.is_empty() {
        return Err(fail("INVALID_INPUT", "items must be non-empty"));
    }
    let origin = origin_of(submit)?;
    let url = format!("{}/v1/audio/speech", origin.trim_end_matches('/'));
    let mut outputs = Vec::new();
    let mut result_items = Vec::new();
    for item in items {
        let item_id = item.get("itemId").and_then(Value::as_str)
            .ok_or_else(|| fail("INVALID_INPUT", "itemId is required"))?;
        let item_input = item.get("input").ok_or_else(|| fail("INVALID_INPUT", "item.input is required"))?;
        let mode_name = mode(item_input)?;
        let text_value = input_text(item_input)?;
        let seed = optional_seed(item_input)?;
        match build_companion_body(h, submit, item_input, mode_name, &text_value, seed, "complete")
            .and_then(|(body, temp_id)| {
                let result = complete_http(h, &url, &body);
                if let Some(tid) = temp_id.as_deref() { release_temp(h, origin, tid); }
                result
            })
            .and_then(|handle| media_from_handle(h, &handle))
        {
            Ok(media) => {
                let media_id = media.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                result_items.push(json!({
                    "itemId": item_id,
                    "status": "succeeded",
                    "mediaId": media_id
                }));
                outputs.push(json!({"kind":"media","role":"audio","media": media}));
            }
            Err(e) => {
                result_items.push(json!({
                    "itemId": item_id,
                    "status": "failed",
                    "error": {
                        "code": e.code,
                        "message": e.message,
                        "retryable": false
                    }
                }));
            }
        }
    }
    Ok(json!({
        "kind":"completed",
        "vendorTaskId": null,
        "appliedParams": Value::Null,
        "outputs": outputs,
        "data": { "schemaVersion": 1, "items": result_items },
        "usage": null
    }))
}

pub struct BreezeSpeechAdapter;
impl Adapter for BreezeSpeechAdapter {
    fn invoke(request: Invocation, host: &mut dyn Host) -> Response {
        let operation = serde_json::to_value(&request.r#operation).expect("closed operation");
        match operation.as_str().expect("operation string") {
            "capabilities" => guest::completed_response(&request, &capabilities()).expect("serializable"),
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
                        return failure(&request, fail("INVALID_INPUT", "Submit does not match the public request contract"))
                    }
                };
                if input.r#capability_key != "audio.synthesize" {
                    return failure(&request, fail("INVALID_INPUT", "capabilityKey must be audio.synthesize"));
                }
                match input.r#operation_id.as_str() {
                    "std/speech.synthesize" => finish(&request, synthesize(host, &input, &request.r#request_id)),
                    "std/speech.batch" => finish(&request, batch(host, &input)),
                    _ => failure(
                        &request,
                        fail("UNSUPPORTED_FEATURE", "This C-pack declares synthesize and batch only"),
                    ),
                }
            }
            _ => failure(
                &request,
                fail("UNSUPPORTED_FEATURE", "This synchronous C-pack does not implement the selected operation"),
            ),
        }
    }
}
bgx_adapter_v2_sdk::export_adapter!(BreezeSpeechAdapter);
