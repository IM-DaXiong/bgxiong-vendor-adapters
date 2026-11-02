use serde_json::{json,Value};
use bgx_adapter_v2_sdk::{CapabilityContractV2,OperationSubmitV2,OperationResultV2};
use bgx_adapter_v2_validator::{validate_input,validate_result,validate_task_input,result::PartialOutputLedger,usage::UsageSnapshot};

fn context(slots:Value)->bgx_adapter_v2_validator::ValidatedOperationInput{
    let fixture:Value=serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();let mut c=fixture["capabilities"][0].clone();
    c["operations"][0]["outputSlots"]=slots;
    c["operations"][0]["execution"]["mode"]=json!("asynchronous");
    let c:CapabilityContractV2=serde_json::from_value(c).unwrap();
    let p:Value=serde_json::from_str(include_str!("../../../examples/payloads.json")).unwrap();let p:OperationSubmitV2=serde_json::from_value(p["validSubmit"].clone()).unwrap();
    validate_input(&c,&p).unwrap()
}
fn completed(outputs:Value)->Value{json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":outputs,"data":{"description":"Structured result"},"usage":null})}

fn projection_context() -> bgx_adapter_v2_validator::ValidatedOperationInput {
    let fixture:Value=serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
    let mut c=fixture["capabilities"][0].clone();
    c["capabilityKey"]=json!("image.generate");
    c["operations"][0]["execution"]["mode"]=json!("asynchronous");
    c["operations"][0]["execution"]["streaming"]=json!(false);
    c["operations"][0]["outputSlots"]=json!([{"role":"result","kind":"image","minCount":1,"maxCount":null}]);
    let mut input_digest=String::new();
    for s in c["schemas"].as_array_mut().unwrap() {
        let mut schema:Value=serde_json::from_str(s["schemaJson"].as_str().unwrap()).unwrap();
        if s["id"]=="input" {
            schema["properties"]["outputProjection"]=json!({"type":"string","enum":["flat","equirectangular360"],"default":"flat"});
            schema["required"].as_array_mut().unwrap().push(json!("outputProjection"));
        } else if s["id"]=="output" {
            schema=json!({"$ref":"https://bgxiong.local/vendor-adapter/v2/profiles/image-projection-v1/output.schema.json"});
        } else {continue;}
        let raw=schema.to_string();let digest=bgx_adapter_v2_validator::hash_bytes(raw.as_bytes());
        if s["id"]=="input" {input_digest=digest.clone();}
        s["schemaJson"]=json!(raw);s["digest"]=json!(digest);
    }
    for name in ["input","output"] {
        c["operations"][0][format!("{name}Schema")]["digest"]=c["schemas"].as_array().unwrap().iter().find(|s|s["id"]==name).unwrap()["digest"].clone();
    }
    let mut request=serde_json::from_str::<Value>(include_str!("../../../examples/payloads.json")).unwrap()["validSubmit"].clone();
    request["capabilityKey"]=json!("image.generate");request["inputSchemaDigest"]=json!(input_digest);
    request["input"]["outputProjection"]=json!("equirectangular360");
    let context=validate_input(&serde_json::from_value(c).unwrap(),&serde_json::from_value(request).unwrap()).unwrap();
    bgx_adapter_v2_validator::image_projection::enable(context,&std::collections::BTreeSet::from(["image-projection-v1".into()])).unwrap()
}

#[test]
fn negotiated_projection_checks_identity_choice_geometry_and_terminal_boundaries() {
    let fixtures: Value=serde_json::from_str(include_str!("../../../profiles/image-projection-v1/fixtures/results.json")).unwrap();
    let registry=bgx_adapter_v2_validator::SchemaRegistry::public().unwrap();
    let schema="https://bgxiong.local/vendor-adapter/v2/profiles/image-projection-v1/output.schema.json";
    for value in fixtures["valid"].as_array().unwrap() {registry.validate(schema,value).unwrap();}
    for value in fixtures["invalid"].as_array().unwrap() {assert!(registry.validate(schema,value).is_err());}
    let c=projection_context();
    let mut good=completed(json!([{"kind":"media","role":"result","media":{"id":"a","kind":"image","handle":"lease-a","mime":"image/png","fileName":"same.png","byteLength":"72"}},
        {"kind":"media","role":"result","media":{"id":"b","kind":"image","handle":"lease-b","mime":"image/png","fileName":"same.png","byteLength":"72"}}]));
    good["data"]=json!({"imageArtifacts":[{"mediaId":"b","role":"result","projection":"equirectangular360"},{"mediaId":"a","role":"result","projection":"equirectangular360"}]});
    validate_result(&c,&serde_json::from_value(good.clone()).unwrap()).unwrap();
    for mutation in ["missing","duplicate","dangling","role","unknown","choice","null","no_images"] {
        let mut bad=good.clone();
        match mutation {
            "missing"=>{bad["data"]["imageArtifacts"].as_array_mut().unwrap().pop();},
            "duplicate"=>bad["data"]["imageArtifacts"][0]["mediaId"]=json!("a"),
            "dangling"=>bad["data"]["imageArtifacts"][0]["mediaId"]=json!("absent"),
            "role"=>bad["data"]["imageArtifacts"][0]["role"]=json!("other"),
            "unknown"=>bad["data"]["imageArtifacts"][0]["projection"]=json!("cube"),
            "choice"=>bad["data"]["imageArtifacts"][0]["projection"]=json!("flat"),
            "null"=>bad["data"]=Value::Null,
            "no_images"=>{bad["outputs"]=json!([]);bad["data"]=json!({"imageArtifacts":[]});},
            _=>unreachable!(),
        }
        assert!(validate_result(&c,&serde_json::from_value(bad).unwrap()).is_err(),"{mutation}");
    }
    let accepted=json!({"kind":"accepted","vendorTaskId":"task","appliedParams":null,"resumeToken":null,"usage":null});
    validate_result(&c,&serde_json::from_value(accepted).unwrap()).unwrap();
    let task=json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"image.generate","operationId":"example/render","operationVersion":1,"vendorTaskId":"task","resumeToken":null,"configuration":{"endpointBaseUrl":"https://example.invalid"}});
    let q=validate_task_input(&c,&serde_json::from_value(task).unwrap(),"query","task").unwrap();
    for status in ["queued","running","succeeded","failed","cancelled","expired"] {
        let terminal=match status {"failed"=>Some("vendorTaskFailed"),"cancelled"=>Some("vendorTaskCancelled"),"expired"=>Some("vendorTaskExpired"),_=>None};
        let mut query=json!({"status":status,"outputs":[],"data":{"imageArtifacts":[]},"usage":null,"progress":null,"resumeToken":null,"terminalFailure":terminal.map(|category|json!({"category":category,"message":"fixture","vendorCode":null,"details":null}))});
        if status=="succeeded" {query["outputs"]=good["outputs"].clone();query["data"]=good["data"].clone();}
        validate_result(&q,&serde_json::from_value(query.clone()).unwrap()).unwrap();
        if status!="succeeded" {query["outputs"]=good["outputs"].clone();query["data"]=good["data"].clone();assert!(validate_result(&q,&serde_json::from_value(query).unwrap()).is_err());}
    }
    use bgx_adapter_v2_validator::image_projection::validate_geometry;
    validate_geometry(&c,Some(2048),Some(1024)).unwrap();
    for size in [(Some(2047),Some(1024)),(None,None),(Some(0),Some(0)),(Some(2),Some(u32::MAX))] {assert!(validate_geometry(&c,size.0,size.1).is_err());}
}

#[test]
fn image_projection_data_uses_existing_local_schema_and_result_channel() {
    let fixture:Value=serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
    let mut contract=fixture["capabilities"][0].clone();
    contract["capabilityKey"]=json!("image.generate");
    contract["operations"][0]["outputSlots"]=json!([{"role":"result","kind":"image","minCount":1,"maxCount":null}]);
    let output=json!({"type":"object","properties":{"imageArtifacts":{"type":"array","items":{"$ref":"projection-item"}}},"required":["imageArtifacts"],"additionalProperties":false}).to_string();
    let item=json!({"type":"object","properties":{"mediaId":{"type":"string"},"role":{"type":"string"},"projection":{"type":"string","enum":["flat","equirectangular360"]}},"required":["mediaId","role","projection"],"additionalProperties":false}).to_string();
    for schema in contract["schemas"].as_array_mut().unwrap() {
        if schema["id"]=="output" {schema["schemaJson"]=json!(output);schema["digest"]=json!(bgx_adapter_v2_validator::hash_bytes(output.as_bytes()));}
    }
    contract["schemas"].as_array_mut().unwrap().push(json!({"id":"projection-item","digest":bgx_adapter_v2_validator::hash_bytes(item.as_bytes()),"schemaJson":item}));
    contract["operations"][0]["outputSchema"]["digest"]=json!(bgx_adapter_v2_validator::hash_bytes(output.as_bytes()));
    let mut payload:Value=serde_json::from_str::<Value>(include_str!("../../../examples/payloads.json")).unwrap()["validSubmit"].clone();
    payload["capabilityKey"]=json!("image.generate");
    let context=validate_input(&serde_json::from_value(contract).unwrap(),&serde_json::from_value(payload).unwrap()).unwrap();
    let mut result=completed(json!([{"kind":"media","role":"result","media":{"id":"image-1","kind":"image","handle":"leased-image","mime":"image/png","fileName":"result.png","byteLength":"72"}}]));
    for projection in ["flat","equirectangular360"] {
        result["data"]=json!({"imageArtifacts":[{"mediaId":"image-1","role":"result","projection":projection}]});
        validate_result(&context,&serde_json::from_value(result.clone()).unwrap()).unwrap();
    }
    result["data"]["imageArtifacts"][0]["projection"]=json!("unknown");
    assert!(validate_result(&context,&serde_json::from_value(result).unwrap()).is_err());
}

#[test]
fn structured_only_multioutput_and_wrong_branches(){
    let c=context(json!([]));let result:OperationResultV2=serde_json::from_value(completed(json!([]))).unwrap();validate_result(&c,&result).unwrap();
    let slots=json!([{"role":"answer","kind":"text","minCount":2,"maxCount":null}]);let c=context(slots);
    assert!(validate_result(&c,&result).is_err());
    let outputs=json!([{"kind":"text","role":"answer","text":"first"},{"kind":"text","role":"answer","text":"second"}]);
    let result=serde_json::from_value(completed(outputs)).unwrap();validate_result(&c,&result).unwrap();
    let wrong=serde_json::from_value(json!({"status":"running","outputs":[],"data":null,"usage":null,"progress":null,"terminalFailure":null,"resumeToken":null})).unwrap();
    assert!(validate_result(&c,&wrong).is_err());
    let task=serde_json::from_value(json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"video.generate","operationId":"example/render","operationVersion":1,
        "vendorTaskId":"accepted-task","resumeToken":null,"configuration":{"endpointBaseUrl":"https://example.invalid"}})).unwrap();
    let q=validate_task_input(&c,&task,"query","accepted-task").unwrap();
    for (status,category) in [("failed","vendorTaskFailed"),("cancelled","vendorTaskCancelled"),("expired","vendorTaskExpired")]{
        let value=json!({"status":status,"outputs":[],"data":null,"usage":null,"progress":null,"terminalFailure":{"category":category,"message":"Stopped","vendorCode":null,"details":null},"resumeToken":null});
        validate_result(&q,&serde_json::from_value(value.clone()).unwrap()).unwrap();
        let mut bad=value;bad["terminalFailure"]["category"]=json!("vendorTaskFailed");
        if status!="failed"{assert!(validate_result(&q,&serde_json::from_value(bad).unwrap()).is_err());}
    }
    assert!(validate_task_input(&c,&task,"query","different-task").is_err());
}

#[test]
fn applied_parameters_and_reported_usage_never_silently_change_input_or_sum_snapshots(){
    let c=context(json!([]));let mut value=completed(json!([]));value["appliedParams"]=json!({"strength":0.7});
    validate_result(&c,&serde_json::from_value(value.clone()).unwrap()).unwrap();
    assert_eq!(c.request()["input"]["strength"],0.5);
    value["appliedParams"]=json!({"missing":1});assert!(validate_result(&c,&serde_json::from_value(value).unwrap()).is_err());
    let usage=json!({"measurements":[{"metric":"output_tokens","unit":"tokens","amount":"123.0000000000000000001","source":"reported"}]});
    let mut snapshot=UsageSnapshot::default();snapshot.replace(&usage,"example").unwrap();snapshot.replace(&usage,"example").unwrap();assert_eq!(snapshot.current(),Some(&usage));
    for mutation in ["estimate","negative","namespace","duplicate"]{
        let mut bad=usage.clone();match mutation{"estimate"=>bad["measurements"][0]["source"]=json!("estimated"),"negative"=>bad["measurements"][0]["amount"]=json!("-1"),
            "namespace"=>bad["measurements"][0]["metric"]=json!("other/private"),"duplicate"=>{let item=bad["measurements"][0].clone();bad["measurements"].as_array_mut().unwrap().push(item);},_=>unreachable!()}
        assert!(snapshot.replace(&bad,"example").is_err());assert_eq!(snapshot.current(),Some(&usage));
    }
    let mut ledger=PartialOutputLedger::default();let outputs=vec![json!({"kind":"text","role":"answer","text":"first"}),json!({"kind":"text","role":"answer","text":"second"})];
    ledger.observe(&outputs[..1]).unwrap();ledger.observe(&outputs).unwrap();assert_eq!(ledger.outputs().len(),2);
    assert!(ledger.observe(&[json!({"kind":"text","role":"answer","text":"replacement"})]).is_err());assert_eq!(ledger.outputs(),outputs);
    let mut media=PartialOutputLedger::default();
    let output=json!({"kind":"media","role":"artifact","media":{"id":"stable-id","kind":"file","handle":"old-lease","mime":"application/octet-stream","fileName":"content.bin","byteLength":"3"}});
    media.observe(&[output.clone()]).unwrap();let mut renewed=output;renewed["media"]["handle"]=json!("new-lease");media.observe(&[renewed.clone()]).unwrap();assert_eq!(media.outputs(),vec![renewed.clone()]);
    renewed["media"]["fileName"]=json!("changed.bin");assert!(media.observe(&[renewed]).is_err());
}
