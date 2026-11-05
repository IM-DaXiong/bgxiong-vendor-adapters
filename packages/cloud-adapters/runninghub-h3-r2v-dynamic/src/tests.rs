use super::*;
use std::collections::BTreeMap;

fn input(n: usize, a: usize) -> Value {
    json!({
        "inputMode":"multi_image_to_video",
        "prompt":"保留原文\n<Picture 1> 与 <Audio 1>，不改台词。",
        "referenceImages":(0..n).map(|i|json!({"mediaId":format!("i{i}")})).collect::<Vec<_>>(),
        "referenceAudios":(0..a).map(|i|json!({"mediaId":format!("a{i}")})).collect::<Vec<_>>(),
        "aspectRatio":"16:9",
        "resolutionValue":{"kind":"megapixels","value":0.4},
        "durationSeconds":5,
        "fps":24,
        "seed":42,
        "advanced":{"audioUsage":"voice_timbre"}
    })
}
fn prepared(n: usize, a: usize) -> Value {
    let mut media = Vec::new();
    for (kind, count, prefix, mime, ext) in [
        ("image", n, "i", "image/jpeg", "jpg"),
        ("audio", a, "a", "audio/wav", "wav"),
    ] {
        for i in 0..count {
            media.push(json!({"id":format!("{prefix}{i}"),"kind":kind,"handle":format!("{prefix}{i}"),"fileName":format!("original-{prefix}{i}.{ext}"),"mime":mime,"byteLength":"17"}));
        }
    }
    json!({"schemaVersion":2,"modelId":MODEL,"capabilityKey":"video.generate","operationId":OP,"operationVersion":1,"inputSchemaDigest":capabilities()["capabilityContracts"][0]["operations"][0]["inputSchema"]["digest"],"input":input(n,a),"media":media,"uploads":[],"configuration":{"endpointBaseUrl":"https://www.runninghub.cn","credentialRef":"user-managed","instanceType":"plus"}})
}
fn invocation(op: &str, p: &Value) -> Invocation {
    serde_json::from_value(json!({"protocolVersion":2,"operation":op,"profileId":"test-binding","capabilitySlot":"video","requestId":"test-request","budgetMs":10000,"payloadJson":p.to_string(),"bindingId":"test-binding","negotiatedVersion":2,"enabledFeatures":[]})).unwrap()
}
fn response_json(v: Value) -> Value {
    json!({"status":200,"headers":[],"payload":{"kind":"buffered","bytes":serde_json::to_vec(&v).unwrap()}})
}
#[derive(Default)]
struct Fixture {
    stats: BTreeMap<String, Value>,
    calls: Vec<Value>,
    checkpoint: bool,
    submit_response: Option<Value>,
    query_response: Option<Value>,
    reject_download: bool,
    downloaded_stat: Option<Value>,
}
impl Fixture {
    fn for_payload(p: &Value) -> Self {
        let mut h = Self::default();
        for m in p["media"].as_array().unwrap() {
            let mut stat = m.clone();
            stat.as_object_mut().unwrap().remove("handle");
            stat["sha256"] = json!("0".repeat(64));
            stat["width"] = Value::Null;
            stat["height"] = Value::Null;
            stat["durationMs"] = Value::Null;
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
        Ok(match c["method"].as_str().unwrap() {
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
                } else if url.ends_with(&format!("/run/workflow/{WORKFLOW}")) {
                    response_json(self.submit_response.clone().unwrap_or_else(||json!({"taskId":"task-1","status":"RUNNING","errorCode":"","errorMessage":""})))
                } else if url.ends_with("/query") {
                    response_json(self.query_response.clone().unwrap_or_else(||json!({"taskId":"task-1","status":"RUNNING","errorCode":"","errorMessage":""})))
                } else {
                    assert_eq!(plan["method"], "GET");
                    if self.reject_download {
                        return Err(serde_json::from_value(json!({"code":"PERMISSION_DENIED","message":"Output origin is not authorized"})).unwrap());
                    }
                    assert!(args["credential"].is_null());
                    assert_eq!(plan["generatedHeaders"], json!([]));
                    let stat = self.downloaded_stat.clone().unwrap_or_else(|| serde_json::from_str(include_str!("../fixtures/sdk-managed-video-quicktime.json")).unwrap());
                    self.stats.insert("download".into(), stat);
                    json!({"status":200,"headers":[{"name":"Content-Type","value":"video/mp4"}],"payload":{"kind":"media","handle":"download"}})
                }
            }
            other => panic!("Unexpected fixture effect: {other}"),
        })
    }
}
fn query_payload() -> Value {
    let mut p = prepared(1, 0);
    for key in ["inputSchemaDigest", "input", "media", "uploads"] {
        p.as_object_mut().unwrap().remove(key);
    }
    p["vendorTaskId"] = json!("task-1");
    p["resumeToken"] = json!({"workflowId":WORKFLOW,"taskId":"task-1"});
    p
}
#[test]
fn increasing_decreasing_and_removing_audio_never_leaves_baked_assets() {
    let original: Value = serde_json::from_str(GRAPH).unwrap();
    for (n, a) in [(1, 0), (3, 2), (9, 3), (2, 1), (1, 0)] {
        let paths = (0..n)
            .map(|i| format!("openapi/ref{i}.jpg"))
            .collect::<Vec<_>>();
        let sounds = (0..a)
            .map(|i| format!("openapi/audio{i}.wav"))
            .collect::<Vec<_>>();
        let g = build_graph(&input(n, a), &paths, &sounds).unwrap();
        let c = g["136"]["inputs"].as_object().unwrap();
        assert_eq!(c.keys().filter(|s| s.starts_with("ref_images.")).count(), n);
        assert_eq!(c.keys().filter(|s| s.starts_with("ref_audios.")).count(), a);
        assert_eq!(
            g.as_object()
                .unwrap()
                .values()
                .filter(|x| x["class_type"] == "LoadImage")
                .count(),
            n
        );
        assert_eq!(
            g.as_object()
                .unwrap()
                .values()
                .filter(|x| x["class_type"] == "LoadAudio")
                .count(),
            a
        );
        assert!(g.get("137").is_none() && g.get("139").is_none());
        for id in [
            "119", "120", "121", "122", "123", "124", "125", "126", "127", "128", "130",
        ] {
            assert_eq!(g[id], original[id], "generation node {id}");
        }
        assert!(g.get("141").is_none() && g.get("142").is_none());
        assert_eq!(original["124"]["inputs"]["model"], json!(["127", 0]));
        assert_eq!(original["126"]["inputs"]["model"], json!(["127", 0]));
        assert_eq!(
            original["127"]["inputs"]["unet_name"],
            "minimax_h3_ref2va_pruned_int8_convrot.safetensors"
        );
        assert!(original.as_object().unwrap().values().all(|n| {
            !["SpectrumApplyMiniMaxH3", "MiniMaxH3TurboLoRA", "MiniMaxH3TurboSampler"]
                .contains(&n["class_type"].as_str().unwrap())
        }));
        assert_eq!(original["124"]["inputs"]["steps"], 20);
        assert_eq!(g["131"]["inputs"]["values.a"], original["131"]["inputs"]["values.a"]);
        assert_eq!(g["131"]["inputs"]["expression"], "max(5, round(a * 24))");
        assert_eq!(
            g["130"]["inputs"]["audio"],
            json!(["121", 0]),
            "input sound must not replace generated soundtrack"
        );
        assert!(g.get("92").is_none());
        assert_eq!(g["9000"], original["92"]);
    }
}
#[test]
fn ordered_originals_full_prompt_and_request_are_sent_once_before_checkpoint() {
    let mut p = prepared(3, 2);
    p["input"]["referenceImages"]
        .as_array_mut()
        .unwrap()
        .reverse();
    p["input"]["prompt"] = json!("完整中文\n\n".repeat(5000));
    let mut h = Fixture::for_payload(&p);
    let accepted = submit(&mut h, &invocation("submit", &p), &p).unwrap();
    assert!(h.checkpoint);
    assert_eq!(accepted["vendorTaskId"], "task-1");
    let submits: Vec<_> = h
        .calls
        .iter()
        .filter(|c| {
            c["args"]["plan"]["url"]
                == format!("https://www.runninghub.cn/openapi/v2/run/workflow/{WORKFLOW}")
        })
        .collect();
    assert_eq!(submits.len(), 1);
    let bytes: Vec<u8> =
        serde_json::from_value(submits[0]["args"]["plan"]["body"]["bytes"].clone()).unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["nodeInfoList"], json!([]));
    let graph: Value = serde_json::from_str(body["workflow"].as_str().unwrap()).unwrap();
    assert_eq!(graph["138"]["inputs"]["value"], p["input"]["prompt"]);
    assert_eq!(graph["1000"]["inputs"]["image"], "openapi/original-i2.jpg");
    assert_eq!(graph["2001"]["inputs"]["audio"], "openapi/original-a1.wav");
    assert_eq!(
        h.calls
            .iter()
            .filter(|c| c["args"]["plan"]["url"]
                == "https://www.runninghub.cn/openapi/v2/media/upload/binary")
            .count(),
        5
    );
}
#[test]
fn declared_scalar_fields_are_consumed_without_rewriting_size() {
    let mut i = input(2, 1);
    i["aspectRatio"] = json!("9:16");
    i["resolutionValue"]["value"] = json!(2.5);
    i["durationSeconds"] = json!(30);
    i["seed"] = json!(20261028);
    i["fps"] = json!(24);
    let paths = vec!["openapi/a.jpg".into(), "openapi/b.jpg".into()];
    let sounds = vec!["openapi/v.wav".into()];
    let g = build_graph(&i, &paths, &sounds).unwrap();
    assert_eq!(g["136"]["inputs"]["width"], json!(["115",0]));
    assert_eq!(g["136"]["inputs"]["height"], json!(["115",1]));
    assert_eq!(g["115"]["inputs"]["aspect_ratio"], "9:16 (Portrait Widescreen)");
    assert_eq!(g["115"]["inputs"]["megapixels"], 2.5);
    assert_eq!(g["132"]["inputs"]["value"], 30);
    assert_eq!(g["129"]["inputs"]["noise_seed"], 20261028);
    assert_eq!(g["130"]["inputs"]["fps"], 24);
    assert_eq!(g["138"]["inputs"]["value"], i["prompt"]);
    assert_eq!(g["1000"]["inputs"]["image"], "openapi/a.jpg");
    assert_eq!(g["1001"]["inputs"]["image"], "openapi/b.jpg");
    assert_eq!(g["2000"]["inputs"]["audio"], "openapi/v.wav");
    // All declared MP/aspect endpoints travel through the original selector.
    let schema: Value = serde_json::from_str(include_str!("../schemas/input.json")).unwrap();
    for ratio in schema["properties"]["aspectRatio"]["enum"].as_array().unwrap() {
        for mp in [0.1, 16.0] {
            for duration in [1, 15, 16, 29, 30] {
                i["aspectRatio"] = ratio.clone();
                i["resolutionValue"]["value"] = json!(mp);
                i["durationSeconds"] = json!(duration);
                let g = build_graph(&i, &paths, &sounds).unwrap();
                assert_eq!(g["115"]["inputs"]["aspect_ratio"], aspect_label(ratio.as_str().unwrap()).unwrap());
                assert_eq!(g["115"]["inputs"]["megapixels"], mp);
                assert_eq!(g["132"]["inputs"]["value"], duration);
                assert_eq!(g["131"]["inputs"]["expression"], "max(5, round(a * 24))");
                assert!(duration * 24 <= 3600);
            }
        }
    }
}
#[test]
fn invalid_contract_and_media_fail_before_any_effect() {
    let mut cases = vec![prepared(0, 0), prepared(10, 0), prepared(1, 4)];
    let mut wrong = prepared(1, 0);
    wrong["input"]["resolutionValue"]["value"] = json!(0.15);
    cases.push(wrong);
    let mut wrong = prepared(1, 0);
    wrong["input"]["resolutionValue"]["value"] = json!(16.1);
    cases.push(wrong);
    let mut wrong = prepared(1, 0);
    wrong["input"]["durationSeconds"] = json!(31);
    cases.push(wrong);
    let mut wrong = prepared(1, 0);
    wrong["input"]["seed"] = json!(9007199254740992u64);
    cases.push(wrong);
    let mut wrong = prepared(1, 0);
    wrong["input"]["fps"] = json!(30);
    cases.push(wrong);
    let mut wrong = prepared(1, 0);
    wrong["input"]["inputMode"] = json!("text_to_video");
    cases.push(wrong);
    let mut wrong = prepared(1, 0);
    wrong["media"][0]["kind"] = json!("audio");
    cases.push(wrong);
    let mut wrong = prepared(1, 0);
    wrong["input"]["negativePrompt"] = json!("");
    cases.push(wrong);
    let mut wrong = prepared(1, 0);
    wrong["input"]["aspectRatio"] = json!("9:5");
    cases.push(wrong);
    for p in cases {
        let mut h = Fixture::for_payload(&p);
        assert!(submit(&mut h, &invocation("submit", &p), &p).is_err());
        assert!(h.calls.is_empty());
    }
}
#[test]
fn capabilities_declare_input_mode_media_roles_and_no_fixed_aspect() {
    let caps = capabilities();
    let op = &caps["capabilityContracts"][0]["operations"][0];
    let schema: Value = serde_json::from_str(include_str!("../schemas/input.json")).unwrap();
    assert_eq!(schema["properties"]["inputMode"]["const"], "multi_image_to_video");
    assert_eq!(schema["properties"]["aspectRatio"]["enum"].as_array().unwrap().len(), 8);
    assert_eq!(schema["properties"]["durationSeconds"]["maximum"], 30);
    assert_eq!(schema["properties"]["resolutionValue"]["properties"]["value"]["maximum"], 16);
    assert!(schema["properties"].get("negativePrompt").is_none());
    assert_eq!(op["mediaSlots"][0]["role"], "reference_image");
    assert_eq!(op["mediaSlots"][1]["role"], "reference_audio");
    assert_eq!(op["mediaSlots"][0]["inputPointer"], "/referenceImages");
    let controls = op["uiSchema"]["controls"].as_array().unwrap();
    assert!(controls.iter().any(|c| c["fieldPointer"] == "/inputMode"));
    assert!(controls.iter().any(|c| c["fieldPointer"] == "/fps"));
    assert_eq!(
        caps["capabilityContracts"][0]["operations"][0]["inputSchema"]["digest"],
        op["inputSchema"]["digest"]
    );
}
#[test]
fn platform_rejection_is_explicit_and_does_not_mutate_input_size() {
    let mut p = prepared(1, 0);
    p["input"]["aspectRatio"] = json!("21:9");
    p["input"]["resolutionValue"]["value"] = json!(16.0);
    let original_w = p["input"]["aspectRatio"].clone();
    let original_h = p["input"]["resolutionValue"].clone();
    let mut h = Fixture::for_payload(&p);
    h.submit_response = Some(json!({
        "taskId":"task-reject",
        "status":"FAILED",
        "errorCode":"size",
        "errorMessage":"unsupported resolution combination"
    }));
    let err = submit(&mut h, &invocation("submit", &p), &p).unwrap_err();
    assert_eq!(err.code, "UPSTREAM_ERROR");
    assert!(err.message.contains("unsupported resolution combination"));
    assert!(err.message.contains("未修改用户画幅和MP"));
    assert!(!h.checkpoint);
    assert_eq!(p["input"]["aspectRatio"], original_w);
    assert_eq!(p["input"]["resolutionValue"], original_h);
}
#[test]
fn query_resumes_original_task_without_submit_and_downloads_without_credentials() {
    let p = query_payload();
    let mut h = Fixture::default();
    h.query_response = Some(
        json!({"taskId":"task-1","status":"SUCCESS","errorCode":"","errorMessage":"","usage":{"consumeCoins":1.25,"taskCostTime":"23"},"results":[{"nodeId":"9000","outputType":"mp4","url":"https://outputs.example.invalid/result.mp4"}]}),
    );
    let v = query(&mut h, &invocation("query", &p), &p).unwrap();
    assert_eq!(v["status"], "succeeded");
    assert_eq!(v["outputs"][0]["role"], "video");
    assert_eq!(v["outputs"][0]["media"]["mime"], "video/quicktime");
    assert_eq!(v["outputs"][0]["media"]["fileName"], "29d5c8c8459f1fc61702a9ae421ae1b2c437afca59d30cf4ff3c606f32a700fb.mov");
    let actual: Value = serde_json::from_str(include_str!("../fixtures/sdk-managed-video-quicktime.json")).unwrap();
    let typed: bgx_adapter_v2_sdk::HostPortCallV2MediaStat = serde_json::from_value(actual.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), actual);
    for (key, value) in [("kind",json!("file")),("mime",json!("application/octet-stream")),("byteLength",json!("0")),("width",json!(0)),("height",Value::Null),("durationMs",Value::Null),("durationMs",json!("0")),("sha256",json!("not-a-digest")),("fileName",json!("forged.mp4"))] {
        let mut invalid = actual.clone();
        invalid[key] = value;
        h.downloaded_stat = Some(invalid);
        assert!(query(&mut h,&invocation("query",&p),&p).unwrap_err().retryable);
    }
    let mut incomplete = actual.clone();
    incomplete.as_object_mut().unwrap().remove("sha256");
    h.downloaded_stat = Some(incomplete);
    assert_eq!(query(&mut h,&invocation("query",&p),&p).unwrap_err().code,"INVALID_INPUT");
    for source in [include_str!("../fixtures/sdk-managed-video-mp4.json"), include_str!("../fixtures/sdk-managed-video-ebml.json")] {
        let actual: Value = serde_json::from_str(source).unwrap();
        h.downloaded_stat = Some(actual.clone());
        h.query_response.as_mut().unwrap()["results"][0]["outputType"] = json!("vendor-video-tag");
        let result = query(&mut h,&invocation("query",&p),&p).unwrap();
        assert_eq!(result["outputs"][0]["media"]["mime"],actual["mime"]);
        assert_eq!(result["outputs"][0]["media"]["fileName"],actual["fileName"]);
    }
    h.downloaded_stat = None;
    assert_eq!(v["usage"]["measurements"][0]["amount"], "1.25");
    assert_eq!(v["usage"]["measurements"][0]["unit"], format!("{PLUGIN}/RH_coin"));
    assert_eq!(v["usage"]["measurements"][1]["unit"], format!("{PLUGIN}/vendor_reported_time_unit"));
    assert_eq!(v["resumeToken"], p["resumeToken"]);
    assert!(!h.calls.iter().any(|c| c["args"]["plan"]["url"]
        .as_str()
        .is_some_and(|s| s.contains("/run/workflow/"))));
    let bytes: Vec<u8> =
        serde_json::from_value(h.calls[0]["args"]["plan"]["body"]["bytes"].clone()).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"taskId":"task-1"})
    );
    h.reject_download = true;
    let failure = query(&mut h, &invocation("query", &p), &p).unwrap_err();
    assert_eq!(failure.code, "PERMISSION_DENIED");
    assert!(failure.retryable);
    assert_eq!(p["vendorTaskId"], "task-1");
    let response = serde_json::to_value(RunningHubH3::invoke(invocation("query", &p), &mut h)).unwrap();
    assert_eq!(response["error"]["retryable"], true);
    assert!(response.get("dataJson").is_none());
}
#[test]
fn unknown_responses_and_uncertain_acceptance_are_not_retried() {
    let p = prepared(1, 0);
    let mut h = Fixture::for_payload(&p);
    h.submit_response = Some(json!({"status":"RUNNING"}));
    assert_eq!(
        submit(&mut h, &invocation("submit", &p), &p)
            .unwrap_err()
            .code,
        "ACCEPTANCE_UNKNOWN"
    );
    assert!(!h.checkpoint);
    let p = query_payload();
    let mut h = Fixture::default();
    h.query_response = Some(json!({"taskId":"task-1","status":"NEW_UNKNOWN_STATE"}));
    assert!(query(&mut h, &invocation("query", &p), &p).is_err());
    h.query_response = Some(
        json!({"taskId":"task-1","status":"FAILED","errorCode":"1000","errorMessage":"fixture generation failure"}),
    );
    let failed = query(&mut h, &invocation("query", &p), &p).unwrap();
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["terminalFailure"]["vendorCode"], "1000");
    let e = unwrap_response(&response_json(
        json!({"errorCode":"803","errorMessage":"fixture rejection","taskId":"not-accepted"}),
    ))
    .unwrap_err();
    assert_eq!(e.vendor.as_deref(), Some("803"));
    h.query_response = Some(
        json!({"taskId":"task-1","status":"SUCCESS","errorCode":"","results":[{"nodeId":"92","outputType":"mp4","url":"https://outputs.example.invalid/old-graph.mp4"}]}),
    );
    let before = h.calls.len();
    let e = query(&mut h, &invocation("query", &p), &p).unwrap_err();
    assert_eq!(e.code, "UPSTREAM_ERROR");
    assert_eq!(
        h.calls.len(),
        before + 1,
        "old-graph result must not be downloaded"
    );
}

#[test]
fn schema_identity_normalizes_line_endings() {
    assert_eq!(schema("input", "{\n}\n"), schema("input", "{\r\n}\r\n"));
}
