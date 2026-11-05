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
