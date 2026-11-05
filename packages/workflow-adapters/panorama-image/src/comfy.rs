//! Local ComfyUI HTTP: upload / prompt / queue / history / view. No task DB.
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;

use alloc::{format, string::String, string::ToString, vec, vec::Vec};
use bgx_adapter_v2_sdk::{host, Host};
use serde_json::{json, Value};

use crate::mapping::Failure;

/// Published schema-policy.json limits.maxBufferedHttpBytes for SDK 2.x.
fn max_buffer() -> u64 {
    1_048_576
}

pub struct Ctx<'a> {
    pub config: &'a Value,
    pub budget_ms: u64,
}

pub fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, Failure> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Failure {
            code: "UPSTREAM_ERROR",
            message: "ComfyUI returned an unexpected structured response".into(),
        })
}

fn malformed() -> Failure {
    Failure {
        code: "UPSTREAM_ERROR",
        message: "ComfyUI returned an unexpected structured response".into(),
    }
}

pub fn port(h: &mut dyn Host, method: &str, args: Value) -> Result<Value, Failure> {
    let value = host::call(h, method, &args).map_err(|e| {
        let v = serde_json::to_value(&e).expect("public host error");
        let code = match v["code"].as_str() {
            Some("TIMEOUT") => "TIMEOUT",
            Some("INVALID_INPUT") => "INVALID_INPUT",
            Some("PERMISSION_DENIED") => "PERMISSION_DENIED",
            _ => "RESOURCE_UNAVAILABLE",
        };
        Failure {
            code,
            message: format!("{method}: {}", host::diagnostic(&e)),
        }
    })?;
    macro_rules! decode {
        ($ty:ty) => {{
            let typed: $ty = serde_json::from_value(value).map_err(|_| Failure::invalid("Managed response violates the frozen SDK DTO"))?;
            Ok(serde_json::to_value(typed).expect("public DTO"))
        }};
    }
    match method {
        "host-http.execute" => decode!(bgx_adapter_v2_sdk::HostPortCallV2HttpResponse),
        "host-media.info" => decode!(bgx_adapter_v2_sdk::HostPortCallV2MediaStat),
        "host-checkpoint.read" => decode!(Option<bgx_adapter_v2_sdk::HostPortCallV2CheckpointRead>),
        "host-checkpoint.write" => decode!(bgx_adapter_v2_sdk::HostPortCallV2CheckpointWrite),
        _ => Err(Failure::invalid("Unsupported managed port")),
    }
}

fn base(config: &Value) -> Result<String, Failure> {
    let b = config
        .get("endpointBaseUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::invalid("configuration.endpointBaseUrl is required"))?;
    if !(b.starts_with("http://") || b.starts_with("https://")) || b.ends_with('/') {
        return Err(Failure::invalid(
            "endpointBaseUrl must be an absolute http(s) origin without trailing slash",
        ));
    }
    Ok(b.to_string())
}

fn credential(config: &Value) -> Value {
    match config.get("credentialRef") {
        Some(Value::Null) | None => Value::Null,
        Some(v) => v.clone(),
    }
}

fn plan(
    ctx: &Ctx,
    method: &str,
    path: &str,
    body: Value,
    headers: Vec<Value>,
    sink: Value,
) -> Result<Value, Failure> {
    let url = format!("{}{}", base(ctx.config)?, path);
    Ok(json!({
        "plan": {
            "method": method,
            "url": url,
            "headers": headers,
            "generatedHeaders": [],
            "body": body,
            "sink": sink,
            "timeoutMs": ctx.budget_ms
        },
        "credential": credential(ctx.config)
    }))
}

fn buffered_json(response: &Value) -> Option<Value> {
    let bytes: Vec<u8> = serde_json::from_value(response["payload"]["bytes"].clone()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn exchange(
    h: &mut dyn Host,
    ctx: &Ctx,
    method: &str,
    path: &str,
    body: Value,
    headers: Vec<Value>,
    acceptance_unknown_on_transport: bool,
) -> Result<Value, Failure> {
    let args = plan(
        ctx,
        method,
        path,
        body,
        headers,
        json!({"kind": "buffer", "maxBytes": max_buffer()}),
    )?;
    let response = port(h, "host-http.execute", args).map_err(|e| {
        if acceptance_unknown_on_transport {
            Failure {
                code: "ACCEPTANCE_UNKNOWN",
                message: "Transport lost before a durable Comfy reply; do not resubmit".into(),
            }
        } else {
            e
        }
    })?;
    let status = response
        .get("status")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            if acceptance_unknown_on_transport {
                Failure {
                    code: "ACCEPTANCE_UNKNOWN",
                    message: "Submit/upload response incomplete; do not resubmit".into(),
                }
            } else {
                malformed()
            }
        })?;
    if response["payload"]["kind"] != "buffered" { return Err(Failure::invalid("HTTP payload does not match buffered sink")); }
    let value = buffered_json(&response).ok_or_else(|| {
        if acceptance_unknown_on_transport {
            Failure {
                code: "ACCEPTANCE_UNKNOWN",
                message: "Response is not JSON; do not resubmit".into(),
            }
        } else {
            malformed()
        }
    })?;
    if status != 200 {
        return Err(Failure {
            code: if (400..500).contains(&status) {
                "INVALID_INPUT"
            } else {
                "UPSTREAM_ERROR"
            },
            message: format!("ComfyUI HTTP {status}"),
        });
    }
    Ok(value)
}

fn large_json(h: &mut dyn Host, ctx: &Ctx, path: &str) -> Result<Value, Failure> {
    exchange(
        h,
        ctx,
        "GET",
        path,
        json!({"kind": "none"}),
        vec![],
        false,
    )
}

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
    if name.is_empty() {
        return Err(malformed());
    }
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
    let id = text(&v, "prompt_id").map_err(|_| Failure {
        code: "ACCEPTANCE_UNKNOWN",
        message: "Submit response has no prompt_id; do not resubmit".into(),
    })?;
    if id.is_empty() {
        return Err(Failure {
            code: "ACCEPTANCE_UNKNOWN",
            message: "Submit response has an empty prompt_id; do not resubmit".into(),
        });
    }
    if v.get("node_errors")
        .and_then(Value::as_object)
        .is_some_and(|o| !o.is_empty())
    {
        return Err(malformed());
    }
    Ok(id.to_string())
}

pub enum Remote {
    Queued,
    Running,
    Succeeded(Vec<(String, String, String)>),
    Failed(Option<String>),
    Lost,
}

fn listed(queue: &Value, key: &str, id: &str) -> bool {
    queue
        .get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter().any(|item| {
                item.get(1)
                    .and_then(Value::as_str)
                    == Some(id)
            })
        })
        .unwrap_or(false)
}

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
    .map_err(|mut e| {
        if e.code == "INVALID_INPUT" {
            e.code = "UPSTREAM_ERROR";
        }
        e
    })?;
    let status = response["status"].as_u64().ok_or_else(malformed)?;
    if status != 200 {
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: format!("ComfyUI view HTTP {status}"),
        });
    }
    if response["payload"]["kind"] != "media" { return Err(Failure::invalid("HTTP payload does not match media sink")); }
    let handle = text(&response["payload"], "handle")?;
    let stat = port(h, "host-media.info", json!({"handle": handle})).map_err(|mut e| {
        if e.code == "INVALID_INPUT" {
            e.code = "UPSTREAM_ERROR";
        }
        e
    })?;
    if stat["kind"] != "image" {
        return Err(Failure {
            code: "UPSTREAM_ERROR",
            message: "ComfyUI output is not an image".into(),
        });
    }
    let w = stat["width"].as_u64().ok_or_else(malformed)?;
    let height = stat["height"].as_u64().ok_or_else(malformed)?;
    if w == 0 || height == 0 || height.checked_mul(2) != Some(w) {
        return Err(Failure::invalid("Panorama requires positive 2:1 image geometry"));
    }
    Ok(json!({
        "kind": "media",
        "role": "result",
        "media": {
            "id": stat["id"],
            "kind": "image",
            "handle": handle,
            "mime": stat["mime"],
            "fileName": stat["fileName"],
            "byteLength": stat["byteLength"]
        }
    }))
}

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
    fn queue_reads_comfy_tuple_prompt_id() {
        let queue=serde_json::json!({"queue_running":[[7,"original-id",{},{}]],"queue_pending":[]});
        assert!(super::listed(&queue,"queue_running","original-id"));
        assert!(!super::listed(&queue,"queue_running","other-id"));
    }
    #[test]
    fn managed_ports_reject_incomplete_nullable_stat_and_wrong_payload() {
        let bad=serde_json::json!({"kind":"image","mime":"image/png","byteLength":"8","sha256":"a".repeat(64),"id":"a","fileName":"a.png"});
        assert!(serde_json::from_value::<bgx_adapter_v2_sdk::HostPortCallV2MediaStat>(bad).is_err());
        assert!(serde_json::from_value::<bgx_adapter_v2_sdk::HostPortCallV2HttpResponse>(serde_json::json!({"status":200,"payload":{"bytes":[]}})).is_err());
    }
    #[test]
    fn percent_encoding() {
        assert_eq!(super::encode("a b.png"), "a%20b.png");
    }
}
