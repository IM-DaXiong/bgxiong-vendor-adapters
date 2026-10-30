use std::{path::Path,fs,process::Command};
use serde_json::{Value,json};
use wasmtime::{Engine,Config,Store};
use wasmtime::component::{Component,Linker,Val};

pub const AUTHOR:&str=r#"
#![cfg_attr(target_arch="wasm32",no_std)]
extern crate alloc;
use bgx_adapter_v2_sdk::{*,host,guest};
use serde_json::{json,Value};
struct Echo;
impl Adapter for Echo{
    fn invoke(request:Invocation,host:&mut dyn Host)->Response{
        let _:()=host::call(host,"host-log.log",&json!({"level":"info","code":"external-author","message":"invoke"})).unwrap();
        let operation=serde_json::to_value(&request).unwrap()["operation"].clone();
        let payload:Value=serde_json::from_str(&request.r#payload_json).unwrap();
        let result=if operation=="capabilities"{
            let capabilities:Value=serde_json::from_str(include_str!("../capabilities.json")).unwrap();
            json!({"schemaVersion":2,"pluginId":"local.external","pluginVersion":"1.0.0","capabilityContracts":[capabilities]})
        }else{json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[],"data":{"description":payload["input"]["prompt"]},"usage":null})};
        guest::completed_response(&request,&result).unwrap()
    }
}
bgx_adapter_v2_sdk::export_adapter!(Echo);
#[cfg(test)]mod tests{
    use super::*;
    struct Fake;
    impl Host for Fake{fn call(&mut self,call:HostPortCallV2)->Result<Value,host::HostError>{
        assert_eq!(serde_json::to_value(call).unwrap()["method"],"host-log.log");Ok(Value::Null)}}
    #[test]fn same_author_native(){
        let invocation:Invocation=serde_json::from_str(include_str!("../request.json")).unwrap();
        let response=Echo::invoke(invocation,&mut Fake);let value=serde_json::to_value(response).unwrap();
        let data:Value=serde_json::from_str(value["dataJson"].as_str().unwrap()).unwrap();
        assert_eq!(data["data"]["description"],"example");
    }
}
"#;

fn copy_tree(from:&Path,to:&Path){
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap(){let entry=entry.unwrap();if entry.file_name()=="target"{continue;}
        if entry.file_type().unwrap().is_dir(){copy_tree(&entry.path(),&to.join(entry.file_name()));}else{fs::copy(entry.path(),to.join(entry.file_name())).unwrap();}}
}
fn section(source:&str,name:&str)->String{
    let marker=format!("[{name}]");let mut found=false;let mut result=String::new();
    for line in source.lines(){if line==marker{found=true;result.push_str(line);result.push('\n');continue;}
        if found&&line.starts_with('['){break;}if found{result.push_str(line);result.push('\n');}}
    assert!(found,"missing published workspace section");result
}
pub fn run(root:&Path,args:&[&str]){
    let output=Command::new("cargo").args(args).current_dir(root).output().unwrap();
    assert!(output.status.success(),"cargo {args:?}\n{}\n{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
}
fn invocation_value(operation:&str,payload:&Value,schemas:&Path)->Val{
    let schema:Value=serde_json::from_str(&fs::read_to_string(schemas.join("invocation.schema.json")).unwrap()).unwrap();
    let operation=schema["properties"]["operation"]["enum"].as_array().unwrap().iter().position(|v|v==operation).unwrap();
    let slot=schema["properties"]["capabilitySlot"]["enum"].as_array().unwrap().iter().position(|v|v=="generic").unwrap();
    Val::Record(vec![("protocol-version".into(),Val::U64(2)),("operation".into(),Val::Enum(format!("v{operation}"))),
        ("profile-id".into(),Val::String("external".into())),("capability-slot".into(),Val::Enum(format!("v{slot}"))),
        ("request-id".into(),Val::String("request".into())),("budget-ms".into(),Val::U64(1000)),
        ("payload-json".into(),Val::String(payload.to_string())),("binding-id".into(),Val::String("external".into())),
        ("negotiated-version".into(),Val::U64(2)),("enabled-features".into(),Val::List(vec![]))])
}
fn data(result:&Val)->Value{
    let Val::Variant(case,Some(payload))=result else{panic!("expected response variant")};assert_eq!(case,"v0");
    let Val::Record(fields)=payload.as_ref() else{panic!("expected response record")};
    let (_,Val::String(value))=fields.iter().find(|(name,_)|name=="data-json").unwrap() else{panic!("expected dataJson")};
    serde_json::from_str(value).unwrap()
}

pub fn prepare_workspace(sdk:&Path,author_source:&str)->(tempfile::TempDir,Value,Value){
    let temporary=tempfile::Builder::new().prefix("bgx-v2-author-").tempdir().unwrap();let root=temporary.path();
    copy_tree(&sdk.join("sdk/rust"),&root.join("sdk/rust"));copy_tree(&sdk.join("wit"),&root.join("wit"));
    copy_tree(&sdk.join("schemas"),&root.join("schemas"));copy_tree(&sdk.join("protocol"),&root.join("protocol"));
    let workspace=fs::read_to_string(sdk.join("Cargo.toml")).unwrap();
    fs::write(root.join("Cargo.toml"),format!("[workspace]\nresolver='2'\nmembers=['sdk/rust','guest']\n{}\n{}\n[profile.release]\npanic='abort'\n",section(&workspace,"workspace.package"),section(&workspace,"workspace.dependencies"))).unwrap();
    fs::copy(sdk.join("rust-toolchain.toml"),root.join("rust-toolchain.toml")).unwrap();
    fs::create_dir_all(root.join("guest/src")).unwrap();fs::write(root.join("guest/src/lib.rs"),author_source).unwrap();
    fs::write(root.join("guest/Cargo.toml"),"[package]\nname='bgx-sdk-external-guest'\nversion='1.0.0'\nedition='2021'\n[lib]\ncrate-type=['cdylib']\n[dependencies]\nbgx-adapter-v2-sdk={path='../sdk/rust'}\nserde_json={workspace=true}\n[profile.release]\npanic='abort'\n").unwrap();
    let cap_file:Value=serde_json::from_str(&fs::read_to_string(sdk.join("examples/capabilities.json")).unwrap()).unwrap();let mut cap=cap_file["capabilities"][0].clone();
    cap["capabilityKey"]=json!("text.generate");cap["transportSlot"]=json!("generic");cap["operations"][0]["id"]=json!("local.external/analyze");cap["operations"][0]["outputSlots"]=json!([]);
    fs::write(root.join("guest/capabilities.json"),cap.to_string()).unwrap();
    let mut payload:Value=serde_json::from_str::<Value>(&fs::read_to_string(sdk.join("examples/payloads.json")).unwrap()).unwrap()["validSubmit"].clone();
    payload["capabilityKey"]=json!("text.generate");payload["operationId"]=json!("local.external/analyze");
    let request=json!({"protocolVersion":2,"operation":"submit","profileId":"external","bindingId":"external","capabilitySlot":"generic","requestId":"request","budgetMs":1000,
        "negotiatedVersion":2,"enabledFeatures":[],"payloadJson":payload.to_string()});
    fs::write(root.join("guest/request.json"),request.to_string()).unwrap();
    run(root,&["generate-lockfile","--offline"]);
    (temporary,cap,payload)
}

#[test]
fn public_sdk_only_compiles_and_executes_the_same_native_and_wasm_author(){
    let sdk=Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (temporary,cap,payload)=prepare_workspace(&sdk,AUTHOR);let root=temporary.path();
    run(root,&["test","--locked","--offline","-p","bgx-sdk-external-guest","--lib"]);
    run(root,&["build","--locked","--offline","--release","--target","wasm32-wasip2","-p","bgx-sdk-external-guest"]);
    let bytes=fs::read(root.join("target/wasm32-wasip2/release/bgx_sdk_external_guest.wasm")).unwrap();
    let mut config=Config::new();config.wasm_component_model(true);let engine=Engine::new(&config).unwrap();let component=Component::new(&engine,&bytes).unwrap();
    let mut linker=Linker::<usize>::new(&engine);linker.define_unknown_imports_as_traps(&component).unwrap();linker.allow_shadowing(true);
    linker.root().instance("bgxiong:vendor-adapter/host-log@2.0.0").unwrap().func_new("log",|mut store,_signature,params,results|{
        assert_eq!(params.len(),1);*store.data_mut()+=1;results[0]=Val::Result(Ok(None));Ok(())}).unwrap();
    let mut store=Store::new(&engine,0usize);let instance=linker.instantiate(&mut store,&component).unwrap();let invoke=instance.get_func(&mut store,"invoke").unwrap();
    for operation in ["capabilities","submit"]{
        let input=if operation=="capabilities"{json!({})}else{payload.clone()};
        let mut result=[Val::Bool(false)];invoke.call(&mut store,&[invocation_value(operation,&input,&root.join("schemas"))],&mut result).unwrap();
        let output=data(&result[0]);if operation=="capabilities"{assert_eq!(output["capabilityContracts"][0],cap);}else{assert_eq!(output["data"]["description"],"example");}
    }
    assert_eq!(*store.data(),2);
}
