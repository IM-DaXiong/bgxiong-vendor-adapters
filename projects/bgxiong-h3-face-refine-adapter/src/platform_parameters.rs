use super::*;

/// Read only classes in the actual submitted graph, once each. Keep the shared
/// HTTP buffer bound; unrelated installed extensions must not affect submission.
pub(super) fn definitions(h: &mut dyn Host, base: &str, graph: &Value) -> Result<Value> {
    let mut definitions = serde_json::Map::new();
    for node in graph.as_object().ok_or_else(|| invalid("workflow must be object"))?.values() {
        let class = node["class_type"].as_str().ok_or_else(|| invalid("node class missing"))?;
        if definitions.contains_key(class) { continue; }
        let mut segment = String::new();
        for byte in class.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') { segment.push(byte as char); }
            else { segment.push_str(&format!("%{byte:02X}")); }
        }
        let response = http_json(h, "GET", &format!("{}/object_info/{segment}", base.trim_end_matches('/')), None)?;
        let definition = response.get(class).filter(|value| value.is_object())
            .ok_or_else(|| fail("RESOURCE_UNAVAILABLE", format!("node unavailable: {class}")))?;
        definitions.insert(class.to_owned(), definition.clone());
    }
    Ok(Value::Object(definitions))
}

pub(super) fn prepare(graph: &mut Value, info: &Value) -> Result<()> {
    for node in graph.as_object_mut().ok_or_else(|| invalid("workflow must be object"))?.values_mut() {
        let class = node["class_type"].as_str().ok_or_else(|| invalid("node class missing"))?.to_owned();
        let definition = info.get(&class).ok_or_else(|| fail("RESOURCE_UNAVAILABLE", format!("node unavailable: {class}")))?;
        let inputs = node["inputs"].as_object_mut().ok_or_else(|| invalid("inputs missing"))?;
        if let Some(required) = definition.pointer("/input/required").and_then(Value::as_object) {
            for key in required.keys() { if !inputs.contains_key(key) { return Err(invalid(&format!("{class}: missing {key}"))); } }
        }
        for (key, value) in inputs.iter_mut() {
            let spec = definition["input"]["required"].get(key).or_else(|| definition["input"]["optional"].get(key));
            let Some(choices) = spec.and_then(|s| s[0].as_array()) else { continue };
            if value.is_array() { continue; }
            if key == "cut_detection" && *value == "auto" { *value = json!("auto (pyscenedetect)"); }
            if key == "detector" {
                if let Some(path) = value.as_str() {
                    let normalized = path.replace('\\', "/");
                    let matches: Vec<_> = choices.iter().filter(|c| c.as_str().is_some_and(|p| p.replace('\\', "/") == normalized)).collect();
                    if matches.len() != 1 { return Err(invalid(&format!("{class}: detector missing or ambiguous"))); }
                    *value = matches[0].clone();
                }
            }
            if !choices.contains(value) { return Err(invalid(&format!("{class}: unsupported {key}"))); }
        }
    }
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn reads_only_unique_graph_classes_and_rejects_missing_definition() {
        struct Nodes { urls: Vec<String>, missing: bool }
        impl Host for Nodes {
            fn call(&mut self, call: bgx_adapter_v2_sdk::HostPortCallV2) -> core::result::Result<Value, host::HostError> {
                let call = serde_json::to_value(call).unwrap();
                let url = call["args"]["plan"]["url"].as_str().unwrap().to_owned();
                assert_eq!(call["args"]["plan"]["method"], "GET");
                self.urls.push(url);
                let body = if self.missing { json!({}) } else { json!({"C /汉": {"input": {"required": {}}}}) };
                Ok(json!({"status":200,"payload":{"bytes":serde_json::to_vec(&body).unwrap()}}))
            }
        }
        let graph = json!({"1":{"class_type":"C /汉","inputs":{}},"2":{"class_type":"C /汉","inputs":{}}});
        let mut host = Nodes { urls: vec![], missing: false };
        let info = definitions(&mut host, "http://localhost/", &graph).unwrap();
        assert!(info.get("C /汉").is_some());
        assert_eq!(host.urls, vec!["http://localhost/object_info/C%20%2F%E6%B1%89"]);
        host.missing = true;
        assert_eq!(definitions(&mut host, "http://localhost", &graph).unwrap_err().code, "RESOURCE_UNAVAILABLE");
    }
    #[test] fn resolves_paths_and_rejects_missing_model() {
        let info = json!({"C":{"input":{"required":{"detector":[["bbox\\face.pt"]],"cut_detection":[["none","auto (pyscenedetect)"]]}}}});
        let mut graph = json!({"1":{"class_type":"C","inputs":{"detector":"bbox/face.pt","cut_detection":"auto"}}});
        prepare(&mut graph, &info).unwrap();
        assert_eq!(graph["1"]["inputs"]["detector"], "bbox\\face.pt");
        assert_eq!(graph["1"]["inputs"]["cut_detection"], "auto (pyscenedetect)");
        graph["1"]["inputs"]["detector"] = json!("other.pt");
        assert!(prepare(&mut graph, &info).is_err());
    }
}
