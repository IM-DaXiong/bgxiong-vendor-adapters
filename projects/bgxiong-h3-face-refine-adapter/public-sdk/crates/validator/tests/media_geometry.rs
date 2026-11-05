use std::collections::BTreeMap;
use serde_json::{Value,json};
use bgx_adapter_v2_validator::{hash_bytes,validate_input,validate_input_with_media,media::*,identity::PackageIdentity};
use bgx_adapter_v2_sdk::{CapabilityContractV2,OperationSubmitV2};

fn setup(widget:&str,value:Value)->(CapabilityContractV2,OperationSubmitV2,MediaValidationContext){
    let fixture:Value=serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
    let mut c=fixture["capabilities"][0].clone();
    let schema=json!({"type":"object","properties":{"source":{"type":"object","properties":{"mediaId":{"type":"string"}},"required":["mediaId"],"additionalProperties":false},
        "selection":{"$ref":format!("https://bgxiong.local/vendor-adapter/v2/interaction-values.schema.json#/$defs/{widget}")}},"required":["source","selection"],"additionalProperties":false}).to_string();
    let digest=hash_bytes(schema.as_bytes());
    for resource in c["schemas"].as_array_mut().unwrap(){if resource["id"]=="input"{resource["schemaJson"]=json!(schema);resource["digest"]=json!(digest);}}
    c["operations"][0]["inputSchema"]["digest"]=json!(digest);
    c["operations"][0]["uiSchema"]["controls"]=json!([
        {"fieldPointer":"/source","widget":"media","label":"Source","description":"","group":""},
        {"fieldPointer":"/selection","widget":widget,"label":"Selection","description":"","group":""}]);
    c["operations"][0]["mediaSlots"]=json!([{"role":"source","inputPointer":"/source","mediaKind":"video","minCount":1,"maxCount":null}]);
    let request=json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"video.generate","operationId":"example/render","operationVersion":1,
        "inputSchemaDigest":digest,"input":{"source":{"mediaId":"source"},"selection":value},
        "media":[{"id":"source","kind":"video","handle":"h1","mime":"video/mp4","fileName":"source.mp4","byteLength":"12"},
                 {"id":"mask","kind":"image","handle":"h2","mime":"image/png","fileName":"mask.png","byteLength":"4"}],
        "uploads":[],"configuration":{"endpointBaseUrl":"https://example.invalid"}});
    let package=PackageIdentity{plugin_id:"local.test".into(),version:"1.0.0".into(),digest:"a".repeat(64)};
    let source=MediaFacts{digest:"a".repeat(64),kind:"video".into(),mime:"video/mp4".into(),byte_length:12,file_name:"source.mp4".into(),
        width:Some(100),height:Some(80),duration_ms:Some(1000),timestamps_ms:Some(vec![0,33,84,119,250,499,750,999])};
    let mask=MediaFacts{digest:"b".repeat(64),kind:"image".into(),mime:"image/png".into(),byte_length:4,file_name:"mask.png".into(),width:Some(100),height:Some(80),duration_ms:None,timestamps_ms:None};
    let lease=|media_id:&str,facts| MediaLease{package:package.clone(),binding_id:"b".into(),request_id:"r".into(),media_id:media_id.into(),live:true,facts};
    let host=MediaValidationContext{package:package.clone(),binding_id:"b".into(),request_id:"r".into(),
        leases_by_handle:BTreeMap::from([("h1".into(),lease("source",source)),("h2".into(),lease("mask",mask))]),uploads_by_ref:BTreeMap::new()};
    (serde_json::from_value(c).unwrap(),serde_json::from_value(request).unwrap(),host)
}

#[test]
fn exact_pixel_and_millisecond_bounds_and_vfr(){
    for (widget,good,bad) in [
        ("time_span",json!({"mediaId":"source","startMs":33,"endMs":1000}),json!({"mediaId":"source","startMs":33,"endMs":1001})),
        ("point",json!({"mediaId":"source","x":99.5,"y":79}),json!({"mediaId":"source","x":100,"y":79})),
        ("box",json!({"mediaId":"source","x":90,"y":70,"width":10,"height":10}),json!({"mediaId":"source","x":90,"y":70,"width":11,"height":10})),
        ("mask",json!({"mediaId":"source","maskMediaId":"mask"}),json!({"mediaId":"source","maskMediaId":"missing"}))]{
        let (c,r,mut host)=setup(widget,good);validate_input_with_media(&c,&r,&host).unwrap();
        host.leases_by_handle.get_mut("h1").unwrap().facts.timestamps_ms=Some(vec![0,40,80,120,300,600,999]);
        validate_input_with_media(&c,&r,&host).unwrap();
        let mut no_ui = serde_json::to_value(&c).unwrap();
        no_ui["operations"][0]["uiSchema"]["controls"] = json!([]);
        let no_ui = serde_json::from_value(no_ui).unwrap();
        validate_input_with_media(&no_ui,&r,&host).unwrap();
        let (_,bad_r,_)=setup(widget,bad);assert!(validate_input_with_media(&c,&bad_r,&host).is_err());
        assert!(validate_input_with_media(&no_ui,&bad_r,&host).is_err());
    }
    let(c,r,mut host)=setup("mask",json!({"mediaId":"source","maskMediaId":"mask"}));
    host.leases_by_handle.get_mut("h2").unwrap().facts.width=Some(101);
    assert!(validate_input_with_media(&c,&r,&host).is_err());
}

#[test]
fn duplicates_overflow_foreign_upload_and_lease_ownership_are_rejected(){
    let(c,r,mut host)=setup("point",json!({"mediaId":"source","x":2,"y":3}));
    validate_input_with_media(&c,&r,&host).unwrap();
    let mut value=serde_json::to_value(&r).unwrap();let duplicate=value["media"][0].clone();value["media"].as_array_mut().unwrap().push(duplicate);
    assert!(validate_input(&c,&serde_json::from_value(value).unwrap()).is_err());
    for name in ["../escape.png","a/b.png",".."]{let mut v=serde_json::to_value(&r).unwrap();v["media"][0]["fileName"]=json!(name);assert!(validate_input(&c,&serde_json::from_value(v).unwrap()).is_err());}
    let mut v=serde_json::to_value(&r).unwrap();v["media"][0]["byteLength"]=json!("18446744073709551616");assert!(validate_input(&c,&serde_json::from_value(v).unwrap()).is_err());
    host.leases_by_handle.get_mut("h1").unwrap().binding_id="someone-else".into();assert!(validate_input_with_media(&c,&r,&host).is_err());
    host.leases_by_handle.get_mut("h1").unwrap().binding_id="b".into();
    let mut v=serde_json::to_value(&r).unwrap();v["uploads"]=json!([{"mediaId":"source","uploadRef":"remote-ref"}]);let r=serde_json::from_value(v).unwrap();
    assert!(validate_input_with_media(&c,&r,&host).is_err());
    let config_digest=hash_bytes(&serde_json::to_vec(&json!({"endpointBaseUrl":"https://example.invalid"})).unwrap());
    host.uploads_by_ref.insert("remote-ref".into(),UploadProvenance{media_digest:"a".repeat(64),package:host.package.clone(),binding_id:"b".into(),configuration_digest:config_digest,media_id:"source".into()});
    validate_input_with_media(&c,&r,&host).unwrap();
    host.uploads_by_ref.get_mut("remote-ref").unwrap().media_digest="c".repeat(64);
    assert!(validate_input_with_media(&c,&r,&host).is_err());
    host.uploads_by_ref.get_mut("remote-ref").unwrap().media_digest="a".repeat(64);
    host.uploads_by_ref.get_mut("remote-ref").unwrap().binding_id="another".into();assert!(validate_input_with_media(&c,&r,&host).is_err());
}
