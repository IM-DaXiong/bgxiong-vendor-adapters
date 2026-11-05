//! Local protocol/mapping tests. No live ComfyUI.
use serde_json::json;

use crate::capability::{capabilities, CAPABILITY_KEY, FEATURE_PROJECTION, MODEL_ID, OPERATION_ID};
use crate::mapping::{build_graph, reference_id};

fn valid_input() -> serde_json::Value {
    json!({
        "prompt": "a bright living room with windows",
        "outputProjection": "equirectangular360",
        "references": [{"mediaId": "img-1"}],
        "width": 2048,
        "height": 1024,
        "seed": 7,
        "advanced": {"steps": 20}
    })
}

#[test]
fn capabilities_declare_projection_feature_contract() {
    let caps = capabilities();
    assert_eq!(caps["pluginId"], "local.thirdparty.comfy-panorama-image");
    let c = &caps["capabilityContracts"][0];
    assert_eq!(c["capabilityKey"], CAPABILITY_KEY);
    assert_eq!(c["modelIds"][0], MODEL_ID);
    assert_eq!(c["operations"][0]["id"], OPERATION_ID);
    assert_eq!(c["operations"][0]["mediaSlots"][0]["maxCount"], 1);
    assert_eq!(c["operations"][0]["mediaSlots"][0]["minCount"], 1);
    // Feature is on the package manifest; capability payload still uses image.generate.
    assert_eq!(FEATURE_PROJECTION, "image-projection-v1");
}

#[test]
fn mapping_accepts_single_reference_and_binds_upload() {
    let (graph, applied) = build_graph(&valid_input(), "uploaded.png").expect("ok");
    assert_eq!(graph["100"]["inputs"]["image"], "uploaded.png");
    assert_eq!(graph["31"]["inputs"]["seed"], 7);
    assert_eq!(graph["31"]["inputs"]["steps"], 20);
    assert_eq!(applied["outputProjection"], "equirectangular360");
    let state = graph["70"]["inputs"]["sticker_state"].as_str().unwrap();
    assert!(state.contains("\"hFOV_deg\":90"));
    assert!(state.contains("\"yaw_deg\":0"));
}

#[test]
fn mapping_rejects_wrong_cardinality_projection_and_size() {
    let mut zero = valid_input();
    zero["references"] = json!([]);
    assert!(build_graph(&zero, "x.png").is_err());

    let mut two = valid_input();
    two["references"] = json!([{"mediaId": "a"}, {"mediaId": "b"}]);
    assert!(build_graph(&two, "x.png").is_err());

    let mut flat = valid_input();
    flat["outputProjection"] = json!("flat");
    assert!(build_graph(&flat, "x.png").is_err());

    let mut size = valid_input();
    size["width"] = json!(1024);
    assert!(build_graph(&size, "x.png").is_err());
}

#[test]
fn reference_id_reads_single_slot() {
    assert_eq!(reference_id(&valid_input()).unwrap(), "img-1");
}

#[test]
fn manifest_feature_constant_matches_plan() {
    let raw = include_str!("../manifest.json");
    assert!(raw.contains("image-projection-v1"));
    assert!(raw.contains("local.thirdparty.comfy-panorama-image"));
    assert!(raw.contains("allowPrivateNetwork"));
}

#[test]
fn query_uses_public_query_dto_and_only_the_original_prompt() {
    use bgx_adapter_v2_sdk::{Host, host};
    struct QueueHost { complete: bool, calls: usize }
    impl Host for QueueHost {
        fn call(&mut self, c:bgx_adapter_v2_sdk::HostPortCallV2)->Result<serde_json::Value,host::HostError> {
            let c=serde_json::to_value(c).unwrap();
            self.calls += 1;
            if c["method"] == "host-media.info" {
                assert_eq!(c["args"]["handle"], "lease-result");
                return Ok(json!({"id":"result-id","kind":"image","mime":"image/jpeg","byteLength":"100","sha256":"a".repeat(64),"width":2048,"height":1024,"durationMs":null,"timestampsMs":null,"fileName":"original.jpg"}));
            }
            assert_eq!(c["method"],"host-http.execute");
            assert_eq!(c["args"]["plan"]["method"],"GET");
            let url=c["args"]["plan"]["url"].as_str().unwrap();
            if url.contains("/view?") {
                assert!(self.complete);
                assert_eq!(c["args"]["plan"]["sink"]["kind"], "media");
                return Ok(json!({"status":200,"headers":[],"payload":{"kind":"media","handle":"lease-result"}}));
            }
            let body=if url.ends_with("/queue") {json!({"queue_running":[[1,"original",{},{}]],"queue_pending":[]})}
                else {assert!(url.ends_with("/history/original")); if self.complete {json!({"original":{"status":{"status_str":"success"},"outputs":{"66":{"images":[{"filename":"original.jpg","subfolder":"","type":"output"}]}}}})} else {json!({})}};
            Ok(json!({"status":200,"headers":[],"payload":{"kind":"buffered","bytes":serde_json::to_vec(&body).unwrap()}}))
        }
    }
    let payload=json!({"modelId":MODEL_ID,"capabilityKey":CAPABILITY_KEY,"operationId":OPERATION_ID,"operationVersion":1,"vendorTaskId":"original","resumeToken":{"promptId":"original"},"configuration":{"endpointBaseUrl":"http://127.0.0.1:8188","credentialRef":null}});
    let invocation=serde_json::from_value(json!({"protocolVersion":2,"operation":"query","profileId":"binding","capabilitySlot":"image","requestId":"q","budgetMs":1000,"payloadJson":payload.to_string(),"bindingId":"binding","negotiatedVersion":2,"enabledFeatures":[]})).unwrap();
    let result=crate::query(&mut QueueHost {complete:false,calls:0},&invocation,&payload).unwrap();
    assert_eq!(result["status"],"running");
    serde_json::from_value::<bgx_adapter_v2_sdk::QueryResult>(result).unwrap();
    let mut h=QueueHost {complete:true,calls:0};
    let result=crate::query(&mut h,&invocation,&payload).unwrap();
    assert_eq!(h.calls,4);
    assert_eq!(result["status"],"succeeded");
    assert_eq!(result["data"]["imageArtifacts"][0]["mediaId"],"result-id");
    assert_eq!(result["outputs"][0]["media"]["mime"],"image/jpeg");
    serde_json::from_value::<bgx_adapter_v2_sdk::QueryResult>(result).unwrap();
}
