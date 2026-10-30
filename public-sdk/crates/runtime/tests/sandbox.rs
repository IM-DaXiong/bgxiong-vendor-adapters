#[path = "../../../sdk/rust/tests/compile_guest.rs"]
mod author;

use bgx_adapter_v2_ports::{read_frame,write_frame,wire::{Host,HostPortCallV2,Invocation,WorkerEnvelopeV2}};
use bgx_adapter_v2_runtime::{Runtime,Cancellation};
use serde_json::{json,Value};
use sha2::{Sha256,Digest};
use std::{fs,path::Path,process::{Command,Stdio,Child},sync::{mpsc,Arc,atomic::{AtomicUsize,Ordering}},thread,time::{Duration,Instant}};
fn sha(bytes:&[u8])->String{hex::encode(Sha256::digest(bytes))}
fn packet(v:Value)->WorkerEnvelopeV2{serde_json::from_value(v).unwrap()}
fn invocation(mode:&str)->Invocation{serde_json::from_value(json!({"protocolVersion":2,"operation":"submit","profileId":"test","bindingId":"test","capabilitySlot":"generic","requestId":"request","budgetMs":500,"negotiatedVersion":2,"enabledFeatures":[],"payloadJson":json!({"mode":mode}).to_string()})).unwrap()}
fn limits()->Value{json!({"memoryBytes":33554432,"fuel":100000000,"wallMs":500,"outputBytes":1048576})}
struct Reap(Child);
impl Drop for Reap{fn drop(&mut self){let _=self.0.kill();let _=self.0.wait();}}
fn fixture(root:&Path,module:&[u8],limits:Value)->Value{
    fs::write(root.join("adapter.wasm"),module).unwrap();
    let manifest=json!({"schemaVersion":2,"pluginId":"local.sandbox","displayName":"Sandbox fixture","version":"1.0.0","protocolRange":{"min":2,"max":2},"requiredFeatures":[],"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":sha(module),"publisherId":null,"publisherKeyId":null,
        "requestedCapabilities":{"endpointPolicy":["staticHosts"],"networkHosts":[],"allowPrivateNetwork":false,"auth":[],"mediaSlots":[],"stateBytes":0,"embeddedHeaders":[]},"limits":limits});
    let manifest=serde_json::to_vec(&manifest).unwrap();fs::write(root.join("manifest.json"),&manifest).unwrap();
    let inventory=format!("{} adapter.wasm\n{} manifest.json\n",sha(module),sha(&manifest));fs::write(root.join("manifest.sha256"),&inventory).unwrap();
    json!({"pluginId":"local.sandbox","version":"1.0.0","digest":sha(inventory.as_bytes())})
}
fn worker(root:&Path,identity:Value,request:Invocation,cancel:bool)->Value{
    let mut command=Command::new(env!("CARGO_BIN_EXE_bgx-adapter-v2-worker"));
    command.arg("--package").arg(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]{use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
    let mut child=Reap(command.spawn().unwrap());let mut input=child.0.stdin.take().unwrap();let mut output=child.0.stdout.take().unwrap();
    let errors=child.0.stderr.take().unwrap();let diagnostic=thread::spawn(move||{use std::io::Read;let mut b=Vec::new();errors.take(8192).read_to_end(&mut b).unwrap();String::from_utf8_lossy(&b).into_owned()});
    let (tx,rx)=mpsc::sync_channel(1);let reader=thread::spawn(move||loop{let frame=read_frame(&mut output);let done=!matches!(&frame,Ok(Some(_)));if tx.send(frame).is_err()||done{break;}});
    let hello=packet(json!({"schemaVersion":2,"kind":"hello","protocolVersion":2,"witPackage":"bgxiong:vendor-adapter@2.0.0","policyDigest":sha(include_bytes!("../../../protocol/schema-policy.json"))}));
    write_frame(&mut input,&hello).unwrap();assert_eq!(serde_json::to_value(rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap().unwrap()).unwrap(),serde_json::to_value(hello).unwrap());
    write_frame(&mut input,&packet(json!({"schemaVersion":2,"kind":"invoke","requestId":"request","package":identity,"invocation":request}))).unwrap();
    let result=loop{
        match rx.recv_timeout(Duration::from_secs(30)){
            Ok(Ok(Some(frame)))=>{let value=serde_json::to_value(frame).unwrap();assert_eq!(value["requestId"],"request");
                if value["kind"]=="response"{break Ok(value["response"].clone());}
                assert_eq!(value["kind"],"port_call");assert_eq!(value["call"]["method"],"host-log.log","unexpected managed effect");
                if cancel{write_frame(&mut input,&packet(json!({"schemaVersion":2,"kind":"cancel","requestId":"request"}))).unwrap();}
                else{write_frame(&mut input,&packet(json!({"schemaVersion":2,"kind":"port_result","requestId":"request","callId":value["callId"],"result":{"ok":null}}))).unwrap();}
            },other=>break Err(format!("Worker ended without final typed response: {other:?}")),
        }
    };
    drop(input);let deadline=Instant::now()+Duration::from_secs(5);let status=loop{if let Some(status)=child.0.try_wait().unwrap(){break status;}if Instant::now()>=deadline{child.0.kill().unwrap();break child.0.wait().unwrap();}thread::sleep(Duration::from_millis(5));};
    if result.is_ok(){assert!(status.success(),"worker returned a response but did not exit successfully: {status}");assert!(matches!(rx.recv_timeout(Duration::from_secs(1)),Ok(Ok(None))),"duplicate final response or missing EOF");}
    drop(rx);reader.join().unwrap();let diagnostic=diagnostic.join().unwrap();
    result.unwrap_or_else(|reason|panic!("{reason}; process={status}; diagnostic={diagnostic}"))
}
struct SlowHost(Arc<AtomicUsize>);
impl Host for SlowHost{fn call(&mut self,call:HostPortCallV2)->Result<Value,bgx_adapter_v2_ports::wire::host::HostError>{assert_eq!(serde_json::to_value(call).unwrap()["method"],"host-log.log");self.0.fetch_add(1,Ordering::SeqCst);thread::sleep(Duration::from_millis(700));Ok(Value::Null)}}

#[test]
fn sandbox_real_guest_and_worker_resource_failures(){
    let sdk=Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source=r#"
#![no_std]
extern crate alloc;
use bgx_adapter_v2_sdk::{*,host,guest};
use serde_json::{json,Value};
struct Test;
impl Adapter for Test{fn invoke(request:Invocation,host:&mut dyn Host)->Response{
    let _:()=host::call(host,"host-log.log",&json!({"level":"info","code":"sandbox","message":"entered"})).unwrap();
    let payload:Value=serde_json::from_str(&request.r#payload_json).unwrap();
    match payload["mode"].as_str().unwrap(){
        "spin"=>loop{core::hint::spin_loop()},
        "trap"=>panic!("intentional fixture trap"),
        "memory"=>{let bytes=alloc::vec![1u8;134217728];core::hint::black_box(bytes);},
        _=>{}
    }
    let text=if payload["mode"]=="output"{alloc::string::String::from_utf8(alloc::vec![b'x';10000]).unwrap()}else{alloc::string::String::from("ok")};
    guest::completed_response(&request,&json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[],"data":{"description":text},"usage":null})).unwrap()
}}
bgx_adapter_v2_sdk::export_adapter!(Test);
"#;
    let (temp,_,_)=author::prepare_workspace(&sdk,source);author::run(temp.path(),&["build","--locked","--offline","--release","--target","wasm32-wasip2","-p","bgx-sdk-external-guest"]);
    let module=fs::read(temp.path().join("target/wasm32-wasip2/release/bgx_sdk_external_guest.wasm")).unwrap();
    let mut runtime=Runtime::new().unwrap();let calls=Arc::new(AtomicUsize::new(0));let start=Instant::now();
    let result=runtime.execute_bytes(&module,&sha(&module),invocation("normal"),Box::new(SlowHost(calls.clone())),serde_json::from_value(limits()).unwrap(),Cancellation::default()).unwrap();
    assert!(serde_json::to_value(result).unwrap().get("dataJson").is_some());assert_eq!(calls.load(Ordering::SeqCst),1);assert!(start.elapsed()>=Duration::from_millis(700));
    let fault=runtime.execute_bytes(&module,&"0".repeat(64),invocation("normal"),Box::new(SlowHost(calls.clone())),serde_json::from_value(limits()).unwrap(),Cancellation::default()).unwrap_err();assert_eq!(fault.code,"INVALID_PACKAGE");assert_eq!(calls.load(Ordering::SeqCst),1);
    let cancelled=Cancellation::default();cancelled.cancel();assert!(runtime.execute_bytes(&module,&sha(&module),invocation("normal"),Box::new(SlowHost(calls.clone())),serde_json::from_value(limits()).unwrap(),cancelled).is_err());
    for import in ["wasi:filesystem/types@0.2.0","wasi:sockets/network@0.2.0"]{
        let denied=wat::parse_str(format!("(component (type $io (instance)) (import \"{import}\" (instance (type $io))))")).unwrap();
        assert_eq!(runtime.execute_bytes(&denied,&sha(&denied),invocation("normal"),Box::new(SlowHost(calls.clone())),serde_json::from_value(limits()).unwrap(),Cancellation::default()).unwrap_err().code,"PERMISSION_DENIED");
    }
    let package=tempfile::tempdir().unwrap();let identity=fixture(package.path(),&module,limits());let normal=worker(package.path(),identity,invocation("normal"),false);assert!(normal.get("dataJson").is_some(),"{normal}");
    for (mode,expected) in [("trap","INVALID_PACKAGE"),("spin","TIMEOUT"),("memory","RESOURCE_UNAVAILABLE"),("output","RESOURCE_UNAVAILABLE")]{
        eprintln!("actual child case: {mode}");let mut budget=limits();if mode=="output"{budget["outputBytes"]=json!(256);}let identity=fixture(package.path(),&module,budget);
        let response=worker(package.path(),identity,invocation(mode),false);assert_eq!(response["error"]["code"],expected,"mode={mode}: {response}");
    }
    let identity=fixture(package.path(),&module,limits());let response=worker(package.path(),identity,invocation("spin"),true);assert_eq!(response["error"]["code"],"RESOURCE_UNAVAILABLE");
    let mut low=limits();low["fuel"]=json!(1000);let identity=fixture(package.path(),&module,low);assert_eq!(worker(package.path(),identity,invocation("normal"),false)["error"]["code"],"TIMEOUT");
    let identity=fixture(package.path(),&module,limits());fs::write(package.path().join("adapter.wasm"),b"tampered").unwrap();assert_eq!(worker(package.path(),identity,invocation("normal"),false)["error"]["code"],"INVALID_PACKAGE");
}
