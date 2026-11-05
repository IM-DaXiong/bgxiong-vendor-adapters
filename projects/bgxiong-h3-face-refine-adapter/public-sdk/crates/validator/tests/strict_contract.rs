use bgx_adapter_v2_sdk::{CapabilityContractV2, OperationSubmitV2};
use bgx_adapter_v2_validator::{SchemaRegistry, hash_bytes, validate_contract, validate_input};
use serde_json::{json, Value};

fn cap() -> CapabilityContractV2 {
    let v: Value = serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
    serde_json::from_value(v["capabilities"][0].clone()).unwrap()
}
fn payload() -> Value {
    serde_json::from_str::<Value>(include_str!("../../../examples/payloads.json")).unwrap()["validSubmit"].clone()
}
fn document(id: &str, schema: Value) -> (String, String, String) {
    let source = serde_json::to_string(&schema).unwrap();
    (id.into(), source.clone(), hash_bytes(source.as_bytes()))
}

#[test]
fn design_fixtures_and_strict_business_input() {
    let contract = cap();
    validate_contract(&contract).unwrap();
    let request: OperationSubmitV2 = serde_json::from_value(payload()).unwrap();
    validate_input(&contract, &request).unwrap();
    for mutation in ["unknown", "missing", "type", "identity", "digest"] {
        let mut p = payload();
        match mutation {
            "unknown" => p["input"]["extra"] = json!(true),
            "missing" => { p["input"].as_object_mut().unwrap().remove("prompt"); },
            "type" => p["input"]["prompt"] = json!(7),
            "identity" => p["modelId"] = json!("not-the-selected-model"),
            "digest" => p["inputSchemaDigest"] = json!("0".repeat(64)),
            _ => unreachable!(),
        }
        let request = serde_json::from_value(p).unwrap();
        assert!(validate_input(&contract, &request).is_err(), "mutation {mutation}");
    }
}

#[test]
fn native_engine_enforces_full_nested_constraints_without_defaults() {
    let schema = json!({"type":"object","properties":{
        "n":{"type":"integer","minimum":1,"maximum":5},
        "xs":{"type":"array","items":{"type":"integer"},"contains":{"const":3},"minContains":1,"maxContains":1},
        "s":{"type":"string","pattern":"^[A-Z]+$"}},
        "required":["n","xs","s"],"additionalProperties":false,
        "if":{"properties":{"n":{"const":1}}},"then":{"properties":{"s":{"const":"A"}}}});
    let r = SchemaRegistry::register(vec![document("test", schema)]).unwrap();
    assert!(r.validate("test", &json!({"n":1,"xs":[2,3],"s":"A"})).is_ok());
    for bad in [json!({"n":6,"xs":[3],"s":"A"}), json!({"n":1,"xs":[3,3],"s":"A"}),
                json!({"n":1,"xs":[3],"s":"B"}), json!({"n":2,"xs":[3],"s":"lower"}),
                json!({"xs":[3],"s":"A"})] { assert!(r.validate("test", &bad).is_err()); }
}

#[test]
fn registration_rejects_unknown_recursive_missing_and_corrupt_schemas() {
    for schema in [json!({"type":"integer","maximumm":3}),
        json!({"type":"object"}), json!({"$ref":"https://unregistered.invalid/schema"}),
        json!({"$ref":"#"}), json!({"type":"nonsense"}),
        json!({"$defs":{"a":{"$ref":"#/$defs/a"}},"$ref":"#/$defs/a"})] {
        assert!(SchemaRegistry::register(vec![document("test", schema)]).is_err());
    }
    let good = document("test", json!({"type":"integer"}));
    assert!(SchemaRegistry::register(vec![good.clone(), good]).is_err());
    assert!(SchemaRegistry::register(vec![("test".into(), "{}".into(), "0".repeat(64))]).is_err());
    let good = json!({"$defs":{"n":{"type":"integer","minimum":2}},"$ref":"#/$defs/n"});
    let r = SchemaRegistry::register(vec![document("test", good)]).unwrap();
    assert!(r.validate("test", &json!(1)).is_err());
    assert!(r.validate("test", &json!(2)).is_ok());
}

#[test]
fn scoped_ids_anchors_and_escaped_pointers_keep_native_schema_semantics() {
    let schema=json!({"$id":"https://example.invalid/schemas/root.json","$defs":{
        "nested":{"$id":"nested.json","type":"integer","minimum":2}},"$ref":"nested.json"});
    let registry=SchemaRegistry::register(vec![document("root",schema)]).unwrap();
    assert!(registry.validate("root",&json!(1)).is_err());assert!(registry.validate("root",&json!(2)).is_ok());
    let schema=json!({"$defs":{"n space":{"$anchor":"quantity","type":"integer","minimum":3}},"$ref":"#quantity"});
    let registry=SchemaRegistry::register(vec![document("anchor",schema)]).unwrap();assert!(registry.validate("anchor",&json!(2)).is_err());
    let schema=json!({"$defs":{"n space":{"type":"integer","minimum":4}},"$ref":"#/$defs/n%20space"});
    let registry=SchemaRegistry::register(vec![document("escaped",schema)]).unwrap();assert!(registry.validate("escaped",&json!(3)).is_err());
    let duplicate=json!({"$defs":{"a":{"$id":"same.json","type":"integer"},"b":{"$id":"same.json","type":"string"}}});
    assert!(SchemaRegistry::register(vec![document("duplicate",duplicate)]).is_err());
    let mut c=serde_json::to_value(cap()).unwrap();c["operations"][0]["uiSchema"]["controls"][0]["widget"]=json!("toggle");
    let c = serde_json::from_value(c).unwrap();
    validate_contract(&c).unwrap();
    assert!(!bgx_adapter_v2_validator::ui_diagnostics(&c).is_empty());
}

#[test]
fn missing_ui_controls_are_advisory_and_nullable_fields_remain_renderable() {
    let mut c=serde_json::to_value(cap()).unwrap();
    let schema=json!({"type":"object","properties":{"group":{"type":"object","properties":{
        "a":{"type":["string","null"]},"b":{"type":"boolean"}},"required":["a","b"],"additionalProperties":false}},
        "required":["group"],"additionalProperties":false}).to_string();
    let hash=hash_bytes(schema.as_bytes());
    for resource in c["schemas"].as_array_mut().unwrap(){if resource["id"]=="input"{resource["schemaJson"]=json!(schema);resource["digest"]=json!(hash);}}
    c["operations"][0]["inputSchema"]["digest"]=json!(hash);
    c["operations"][0]["uiSchema"]["controls"]=json!([{"fieldPointer":"/group/a","widget":"text","label":"A","description":"","group":""}]);
    let incomplete = serde_json::from_value(c.clone()).unwrap();
    validate_contract(&incomplete).unwrap();
    assert!(!bgx_adapter_v2_validator::ui_diagnostics(&incomplete).is_empty());
    c["operations"][0]["uiSchema"]["controls"].as_array_mut().unwrap().push(json!({"fieldPointer":"/group/b","widget":"toggle","label":"B","description":"","group":""}));
    validate_contract(&serde_json::from_value(c).unwrap()).unwrap();
}

#[test]
fn host_annotation_keywords_are_accepted() {
    let schema = json!({"type":"object",
        "x-resourceHint":{"blocking":false,"basis":"reference","vramPeakBytes":null,"reference":{"width":1280}},
        "properties":{"width":{"type":"integer","default":1280,"x-portraitDefault":720},"source":{"type":"object","x-mediaKind":"video",
            "properties":{"mediaId":{"type":"string"}},"required":["mediaId"],"additionalProperties":false}},
        "required":["source"],"additionalProperties":false});
    let r = SchemaRegistry::register(vec![document("test", schema)]).unwrap();
    assert!(r.validate("test", &json!({"source":{"mediaId":"a"}})).is_ok());
    assert!(r.validate("test", &json!({"source":{}})).is_err());
    let unknown = json!({"type":"object","x-unlisted":true});
    assert!(SchemaRegistry::register(vec![document("test", unknown)]).is_err());
}
