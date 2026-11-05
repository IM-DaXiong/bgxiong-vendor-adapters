use std::collections::BTreeMap;
use bgx_adapter_v2_sdk::{CapabilityContractV2, Invocation, OperationSubmitV2};
use bgx_adapter_v2_validator::{capabilities::*, package::*, hash_bytes};
use serde_json::{json, Value};

fn package() -> VerifiedPackage {
    let manifest=json!({"schemaVersion":2,"pluginId":"local.catalog","displayName":"Catalog","version":"1.0.0",
        "protocolRange":{"min":2,"max":2},"requiredFeatures":[],"optionalFeatures":[],"language":"rust",
        "entry":"adapter.wasm","artifactSha256":hash_bytes(b"test"),"publisherId":null,"publisherKeyId":null,
        "requestedCapabilities":{"endpointPolicy":["userConfigurable"],"networkHosts":[],"allowPrivateNetwork":false,
        "auth":["none"],"mediaSlots":[],"stateBytes":0,"embeddedHeaders":[]},
        "limits":{"memoryBytes":65536,"fuel":1000,"wallMs":100,"outputBytes":256}});
    let bytes=serde_json::to_vec(&manifest).unwrap();
    let files=BTreeMap::from([("manifest.json".into(),hash_bytes(&bytes)),("adapter.wasm".into(),hash_bytes(b"test"))]);
    let inventory=files.iter().map(|(k,v)|format!("{v} {k}\n")).collect::<String>();
    verify_package(&PackageContents{manifest_bytes:&bytes,hash_manifest_bytes:inventory.as_bytes(),signature:None,measured_file_hashes:&files},&TrustContext::default()).unwrap()
}
fn contracts() -> Vec<CapabilityContractV2> {
    let fixture:Value=serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
    ["video.generate","video.speech","video.edit","local.catalog/unseen"].iter().enumerate().map(|(index,key)|{
        let mut c=fixture["capabilities"][0].clone();
        c["localId"]=json!(format!("cap-{index}"));c["capabilityKey"]=json!(key);
        c["transportSlot"]=json!("generic");c["operations"][0]["id"]=json!(format!("local.catalog/op-{index}"));
        serde_json::from_value(c).unwrap()
    }).collect()
}

#[test]
fn multi_capability_generic_and_missing_media_are_discoverable() {
    let package=package();let configuration=json!({"endpointBaseUrl":"https://example.invalid"});
    let a=Binding::new("a".into(),&package,1,configuration.clone()).unwrap();
    let b=Binding::new("b".into(),&package,1,configuration).unwrap();
    let mut catalog=CapabilityRegistry::default();let contracts=contracts();
    let first=catalog.register(&package,&a,&contracts).unwrap();catalog.register(&package,&b,&contracts).unwrap();
    assert_eq!(catalog.list(None).len(),8);
    for key in ["video.generate","video.speech","video.edit","local.catalog/unseen"] {
        let views=catalog.list(Some(key));assert_eq!(views.len(),2);
        assert!(views.iter().all(|v|v.supported&&!v.environment_ready));
        assert_ne!(views[0].reference.capability_id,views[1].reference.capability_id);
    }
    assert!(catalog.register(&package,&a,&contracts).is_err());
    assert_eq!(catalog.list(None).len(),8);
    let good=json!({"packageValid":true,"connection":"ready","authentication":"ready","resources":"ready","issues":[]});
    catalog.set_readiness(&first[0],good).unwrap();
    assert_eq!(catalog.list(None).iter().filter(|v|v.environment_ready).count(),1);
    catalog.remove_binding("a");assert_eq!(catalog.list(None).len(),4);
}

#[test]
fn stale_bindings_and_changed_schema_are_rejected_at_submit() {
    let package=package();let binding=Binding::new("a".into(),&package,1,json!({"endpointBaseUrl":"https://example.invalid"})).unwrap();
    let mut catalog=CapabilityRegistry::default();let contracts=contracts();let refs=catalog.register(&package,&binding,&contracts).unwrap();
    let reference=refs.iter().find(|r|r.capability_key=="video.generate").unwrap();
    let mut payload:Value=serde_json::from_str::<Value>(include_str!("../../../examples/payloads.json")).unwrap()["validSubmit"].clone();
    payload["operationId"]=json!(reference.operation_id);
    let request:OperationSubmitV2=serde_json::from_value(payload.clone()).unwrap();
    let invocation:Invocation=serde_json::from_value(json!({"protocolVersion":2,"operation":"submit","profileId":"a","bindingId":"a",
        "capabilitySlot":"generic","requestId":"one","budgetMs":1000,"negotiatedVersion":2,"enabledFeatures":[],"payloadJson":payload.to_string()})).unwrap();
    catalog.validate_submission(reference,&binding,&invocation,&request).unwrap();
    let stale=Binding::new("a".into(),&package,2,binding.configuration().clone()).unwrap();
    assert!(catalog.validate_submission(reference,&stale,&invocation,&request).is_err());
    let mut changed=reference.clone();changed.input_schema_digest="0".repeat(64);
    assert!(catalog.validate_submission(&changed,&binding,&invocation,&request).is_err());
    let other=Binding::new("other".into(),&package,1,binding.configuration().clone()).unwrap();
    let mut conflicting=serde_json::to_value(&contracts[0]).unwrap();
    let new_schema=json!({"type":"object","properties":{},"required":[],"additionalProperties":false}).to_string();
    let digest=hash_bytes(new_schema.as_bytes());
    for resource in conflicting["schemas"].as_array_mut().unwrap(){if resource["id"]=="input"{resource["schemaJson"]=json!(new_schema);resource["digest"]=json!(digest);}}
    conflicting["operations"][0]["inputSchema"]["digest"]=json!(digest);
    conflicting["operations"][0]["uiSchema"]["controls"]=json!([]);
    let conflicting=serde_json::from_value(conflicting).unwrap();
    assert!(catalog.register(&package,&other,&[conflicting]).is_err());
    assert_eq!(catalog.list(None).len(),4);
}

#[test]
fn discovery_exposes_configuration_schema_before_configuration_is_complete() {
    let package=package();let binding=Binding::new("new".into(),&package,1,json!({})).unwrap();
    let mut catalog=CapabilityRegistry::default();let contracts=contracts();let refs=catalog.register(&package,&binding,&contracts).unwrap();
    assert_eq!(refs.len(),4);
    for view in catalog.list(None){assert!(view.supported);assert!(!view.environment_ready);assert_eq!(view.readiness["issues"][0]["code"],"INVALID_CONFIGURATION");}
    assert!(bgx_adapter_v2_validator::validate_configuration(catalog.contract_for(&refs[0],&binding).unwrap(),binding.configuration()).is_err());
    assert!(catalog.set_readiness(&refs[0],json!({"packageValid":true,"connection":"ready","authentication":"ready","resources":"ready","issues":[]})).is_err());
}

#[test]
fn refresh_is_atomic_and_preserves_operation_version_schema_identity() {
    let package=package();let binding=Binding::new("a".into(),&package,1,json!({"endpointBaseUrl":"https://example.invalid"})).unwrap();
    let mut catalog=CapabilityRegistry::default();let original=contracts();let refs=catalog.register(&package,&binding,&original).unwrap();
    let mut updated=serde_json::to_value(&original[0]).unwrap();updated["operations"][0]["label"]=json!("Updated label");
    let mut replacement=original.clone();replacement[0]=serde_json::from_value(updated).unwrap();
    let new_refs=catalog.refresh(&package,&binding,&replacement).unwrap();assert_eq!(catalog.list(None).len(),4);
    assert!(catalog.contract_for(refs.iter().find(|r|r.capability_key=="video.generate").unwrap(),&binding).is_err());
    let mut conflicting=serde_json::to_value(&replacement[0]).unwrap();let schema=json!({"type":"object","properties":{},"required":[],"additionalProperties":false}).to_string();let digest=hash_bytes(schema.as_bytes());
    for s in conflicting["schemas"].as_array_mut().unwrap(){if s["id"]=="input"{s["schemaJson"]=json!(schema);s["digest"]=json!(digest);}}
    conflicting["operations"][0]["inputSchema"]["digest"]=json!(digest);conflicting["operations"][0]["uiSchema"]["controls"]=json!([]);
    replacement[0]=serde_json::from_value(conflicting).unwrap();assert!(catalog.refresh(&package,&binding,&replacement).is_err());
    for reference in new_refs{assert!(catalog.contract_for(&reference,&binding).is_ok());}
}

#[test]
fn standard_speech_minimum_optional_fields_and_frozen_digest() {
    use bgx_adapter_v2_validator::{validate_contract, validate_input};
    let raw: Value = serde_json::from_str(include_str!("../../../profiles/speech-v1/fixtures/capability.json")).unwrap();
    let contract: CapabilityContractV2 = serde_json::from_value(raw.clone()).unwrap();
    validate_contract(&contract).unwrap(); // controls=[], no UI admission requirement
    let submit = |input: Value| serde_json::from_value(json!({
        "schemaVersion":2,"modelId":"test-model","capabilityKey":"audio.synthesize",
        "operationId":"std/speech.synthesize","operationVersion":1,
        "inputSchemaDigest":raw["operations"][0]["inputSchema"]["digest"],
        "input":input,"media":[],"uploads":[],"configuration":{
            "schemaVersion":1,"origin":"http://127.0.0.1:12345","instanceId":"test","credentialSlot":"test"
        }})).unwrap();
    for source in [include_str!("../../../profiles/speech-v1/fixtures/minimal.json"), include_str!("../../../profiles/speech-v1/fixtures/optional.json")] {
        validate_input(&contract, &submit(serde_json::from_str(source).unwrap())).unwrap();
    }
    assert!(validate_input(&contract, &submit(json!({"schemaVersion":1}))).is_err());
    assert!(validate_input(&contract, &submit(json!({"schemaVersion":1,"text":"ok","reference":{"audio":{"mediaId":"foreign"}}}))).is_err());
    let registry = bgx_adapter_v2_validator::SchemaRegistry::register(raw["schemas"].as_array().unwrap().iter().map(|s| (
        s["id"].as_str().unwrap().into(),s["schemaJson"].as_str().unwrap().into(),s["digest"].as_str().unwrap().into())).collect()).unwrap();
    let output = raw["operations"][0]["outputSchema"]["id"].as_str().unwrap();
    registry.validate(output, &Value::Null).unwrap();
    registry.validate(output, &json!({"schemaVersion":1,"attachments":[{"kind":"alignment","payload":"bad optional data"}]})).unwrap();
    // Host/runtime contracts use the same validator and local reference closure.
    let mut sources: Vec<_> = raw["schemas"].as_array().unwrap().iter().map(|s| (
        s["id"].as_str().unwrap().to_owned(), s["schemaJson"].as_str().unwrap().to_owned(), s["digest"].as_str().unwrap().to_owned())).collect();
    for source in [
        include_str!("../../../profiles/speech-v1/host.schema.json"),
        include_str!("../../../profiles/local-runtime-v1/runtime.schema.json"),
        include_str!("../../../profiles/local-runtime-v1/bundle.schema.json"),
        include_str!("../../../profiles/local-runtime-v1/control.schema.json"),
    ] {
        let value: Value = serde_json::from_str(source).unwrap();
        sources.push((value["$id"].as_str().unwrap().into(), source.into(), hash_bytes(source.as_bytes())));
    }
    let all = bgx_adapter_v2_validator::SchemaRegistry::register(sources).unwrap();
    all.validate_at("https://bgxiong.local/vendor-adapter/v2/profiles/speech-v1/host.schema.json", "/$defs/TaskRef", &json!({"taskId":"task-1"})).unwrap();
    all.validate_at("https://bgxiong.local/local-runtime/v1/control.schema.json", "/$defs/ReadyMessage", &json!({"schemaVersion":1,"type":"ready","instanceId":"runtime-1","nonce":"nonce-1"})).unwrap();
    let mut forged=raw.clone();
    forged["operations"][0]["inputSchema"]["digest"]=json!("0".repeat(64));
    assert!(validate_contract(&serde_json::from_value(forged).unwrap()).is_err());
    let mut wrong_version=raw;
    wrong_version["operations"][0]["version"]=json!(2);
    assert!(validate_contract(&serde_json::from_value(wrong_version).unwrap()).is_err());
}
