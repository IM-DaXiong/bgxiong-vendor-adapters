use super::*;
fn input()->Value { serde_json::from_str(include_str!("../request-continuum.json")).unwrap() }
#[test]
fn run_name_contract_requires_explicit_closed_selection_without_backfill() {
    let schema:Value=serde_json::from_str(INPUT_SCHEMA).unwrap();
    let advanced=&schema["properties"]["advanced"];
    let run=&advanced["properties"]["runName"];
    assert_eq!(run["enum"],json!(["p13-independent-software-v101-20261023"]));
    assert_eq!(run["default"],input()["advanced"]["runName"]);
    assert!(advanced["required"].as_array().unwrap().contains(&json!("runName")));
    assert_eq!(advanced["additionalProperties"],false);
    for field in advanced["properties"].as_object().unwrap().values() {assert!(!field["default"].is_null());}
    let g=build_graph(&input(),&["original.png"],"full.wav").ok().unwrap();
    assert_eq!(g["15"]["inputs"]["run_name"],run["default"]);
    let original:Value=serde_json::from_str(GRAPH).unwrap();
    for node in ["16","17","18","19","20"] {assert_eq!(g[node],original[node]);}
    assert_eq!(g["15"]["inputs"]["run_storage"],"Save + Auto Resume");
    let mut absent=input();absent["advanced"].as_object_mut().unwrap().remove("runName");
    assert!(build_graph(&absent,&["original.png"],"full.wav").is_err());
    let mut undeclared=input();undeclared["advanced"]["runName"]=json!("another-name");
    assert!(build_graph(&undeclared,&["original.png"],"full.wav").is_err());
}
#[test]
fn image_and_driving_audio_use_the_native_condition_and_assembly_chain() {
    let g=build_graph(&input(), &["original.png"], "complete.wav").ok().unwrap();
    assert_eq!(g["3"]["inputs"]["image"],"original.png");
    assert_eq!(g["4"]["inputs"]["audio"],"complete.wav");
    assert_eq!(g["15"]["inputs"]["first_frame"],json!(["3",0]));
    assert_eq!(g["15"]["inputs"]["reference_image_1"],json!(["3",0]));
    assert_eq!(g["15"]["inputs"]["driving_audio"],json!(["4",0]));
    assert_eq!(g["15"]["inputs"]["audio_vae"],json!(["11",0]));
    assert_eq!(g["17"]["inputs"]["samples"],json!(["15",1]));
    assert_eq!(g["18"]["inputs"]["driving_audio"],json!(["15",4]));
    assert_eq!(g["18"]["inputs"]["audio"],json!(["17",0]));
    assert_eq!(g["19"]["inputs"]["audio"],json!(["18",1]));
    let original:Value=serde_json::from_str(GRAPH).unwrap();
    for n in ["8","9","10","11","12","13","16","17","18","20","21","22"] {assert_eq!(g[n],original[n]);}
}
#[test]
fn full_text_and_explicit_parameters_reach_the_graph_without_rewrite() {
    let mut i=input();i["prompt"]=json!("连续镜头\n\"原文\" \\ <Picture 1> 🐼");i["seed"]=json!(9007199254740991u64);
    i["advanced"]["runName"]=json!("p13-independent-software-v101-20261023");
    let g=build_graph(&i,&["one.png"],"full.wav").ok().unwrap();
    assert_eq!(g["5"]["inputs"]["value"],i["prompt"]);
    assert_eq!(g["15"]["inputs"]["base_seed"],i["seed"]);
    assert_eq!(g["15"]["inputs"]["run_name"],i["advanced"]["runName"]);
    assert_eq!(g["6"]["inputs"]["value"],512);assert_eq!(g["7"]["inputs"]["value"],512);
    assert_eq!(g["15"]["inputs"]["chunks"],1);assert_eq!(g["15"]["inputs"]["chunk_seconds"],5);
    assert_eq!(g["14"]["inputs"]["steps"],6);assert_eq!(g["19"]["inputs"]["fps"],24);
}
#[test]
fn unsupported_inputs_and_purposes_are_rejected_before_building_a_request() {
    for (key,value) in [("durationSeconds",json!(6)),("width",json!(480)),("fps",json!(25)),("prompt",json!("")),("hidden",json!(1)),("referenceImages",json!([])),("referenceImages",json!([{"mediaId":"a"},{"mediaId":"b"}])),("referenceAudios",json!([])),("seed",json!(9007199254740992u64))] {
        let mut i=input();i[key]=value;assert!(build_graph(&i,&["one"],"full").is_err(),"{key}");
    }
    for (key,value) in [("chunks",json!(2)),("chunkSeconds",json!(6)),("steps",json!(7)),("runName",json!("")),("audioUsage",json!("voice_timbre")),("hidden",json!(false))] {
        let mut i=input();i["advanced"][key]=value;assert!(build_graph(&i,&["one"],"full").is_err(),"{key}");
    }
    assert!(build_graph(&input(),&[],"full").is_err());assert!(build_graph(&input(),&["one"],"").is_err());
}
#[test]
fn public_media_declarations_and_audio_measurement_match_the_supported_profile() {
    let caps=capabilities();let op=&caps["capabilityContracts"][0]["operations"][0];
    assert_eq!(op["mediaSlots"],json!([{"role":"referenceImages","inputPointer":"/referenceImages","mediaKind":"image","minCount":1,"maxCount":1},{"role":"referenceAudios","inputPointer":"/referenceAudios","mediaKind":"audio","minCount":1,"maxCount":1}]));
    let schema:Value=serde_json::from_str(INPUT_SCHEMA).unwrap();assert_eq!(schema["properties"]["referenceImages"]["maxItems"],1);
    for key in ["width","height","fps","durationSeconds"] {assert_eq!(schema["properties"][key]["default"],input()[key]);}
    assert!(validate_audio_stat(&json!({"kind":"audio","mime":"audio/x-wav","durationMs":"5000"})).is_ok());
    for stat in [json!({"kind":"audio","mime":"audio/x-wav","durationMs":"4999"}),json!({"kind":"audio","mime":"audio/x-wav","durationMs":"5001"}),json!({"kind":"audio","mime":"audio/mpeg","durationMs":"5000"}),json!({"kind":"image","mime":"audio/x-wav","durationMs":"5000"})] {assert!(validate_audio_stat(&stat).is_err());}
}
