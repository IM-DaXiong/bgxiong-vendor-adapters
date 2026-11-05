use super::*;
use std::collections::BTreeMap;
fn input(kind: ModelKind) -> Value {
    let mut i = json!({"inputMode":"multi_image_to_video","prompt":"第一行完整原词\n<image1> 第二行，不截断","referenceImages":(0..kind.max_refs()).map(|n|json!({"mediaId":format!("image-{n}")})).collect::<Vec<_>>(),"durationSeconds":5,"fps":24,"aspectRatio":"16:9","resolution":"mp:0.4"});
    if kind.requires_reference_audio() {
        i["referenceAudios"] = json!([{"mediaId":"audio-0"}]);
        i["advanced"] = json!({"audioUsage":"soundtrack_copy"});
    }
    i
}
fn payload(kind: ModelKind) -> Value {
    let mut media=(0..kind.max_refs()).map(|n|json!({"id":format!("image-{n}"),"kind":"image","handle":format!("image-handle-{n}"),"mime":"image/jpeg","fileName":format!("source-{n}.jpg"),"byteLength":"200"})).collect::<Vec<_>>();
    if kind.requires_reference_audio() {
        media.push(json!({"id":"audio-0","kind":"audio","handle":"audio-handle-0","mime":"audio/wav","fileName":"original.wav","byteLength":"200"}));
    }
    json!({"schemaVersion":2,"modelId":kind.model_id(),"capabilityKey":"video.generate","operationId":kind.operation_id(),"operationVersion":1,"inputSchemaDigest":schema("input",kind.input_schema())["digest"],"input":input(kind),"media":media,"uploads":[],"configuration":{"endpointBaseUrl":"https://www.runninghub.cn","credentialRef":"user-managed","instanceType":"plus"}})
}
fn invocation(op: &str, p: &Value) -> Invocation {
    serde_json::from_value(json!({"protocolVersion":2,"operation":op,"profileId":"test-binding","capabilitySlot":"video","requestId":"test-request","budgetMs":10000,"payloadJson":p.to_string(),"bindingId":"test-binding","negotiatedVersion":2,"enabledFeatures":[]})).unwrap()
}
fn response_json(v: Value) -> Value {
    json!({"status":200,"headers":[],"payload":{"kind":"buffered","bytes":serde_json::to_vec(&v).unwrap()}})
}
fn video_stat(mime: &str, file_name: &str) -> Value {
    json!({"id":"artifact","kind":"video","mime":mime,"fileName":format!("{}.{}", "a".repeat(64),file_name.rsplit('.').next().unwrap()),"byteLength":"200","sha256":"a".repeat(64),"width":864,"height":480,"durationMs":"15080","timestampsMs":null})
}
#[derive(Default)]
struct Fixture {
    stats: BTreeMap<String, Value>,
    calls: Vec<Value>,
    checkpoint: bool,
    submit_response: Option<Value>,
    query_response: Option<Value>,
    deny_download: bool,
    download_stat: Option<Value>,
    omit_port_field: Option<(&'static str, &'static str)>,
}
impl Fixture {
    fn for_payload(p: &Value) -> Self {
        let mut h = Self::default();
        for m in p["media"].as_array().unwrap() {
            let mut stat = m.clone();
            stat.as_object_mut().unwrap().remove("handle");
            stat["sha256"] = json!("b".repeat(64));
            stat["width"] = if m["kind"] == "image" {
                json!(1024)
            } else {
                Value::Null
            };
            stat["height"] = if m["kind"] == "image" {
                json!(768)
            } else {
                Value::Null
            };
            stat["durationMs"] = if m["kind"] == "audio" {
                json!("1000")
            } else {
                Value::Null
            };
            stat["timestampsMs"] = Value::Null;
            h.stats.insert(m["handle"].as_str().unwrap().into(), stat);
        }
        h
    }
}
impl Host for Fixture {
    fn call(
        &mut self,
        c: bgx_adapter_v2_sdk::HostPortCallV2,
    ) -> core::result::Result<Value, host::HostError> {
        let c = serde_json::to_value(c).unwrap();
        let args = &c["args"];
        self.calls.push(c.clone());
        let mut result = match c["method"].as_str().unwrap() {
            "host-media.info" => self.stats[args["handle"].as_str().unwrap()].clone(),
            "host-checkpoint.read" => json!({"revision":"0","valueJson":"{\"revision\":\"0\"}"}),
            "host-checkpoint.write" => {
                self.checkpoint = true;
                json!({"revision":"1"})
            }
            "host-http.execute" => {
                let plan = &args["plan"];
                let url = plan["url"].as_str().unwrap();
                if url.ends_with("/media/upload/binary") {
                    assert_eq!(args["credential"], "user-managed");
                    assert_eq!(plan["generatedHeaders"][0]["kind"], "bearer");
                    assert_eq!(plan["headers"], json!([]));
                    let part = &plan["body"]["parts"][0];
                    assert_eq!(part["kind"], "file");
                    let original = part["fileName"].as_str().unwrap();
                    response_json(
                        json!({"code":0,"data":{"fileName":format!("openapi/{original}"),"download_url":"https://example.invalid/not-for-comfy"}}),
                    )
                } else if url.contains("/run/ai-app/") {
                    response_json(self.submit_response.clone().unwrap_or_else(||json!({"taskId":"task-1","status":"RUNNING","errorCode":"","errorMessage":""})))
                } else if url.ends_with("/query") {
                    response_json(self.query_response.clone().unwrap_or_else(||json!({"taskId":"task-1","status":"RUNNING","errorCode":"","errorMessage":""})))
                } else {
                    assert_eq!(plan["method"], "GET");
                    assert!(args["credential"].is_null());
                    assert_eq!(plan["generatedHeaders"], json!([]));
                    if self.deny_download {
                        return Err(serde_json::from_value(json!({"code":"PERMISSION_DENIED","message":"Output origin denied by explicit grant"})).unwrap());
                    }
                    self.stats.insert(
                        "download".into(),
                        self.download_stat
                            .clone()
                            .unwrap_or_else(|| video_stat("video/mp4", "result.mp4")),
                    );
                    json!({"status":200,"headers":[{"name":"Content-Type","value":"video/mp4"}],"payload":{"kind":"media","handle":"download"}})
                }
            }
            other => panic!("Unexpected fixture effect: {other}"),
        };
        if let Some((method, field)) = self.omit_port_field {
            if c["method"] == method {
                result.as_object_mut().unwrap().remove(field);
            }
        }
        Ok(result)
    }
}
#[test]
fn every_workflow_has_its_declared_fixed_media_and_native_timing() {
    let caps = capabilities();
    let _: bgx_adapter_v2_sdk::RuntimeCapsV2 =
        serde_json::from_value(caps.clone()).expect("public typed capability contract");
    let contracts = caps["capabilityContracts"].as_array().unwrap();
    assert_eq!(contracts.len(), 7);
    for (kind, c) in MODELS.iter().zip(contracts) {
        assert_eq!(c["modelIds"][0], kind.model_id());
        let schema: Value = serde_json::from_str(kind.input_schema()).unwrap();
        assert_eq!(schema["properties"]["fps"]["const"], 24);
        assert_eq!(
            schema["properties"]["referenceImages"]["minItems"],
            kind.max_refs()
        );
        assert_eq!(
            schema["properties"]["referenceImages"]["maxItems"],
            kind.max_refs()
        );
        assert_eq!(
            schema["properties"].get("referenceAudios").is_some(),
            kind.requires_reference_audio()
        );
        assert_eq!(
            c["operations"][0]["mediaSlots"].as_array().unwrap().len(),
            if kind.requires_reference_audio() {
                2
            } else {
                1
            }
        );
    }
}
#[test]
fn all_seven_submit_exact_original_assets_prompt_and_workflow_specific_pins() {
    for kind in MODELS {
        let p = payload(kind);
        let mut h = Fixture::for_payload(&p);
        let a = submit(&mut h, &invocation("submit", &p), &p).unwrap();
        assert_eq!(a["kind"], "accepted");
        assert!(h.checkpoint);
        assert_eq!(a["resumeToken"]["appId"], kind.app_id());
        let submits = h
            .calls
            .iter()
            .filter(|v| {
                v["method"] == "host-http.execute"
                    && v["args"]["plan"]["url"]
                        .as_str()
                        .unwrap()
                        .contains("/run/ai-app/")
            })
            .collect::<Vec<_>>();
        assert_eq!(submits.len(), 1);
        let plan = &submits[0]["args"]["plan"];
        assert_eq!(plan["url"], kind.submit_url());
        let bytes: Vec<u8> = serde_json::from_value(plan["body"]["bytes"].clone()).unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["instanceType"], "plus");
        assert!(body.get("workflow").is_none());
        let nodes = body["nodeInfoList"].as_array().unwrap();
        let widgets = kind.widgets();
        assert_eq!(nodes.iter().find(|v| v["nodeId"] == widgets.duration_node && v["fieldName"] == widgets.duration_field).unwrap()["fieldValue"], "5");
        assert_eq!(nodes.iter().find(|v| v["nodeId"] == widgets.aspect_node && v["fieldName"] == "aspect_ratio").unwrap()["fieldValue"], "16:9 (Widescreen)");
        assert_eq!(nodes.iter().find(|v| v["nodeId"] == widgets.resolution_node && v["fieldName"] == "megapixels").unwrap()["fieldValue"], "0.4");
        assert!(!nodes.iter().any(|v| v["fieldName"] == "noise_seed"));
        assert_eq!(
            nodes
                .iter()
                .find(|v| v["nodeId"] == widgets.prompt_node
                    && v["fieldName"] == widgets.prompt_field)
                .unwrap()["fieldValue"],
            p["input"]["prompt"]
        );
        for (n, id) in kind.ref_node_ids().iter().enumerate() {
            assert!(nodes.iter().any(
                |v| v["nodeId"] == *id && v["fieldValue"] == format!("openapi/source-{n}.jpg")
            ));
        }
        for (node, field, value) in kind.weight_pins() {
            assert!(nodes.iter().any(|v| v["nodeId"] == *node
                && v["fieldName"] == *field
                && v["fieldValue"] == *value));
        }
        if kind.requires_reference_audio() {
            assert!(nodes
                .iter()
                .any(|v| v["nodeId"] == "142" && v["fieldValue"] == "openapi/original.wav"));
        }
        for (tier, value) in [("mp:0.25", "0.25"), ("mp:0.4", "0.4"), ("mp:0.6", "0.6"), ("mp:1", "1")] {
            let mut chosen = p["input"].clone();
            chosen["resolution"] = json!(tier);
            chosen["durationSeconds"] = json!(15);
            chosen["aspectRatio"] = json!("9:16");
            let images = (0..kind.max_refs()).map(|n| format!("openapi/source-{n}.jpg")).collect::<Vec<_>>();
            let audios = if kind.requires_reference_audio() { vec![String::from("openapi/original.wav")] } else { vec![] };
            let mapped = node_info(kind, &chosen, &images, &audios).unwrap();
            let mapped = mapped.as_array().unwrap();
            assert_eq!(mapped.iter().find(|v| v["nodeId"] == widgets.resolution_node && v["fieldName"] == "megapixels").unwrap()["fieldValue"], value);
            assert_eq!(mapped.iter().find(|v| v["nodeId"] == widgets.duration_node && v["fieldName"] == widgets.duration_field).unwrap()["fieldValue"], "15");
            assert_eq!(mapped.iter().find(|v| v["nodeId"] == widgets.aspect_node && v["fieldName"] == "aspect_ratio").unwrap()["fieldValue"], "9:16 (Portrait Widescreen)");
        }
    }
}
#[test]
fn invalid_submissions_stop_before_upload_or_remote_acceptance() {
    for kind in MODELS {
        for mutation in 0..8 {
            let mut p = payload(kind);
            match mutation {
                0 => p["input"]["durationSeconds"] = json!(16),
                1 => p["input"]["fps"] = json!(25),
                2 => p["input"]["resolution"] = json!("mp:0.3"),
                3 => {
                    p["input"]["referenceImages"].as_array_mut().unwrap().pop();
                }
                4 => p["input"]["seed"] = json!(42),
                5 => p["input"]["advanced"] = json!({"audioUsage":"voice_timbre"}),
                6 => p["input"]["resolution"] = json!("0.4"),
                _ => p["input"]["undeclared"] = json!(true),
            }
            let mut h = Fixture::for_payload(&p);
            assert!(submit(&mut h, &invocation("submit", &p), &p).is_err());
            assert!(h.calls.is_empty());
        }
    }
}
#[test]
fn query_downloads_original_without_credentials_and_preserves_remote_identity() {
    let kind = ModelKind::Slot3;
    let p = payload(kind);
    let mut h = Fixture::for_payload(&p);
    let a = submit(&mut h, &invocation("submit", &p), &p).unwrap();
    let mut q = p.clone();
    q.as_object_mut().unwrap().remove("input");
    q["vendorTaskId"] = a["vendorTaskId"].clone();
    q["resumeToken"] = a["resumeToken"].clone();
    h.query_response = Some(
        json!({"taskId":"task-1","status":"SUCCESS","errorCode":"","results":[{"nodeId":"original-output","outputType":"mp4","url":"https://rh-comfyui01.tos-cn-beijing.volces.com/original.mp4"}],"usage":{"consumeCoins":"0.25","taskCostTime":275}}),
    );
    let result = query(&mut h, &invocation("query", &q), &q).unwrap();
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["data"]["appId"], kind.app_id());
    assert_eq!(result["outputs"][0]["media"]["mime"], "video/mp4");
    assert_eq!(result["usage"]["measurements"][0]["amount"], "0.25");
    let _: bgx_adapter_v2_sdk::UsageV2 = serde_json::from_value(result["usage"].clone()).unwrap();
    for measurement in result["usage"]["measurements"].as_array().unwrap() {
        assert_eq!(
            measurement["metric"]
                .as_str()
                .unwrap()
                .split_once('/')
                .unwrap()
                .0,
            PLUGIN
        );
        assert_eq!(
            measurement["unit"]
                .as_str()
                .unwrap()
                .split_once('/')
                .unwrap()
                .0,
            PLUGIN
        );
    }
    let mut query_results = vec![result.clone()];
    let mut managed_stats = Vec::new();
    let download = h
        .calls
        .iter()
        .find(|v| v["args"]["plan"]["method"] == "GET")
        .unwrap();
    assert!(download["args"]["credential"].is_null());
    assert_eq!(download["args"]["plan"]["generatedHeaders"], json!([]));
    // API and HTTP both advertise MP4. Only the SDK's probed original decides
    // the media type/name; accepting video does not mean transcoding a container.
    for observed in [
        include_str!("../fixtures/sdk-managed-video-quicktime.json"),
        include_str!("../fixtures/sdk-managed-video-mp4.json"),
    ] {
        let stat: Value = serde_json::from_str(observed).unwrap();
        h.download_stat = Some(stat.clone());
        let original = query(&mut h, &invocation("query", &q), &q).unwrap();
        assert_eq!(original["outputs"][0]["media"]["mime"], stat["mime"]);
        assert_eq!(
            original["outputs"][0]["media"]["fileName"],
            stat["fileName"]
        );
        assert_eq!(
            original["outputs"][0]["media"]["byteLength"],
            stat["byteLength"]
        );
        managed_stats.push(stat);
        query_results.push(original);
    }
    for (mime, name) in [
        ("video/quicktime", "original.mov"),
        ("video/webm", "original.webm"),
        ("video/x-matroska", "original.mkv"),
        ("video/x-msvideo", "original.avi"),
        ("video/mpeg", "original.mpeg"),
    ] {
        h.download_stat = Some(video_stat(mime, name));
        let original = query(&mut h, &invocation("query", &q), &q).unwrap();
        assert_eq!(original["status"], "succeeded");
        assert_eq!(original["outputs"][0]["media"]["mime"], mime);
        assert_eq!(
            original["outputs"][0]["media"]["fileName"],
            h.download_stat.as_ref().unwrap()["fileName"]
        );
        query_results.push(original);
    }
    h.query_response.as_mut().unwrap()["results"][0]["outputType"] =
        json!("unreliable-upstream-label");
    assert_eq!(
        query(&mut h, &invocation("query", &q), &q).unwrap()["outputs"][0]["media"]["mime"],
        "video/mpeg"
    );
    for (field, bad) in [
        ("kind", json!("file")),
        ("mime", json!("text/html")),
        ("byteLength", json!("0")),
        ("durationMs", Value::Null),
        ("width", json!(0)),
        ("height", json!(0)),
        ("sha256", json!("not-a-sha")),
        ("fileName", json!("../fake.mov")),
    ] {
        let mut bad_stat = video_stat("video/quicktime", "original.mov");
        bad_stat[field] = bad;
        h.download_stat = Some(bad_stat);
        let rejected = query(&mut h, &invocation("query", &q), &q).unwrap_err();
        assert_eq!(rejected.code, "UPSTREAM_ERROR");
        assert!(rejected.message.contains("task-1"));
    }
    h.download_stat = Some(video_stat("video/quicktime", "original.mov"));
    if let Ok(file) = std::env::var("RUNNINGHUB_CONTRACT_FIXTURES") {
        std::fs::write(file, json!({"capabilities":capabilities(),"usage":result["usage"],"queryResults":query_results,"managedVideoStats":managed_stats}).to_string()).unwrap();
    }
    h.deny_download = true;
    let denied = query(&mut h, &invocation("query", &q), &q).unwrap_err();
    assert_eq!(denied.code, "PERMISSION_DENIED");
    assert!(denied.retryable);
    assert!(denied.message.contains("task-1"));
    h.deny_download = false;
    assert_eq!(
        query(&mut h, &invocation("query", &q), &q).unwrap()["status"],
        "succeeded"
    );
    q["resumeToken"]["appId"] = json!(ModelKind::Slot2.app_id());
    let count = h.calls.len();
    assert!(query(&mut h, &invocation("query", &q), &q).is_err());
    assert_eq!(h.calls.len(), count);
}
#[test]
fn incomplete_host_results_cannot_pass_using_permissive_native_mocks() {
    let p = payload(ModelKind::Ref1);
    for (method, field) in [
        ("host-http.execute", "headers"),
        ("host-media.info", "sha256"),
        ("host-checkpoint.read", "valueJson"),
        ("host-checkpoint.write", "revision"),
    ] {
        let mut h = Fixture::for_payload(&p);
        h.omit_port_field = Some((method, field));
        let value = match method {
            "host-http.execute" => http(
                &mut h,
                &invocation("query", &p),
                &p["configuration"],
                "GET",
                "https://rh-comfyui01.tos-cn-beijing.volces.com/original.mp4".into(),
                json!({"kind":"none"}),
                true,
            ),
            "host-media.info" => port(&mut h, method, json!({"handle":"image-handle-0"})),
            "host-checkpoint.read" => port(&mut h, method, json!({"requestId":"test-request"})),
            _ => port(
                &mut h,
                method,
                json!({"requestId":"test-request","expectedRevision":"0","valueJson":"{}"}),
            ),
        };
        assert_eq!(value.unwrap_err().code, "UPSTREAM_ERROR", "{method}");
    }
}
#[test]
fn missing_acceptance_is_not_faked_or_retried() {
    let p = payload(ModelKind::Ref1);
    let mut h = Fixture::for_payload(&p);
    h.submit_response = Some(json!({"status":"RUNNING"}));
    let e = submit(&mut h, &invocation("submit", &p), &p).unwrap_err();
    assert_eq!(e.code, "ACCEPTANCE_UNKNOWN");
    assert!(!h.checkpoint);
    assert_eq!(
        h.calls
            .iter()
            .filter(|v| v["args"]["plan"]["url"]
                .as_str()
                .is_some_and(|s| s.contains("/run/ai-app/")))
            .count(),
        1
    );
}

#[test]
fn schema_identity_normalizes_line_endings() {
    assert_eq!(schema("input", "{\n}\n"), schema("input", "{\r\n}\r\n"));
}
