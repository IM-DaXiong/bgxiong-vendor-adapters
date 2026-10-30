use super::*;

// Preserve upstream structure while omitting echoed inputs, credentials and binary content.
// Diagnostics travel in the existing error message, which the host records durably.
fn sanitize(value: &Value, depth: usize) -> Value {
    if depth > 8 { return json!("[depth limit]"); }
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, v) in map.iter().take(32) {
                let safe = matches!(key.as_str(), "error" | "errors" | "node_errors" | "type" | "message" | "details" | "extra_info" | "class_type" | "input_name" | "input_type" | "received_type" | "linked_node" | "dependent_outputs") || key.chars().all(|c| c.is_ascii_digit());
                out.insert(key.clone(), if safe { sanitize(v, depth + 1) } else { json!("[redacted]") });
            }
            Value::Object(out)
        }
        Value::Array(values) => Value::Array(values.iter().take(16).map(|v| sanitize(v, depth + 1)).collect()),
        Value::String(s) => {
            let lower = s.to_ascii_lowercase();
            if s.len() > 512 || ["bearer ", "token", "password", "secret", "api_key", "apikey", "http://", "https://", "data:", "base64"].iter().any(|needle| lower.contains(needle)) {
                json!("[redacted text]")
            } else { Value::String(s.clone()) }
        }
        _ => value.clone(),
    }
}

pub(super) fn describe(bytes: &[u8]) -> String {
    let hash: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    let clean = serde_json::from_slice::<Value>(bytes).map(|v| sanitize(&v, 0).to_string())
        .unwrap_or_else(|_| "[non-JSON body omitted: may contain credentials/media]".into());
    let truncated = clean.len() > 4096;
    let detail: String = clean.chars().take(1024).collect();
    format!("responseBytes={} sha256={} diagnostic={} truncated={}", bytes.len(), hash, detail, truncated || detail.len() < clean.len())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn retains_node_reason_but_removes_echoed_secrets_and_media() {
        let bytes = serde_json::to_vec(&json!({"error":{"type":"prompt_outputs_failed_validation","message":"Prompt outputs failed validation"},"node_errors":{"1":{"errors":[{"type":"value_not_in_list","message":"Value not in list","extra_info":{"input_name":"detector","received_value":"private.png","token":"super-secret"}}]}},"media":"base64-private"})).unwrap();
        let d = describe(&bytes);
        assert!(d.contains("value_not_in_list")); assert!(d.contains("detector"));
        assert!(!d.contains("private.png")); assert!(!d.contains("super-secret")); assert!(!d.contains("base64-private"));
        assert!(describe(&vec![b'x'; 10000]).len() < 4400);
    }
}
