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
        "../../../../contract/v2/protocol/schema-policy.json"
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
            alloc::format!("Managed host operation {method} failed: {}", v["message"]),
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
        plan(
            ctx,
            "GET",
            path,
            json!({"kind": "none"}),
            vec![],
            json!({"kind": "media"}),
        )?,
    )
    .map_err(upstream_media_error)?;
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
            return Err(Failure::new(
                "UPSTREAM_ERROR",
                "ComfyUI response is too large",
            ));
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
    )
    .map_err(upstream_media_error)?;
    let status = response["status"].as_u64().ok_or_else(malformed)?;
    if status != 200 {
        return Err(status_error(status, &Value::Null));
    }
    let handle = text(&response["payload"], "handle")?;
    let stat =
        port(h, "host-media.info", json!({"handle": handle})).map_err(upstream_media_error)?;
    if stat["kind"] != "image" {
        return Err(Failure::new(
            "UPSTREAM_ERROR",
            "ComfyUI output is not an image",
        ));
    }
    Ok(
        json!({"kind": "media", "role": "image", "media": {"id": stat["id"], "kind": "image", "handle": handle,
        "mime": stat["mime"], "fileName": stat["fileName"], "byteLength": stat["byteLength"]}}),
    )
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

fn upstream_media_error(mut error: Failure) -> Failure {
    if error.code == "INVALID_INPUT" {
        error.code = "UPSTREAM_ERROR";
    }
    error
}
