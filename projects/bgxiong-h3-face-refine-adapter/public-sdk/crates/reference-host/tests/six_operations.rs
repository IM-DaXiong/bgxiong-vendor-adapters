use bgx_adapter_v2_ports::wire::Invocation;
use bgx_adapter_v2_reference_host::{
    checkpoint::{CheckpointStore, SnapshotPolicy},
    config::*,
    credential::DevelopmentSecret,
    dispatch::{EventFuture, EventSink},
    media::*,
    policy::DevelopmentNetworkGrant,
    ReferenceHost,
};
use bgx_adapter_v2_transport::{Control, Options};
use bgx_adapter_v2_validator::{
    capabilities::{Binding, CapabilityRef},
    hash_bytes,
    package::TrustContext,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

const KEY: &str = "explicit-six-operation-test-key";
struct Fixture {
    _source: tempfile::TempDir,
    package_a: tempfile::TempDir,
    package_b: tempfile::TempDir,
    sdk: PathBuf,
}
fn build(directory: &Path, args: &[&str]) {
    let mut command = std::process::Command::new("cargo");
    command.current_dir(directory).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(||{
        let sdk=Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        build(&sdk,&["build","--locked","--offline","-p","bgx-adapter-v2-runtime","--bin","bgx-adapter-v2-worker"]);
        let source=tempfile::tempdir().unwrap();fs::create_dir(source.path().join("src")).unwrap();
        let sdk_crate=sdk.join("sdk/rust").to_str().unwrap().replace('\\',"/");let sdk_crate=sdk_crate.strip_prefix("//?/").unwrap_or(&sdk_crate);
        fs::write(source.path().join("Cargo.toml"),format!("[workspace]\n[package]\nname='six-operation-guest'\nversion='1.0.0'\nedition='2021'\n[lib]\ncrate-type=['cdylib']\n[dependencies]\nbgx-adapter-v2-sdk={{path='{sdk_crate}'}}\nserde_json={{version='=1.0.149',default-features=false,features=['alloc']}}\n[profile.release]\npanic='abort'\n")).unwrap();
        let examples:Value=serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
        let config=json!({"type":"object","properties":{"endpointBaseUrl":{"type":"string"},"credentialRef":{"type":"string"}},"required":["endpointBaseUrl","credentialRef"],"additionalProperties":false}).to_string();
        let input=json!({"type":"object","properties":{"prompt":{"type":"string"},"strength":{"type":"number","minimum":0,"maximum":1},"source":{"type":"object","properties":{"mediaId":{"type":"string"}},"required":["mediaId"],"additionalProperties":false}},"required":["prompt","strength"],"additionalProperties":false}).to_string();
        let mut contracts=vec![];
        for (index,name) in ["sync","async","classify"].iter().enumerate(){
            let mut c=examples["capabilities"][usize::from(index==1)].clone();c["localId"]=json!(name);c["transportSlot"]=json!(if index==2{"generic"}else{"video"});
            c["capabilityKey"]=json!(if index==2{"local.six/classify"}else{"video.generate"});c["operations"][0]["id"]=json!(format!("local.six/{name}"));
            c["operations"][0]["execution"]["streaming"]=json!(index==0);c["operations"][0]["execution"]["cancellation"]=json!("remote");
            c["operations"][0]["mediaSlots"]=json!([{"role":"source","inputPointer":"/source","mediaKind":"image","minCount":0,"maxCount":null}]);
            if index==0{c["operations"][0]["outputSlots"]=json!([{"role":"artifact","kind":"file","minCount":1,"maxCount":null}]);}
            for schema in c["schemas"].as_array_mut().unwrap(){if schema["id"]=="config"{schema["schemaJson"]=json!(config);schema["digest"]=json!(hash_bytes(config.as_bytes()));}if schema["id"]=="input"{schema["schemaJson"]=json!(input);schema["digest"]=json!(hash_bytes(input.as_bytes()));}}
            c["configurationSchema"]["digest"]=json!(hash_bytes(config.as_bytes()));c["operations"][0]["inputSchema"]["digest"]=json!(hash_bytes(input.as_bytes()));contracts.push(c);
        }
        fs::write(source.path().join("src/capabilities.json"),json!({"schemaVersion":2,"pluginId":"local.six","pluginVersion":"1.0.0","capabilityContracts":contracts}).to_string()).unwrap();
        let guest=r###"
#![no_std]
extern crate alloc;
use alloc::{format,string::{String,ToString},vec::Vec};
use bgx_adapter_v2_sdk::{*,host,guest};use serde_json::{json,Value};
struct Guest;
fn call<T:serde_json::value::Index>(_:T){} // replaced below; no private host API
fn http(host:&mut dyn Host,config:&Value,path:&str,body:Value)->Value{
 let modes:Vec<String>=host::call(host,"host-credential.capabilities",&json!({"handle":config["credentialRef"]})).unwrap();assert!(modes.iter().any(|m|m=="bearer"));
 let signature:String=host::call(host,"host-credential.sign",&json!({"handle":config["credentialRef"],"specJson":json!({"version":1,"kind":"hmac-sha256","message":"public test message","encoding":"hex"}).to_string()})).unwrap();assert_eq!(signature.len(),64);
 let plan=json!({"method":"POST","url":format!("{}{}",config["endpointBaseUrl"].as_str().unwrap(),path),"headers":[],"generatedHeaders":[{"kind":"bearer","name":"Authorization","targetField":"Authorization"}],"body":body,"sink":{"kind":"buffer","maxBytes":4096},"timeoutMs":5000});
 let response:Value=host::call(host,"host-http.execute",&json!({"plan":plan,"credential":config["credentialRef"]})).unwrap();
 assert_eq!(response["status"],200);let bytes:Vec<u8>=serde_json::from_value(response["payload"]["bytes"].clone()).unwrap();serde_json::from_slice(&bytes).unwrap()
}
impl Adapter for Guest{fn invoke(request:Invocation,host:&mut dyn Host)->Response{
 let i=serde_json::to_value(&request).unwrap();let op=i["operation"].as_str().unwrap();let p:Value=serde_json::from_str(i["payloadJson"].as_str().unwrap()).unwrap();
 let _:()=host::call(host,"host-log.log",&json!({"level":"info","code":"six.operation","message":op})).unwrap();
 let _:()=host::call(host,"host-state.put",&json!({"key":"last","value":op})).unwrap();let saved:Option<String>=host::call(host,"host-state.get",&json!({"key":"last"})).unwrap();assert_eq!(saved.as_deref(),Some(op));
 let result=match op{
  "capabilities"=>serde_json::from_str(include_str!("capabilities.json")).unwrap(),
  "probe"=>{
    if p["kind"]=="package_validation"{
      let denied:Result<Value,_>=host::call(host,"host-http.execute",&json!({"plan":{"method":"GET","url":"http://127.0.0.1/forbidden","headers":[],"generatedHeaders":[],"body":{"kind":"none"},"sink":{"kind":"buffer","maxBytes":64},"timeoutMs":100},"credential":null}));assert!(denied.is_err());
      json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]})
    }else{http(host,&p["configuration"],"/probe",json!({"kind":"none"}));
      let config=&p["configuration"];let opened:Value=host::call(host,"host-http-stream.open",&json!({"plan":{"method":"GET","url":format!("{}/stream",config["endpointBaseUrl"].as_str().unwrap()),"headers":[],"generatedHeaders":[{"kind":"bearer","name":"Authorization","targetField":"Authorization"}],"body":{"kind":"none"},"sink":{"kind":"buffer","maxBytes":4096},"timeoutMs":5000},"credential":config["credentialRef"]})).unwrap();
      let mut bytes=Vec::new();loop{let part:Value=host::call(host,"host-http-stream.read",&json!({"streamId":opened["streamId"],"maxBytes":1024})).unwrap();bytes.extend(serde_json::from_value::<Vec<u8>>(part["bytes"].clone()).unwrap());if part["eof"]==true{break;}}
      let _:()=host::call(host,"host-http-stream.close",&json!({"streamId":opened["streamId"]})).unwrap();assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["ok"],true);
      json!({"packageValid":true,"connection":"ready","authentication":"ready","resources":"ready","issues":[]})}
  },
  "upload"=>{let stat:Value=host::call(host,"host-media.info",&json!({"handle":p["media"]["handle"]})).unwrap();assert_eq!(stat["fileName"],p["media"]["fileName"]);let bytes:Vec<u8>=host::call(host,"host-media.read",&json!({"handle":p["media"]["handle"],"offset":"0","maxBytes":8})).unwrap();assert!(!bytes.is_empty());http(host,&p["configuration"],"/upload",json!({"kind":"media","handle":p["media"]["handle"]}));json!({"mediaId":p["media"]["id"],"uploadRef":"uploaded-source"})},
  "submit"=>{
   let remote=http(host,&p["configuration"],"/submit",json!({"kind":"bytes","bytes":p.to_string().into_bytes()}));
   let checkpoint:Value=host::call(host,"host-checkpoint.read",&json!({"requestId":i["requestId"]})).unwrap();
   if p["operationId"]=="local.six/async"{
     let result=json!({"kind":"accepted","vendorTaskId":remote["job"],"appliedParams":null,"resumeToken":null,"usage":null});
     let mut value:Value=serde_json::from_str(checkpoint["valueJson"].as_str().unwrap()).unwrap();let rev=value["revision"].as_str().unwrap().parse::<u64>().unwrap();value["revision"]=json!((rev+1).to_string());value["state"]=json!("accepted");value["vendorTaskId"]=remote["job"].clone();value["result"]=result.clone();
     let _:Value=host::call(host,"host-checkpoint.write",&json!({"requestId":i["requestId"],"expectedRevision":rev.to_string(),"valueJson":value.to_string()})).unwrap();result
   }else{
     let mut outputs=json!([]);
     if p["operationId"]=="local.six/sync"{
       let _:()=host::call(host,"host-stream.emit-event",&json!({"event":{"requestId":i["requestId"],"sequence":1,"type":"text_delta","payload":{"channel":"answer","text":"first real event"}}})).unwrap();
       let h:String=host::call(host,"host-media.create",&json!({"kind":"file","mimeHint":"application/octet-stream"})).unwrap();let _:()=host::call(host,"host-media.write",&json!({"handle":h,"bytes":[165,0,1]})).unwrap();let _:()=host::call(host,"host-media.finish",&json!({"handle":h})).unwrap();
       let stat:Value=host::call(host,"host-media.info",&json!({"handle":h})).unwrap();outputs=json!([{"kind":"media","role":"artifact","media":{"id":stat["id"],"handle":h,"kind":stat["kind"],"mime":stat["mime"],"fileName":stat["fileName"],"byteLength":stat["byteLength"]}}]);
       let partial:String=host::call(host,"host-media.create",&json!({"kind":"file","mimeHint":"application/octet-stream"})).unwrap();let _:()=host::call(host,"host-media.release",&json!({"handle":partial})).unwrap();
     }
     json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":outputs,"data":{"description":"ADAPTER_LABEL"},"usage":null})
   }
  },
  "query"=>{http(host,&p["configuration"],"/query",json!({"kind":"none"}));json!({"status":"succeeded","outputs":[],"data":{"description":"queried original task"},"usage":null,"progress":null,"terminalFailure":null,"resumeToken":null})},
  "cancel"=>{http(host,&p["configuration"],"/cancel",json!({"kind":"none"}));json!({"outcome":"remote_requested"})},
  _=>panic!("closed transport")
 };guest::completed_response(&request,&result).unwrap()
}}
bgx_adapter_v2_sdk::export_adapter!(Guest);
"###.replace("fn call<T:serde_json::value::Index>(_:T){} // replaced below; no private host API","");
        fs::write(source.path().join("src/lib.rs"),guest.replace("ADAPTER_LABEL","adapter-a")).unwrap();
        build(source.path(),&["generate-lockfile","--offline"]);
        let make=|label:&str|{fs::write(source.path().join("src/lib.rs"),guest.replace("ADAPTER_LABEL",label)).unwrap();build(source.path(),&["build","--release","--locked","--offline","--target","wasm32-wasip2"]);
            let root=tempfile::tempdir().unwrap();let artifact=fs::read(source.path().join("target/wasm32-wasip2/release/six_operation_guest.wasm")).unwrap();fs::write(root.path().join("adapter.wasm"),&artifact).unwrap();
            let manifest=json!({"schemaVersion":2,"pluginId":"local.six","displayName":label,"version":"1.0.0","protocolRange":{"min":2,"max":2},"requiredFeatures":["stream-events-v1"],"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":hash_bytes(&artifact),"publisherId":null,"publisherKeyId":null,"requestedCapabilities":{"endpointPolicy":["userConfigurable","allowPrivateNetwork"],"networkHosts":[],"allowPrivateNetwork":true,"auth":["bearer","hmac-sha256"],"mediaSlots":["image","file"],"stateBytes":1024,"embeddedHeaders":[]},"limits":{"memoryBytes":67108864,"fuel":1000000000,"wallMs":5000,"outputBytes":1048576}});let bytes=serde_json::to_vec(&manifest).unwrap();fs::write(root.path().join("manifest.json"),&bytes).unwrap();fs::write(root.path().join("manifest.sha256"),format!("{} adapter.wasm\n{} manifest.json\n",hash_bytes(&artifact),hash_bytes(&bytes))).unwrap();root};
        let package_a=make("adapter-a");let package_b=make("adapter-b");Fixture{_source:source,package_a,package_b,sdk}
    })
}
fn config(workspace: DevelopmentWorkspace, origin: &str, fixture: &Fixture) -> ReferenceConfig {
    ReferenceConfig {
        workspace,
        sdk_root: fixture.sdk.clone(),
        worker: fixture.sdk.join("target/debug/bgx-adapter-v2-worker.exe"),
        transport: Options {
            handshake_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(5),
            cancel_grace: Duration::from_millis(100),
        },
        network: DevelopmentNetworkGrant {
            origins: vec![origin.into()],
            allow_private: true,
            allow_embedded_authorization: false,
            denied_hosts: vec![],
            denied_networks: vec![],
        },
        credentials: vec![CredentialProvision {
            reference: "stable-credential".into(),
            secret: DevelopmentSecret {
                bytes: KEY.as_bytes().to_vec().into(),
                public_id: None,
                custom_header: None,
            },
            modes: BTreeSet::from(["bearer".into(), "hmac-sha256".into()]),
            origins: vec![origin.into()],
            ttl: Duration::from_secs(60),
        }],
        media_ttl: Duration::from_secs(60),
        probe: None,
        configuration_ref: "nonsecret-config".into(),
    }
}
fn make_host(workspace: DevelopmentWorkspace, origin: &str, path: &Path) -> ReferenceHost {
    let f = fixture();
    let mut trust = TrustContext::default();
    trust.supported_features.insert("stream-events-v1".into());
    let package = LoadedPackage::load(path, &trust).unwrap();
    let binding = Binding::new(
        "binding".into(),
        package.verified(),
        1,
        json!({"endpointBaseUrl":origin,"credentialRef":"stable-credential"}),
    )
    .unwrap();
    ReferenceHost::new(config(workspace, origin, f), package, binding).unwrap()
}
fn invocation(op: &str, id: &str, payload: Value, slot: &str) -> Invocation {
    serde_json::from_value(json!({"protocolVersion":2,"operation":op,"profileId":"binding","bindingId":"binding","capabilitySlot":slot,"requestId":id,"budgetMs":5000,"negotiatedVersion":2,"enabledFeatures":["stream-events-v1"],"payloadJson":payload.to_string()})).unwrap()
}
fn control() -> Control {
    Control::new(Some(Instant::now() + Duration::from_secs(30)))
}
fn data(response: &bgx_adapter_v2_ports::wire::Response) -> Value {
    let response = serde_json::to_value(response).unwrap();
    assert!(response.get("error").is_none(), "{response}");
    serde_json::from_str(response["dataJson"].as_str().unwrap()).unwrap()
}
fn submit(reference: &CapabilityRef, origin: &str) -> Value {
    json!({"schemaVersion":2,"modelId":reference.model_id,"capabilityKey":reference.capability_key,"operationId":reference.operation_id,"operationVersion":reference.operation_version,"inputSchemaDigest":reference.input_schema_digest,"input":{"prompt":"Real independent guest","strength":0.5},"media":[],"uploads":[],"configuration":{"endpointBaseUrl":origin,"credentialRef":"stable-credential"}})
}
fn task(reference: &CapabilityRef, origin: &str) -> Value {
    json!({"schemaVersion":2,"modelId":reference.model_id,"capabilityKey":reference.capability_key,"operationId":reference.operation_id,"operationVersion":reference.operation_version,"vendorTaskId":"real-job-one","resumeToken":null,"configuration":{"endpointBaseUrl":origin,"credentialRef":"stable-credential"}})
}
#[derive(Default)]
struct Sink(Mutex<Vec<Value>>);
impl EventSink for Sink {
    fn consume<'a>(
        &'a self,
        event: &'a bgx_adapter_v2_ports::wire::StreamEventV1,
    ) -> EventFuture<'a> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .push(serde_json::to_value(event).unwrap());
            Ok(())
        })
    }
}
async fn server() -> (
    String,
    Arc<Mutex<BTreeMap<String, usize>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(BTreeMap::new()));
    let observed = calls.clone();
    let run = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let calls = observed.clone();
            tokio::spawn(async move {
                let mut header = vec![];
                loop {
                    let byte = stream.read_u8().await.unwrap();
                    header.push(byte);
                    if header.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    assert!(header.len() < 32768);
                }
                let text = String::from_utf8(header).unwrap();
                let path = text
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                assert!(text
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {KEY}")));
                let length = text
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|s| s.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                assert!(length < 1048576);
                let mut body = vec![0; length];
                stream.read_exact(&mut body).await.unwrap();
                if path == "/upload" {
                    assert!(body.starts_with(b"\x89PNG"));
                }
                *calls.lock().unwrap().entry(path).or_default() += 1;
                let body = json!({"ok":true,"job":"real-job-one"}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    (origin, calls, run)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn six_operations_sync_async_generic_and_second_adapter_run_through_real_worker() {
    let f = fixture();
    let (origin, calls, server) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let host = make_host(workspace.clone(), &origin, f.package_a.path());
    host.invoke(
        invocation("capabilities", "caps", json!({}), "generic"),
        None,
        None,
        control(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(host.catalog().unwrap().len(), 3);
    host.invoke(
        invocation("capabilities", "caps-refresh", json!({}), "generic"),
        None,
        None,
        control(),
        None,
    )
    .await
    .unwrap();
    let refs = host.catalog().unwrap();
    let synchronous = refs
        .iter()
        .find(|v| v.reference.operation_id == "local.six/sync")
        .unwrap()
        .reference
        .clone();
    let asynchronous = refs
        .iter()
        .find(|v| v.reference.operation_id == "local.six/async")
        .unwrap()
        .reference
        .clone();
    let generic = refs
        .iter()
        .find(|v| v.reference.operation_id == "local.six/classify")
        .unwrap()
        .reference
        .clone();
    let package_probe = host
        .invoke(
            invocation(
                "probe",
                "package-probe",
                json!({"schemaVersion":2,"kind":"package_validation","configuration":{}}),
                "generic",
            ),
            None,
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(data(&package_probe.response)["connection"], "not_checked");
    assert!(calls.lock().unwrap().is_empty());
    host.invoke(invocation("probe","connect",json!({"schemaVersion":2,"kind":"connectivity","configuration":{"endpointBaseUrl":origin,"credentialRef":"stable-credential"}}),"video"),Some(asynchronous.clone()),None,control(),None).await.unwrap();
    let image = workspace.root().join("source.png");
    image::RgbImage::from_pixel(2, 3, image::Rgb([10u8, 20, 30]))
        .save(&image)
        .unwrap();
    let lease = host
        .import_media("upload", "source", &image, "image", control())
        .await
        .unwrap();
    let input_media = json!({"id":"source","kind":"image","handle":lease.handle,"mime":lease.stat.r#mime,"fileName":lease.file_name,"byteLength":lease.stat.r#byte_length});
    let upload=host.invoke(invocation("upload","upload",json!({"schemaVersion":2,"modelId":asynchronous.model_id,"capabilityKey":asynchronous.capability_key,"operationId":asynchronous.operation_id,"operationVersion":asynchronous.operation_version,"role":"source","media":input_media,"configuration":{"endpointBaseUrl":origin,"credentialRef":"stable-credential"}}),"video"),Some(asynchronous.clone()),None,control(),None).await.unwrap();
    assert_eq!(data(&upload.response)["uploadRef"], "uploaded-source");
    let sink = Arc::new(Sink::default());
    let sync = host
        .invoke(
            invocation("submit", "sync", submit(&synchronous, &origin), "video"),
            Some(synchronous.clone()),
            None,
            control(),
            Some(sink.clone()),
        )
        .await
        .unwrap();
    assert!(sync.worker_pid.unwrap() > 0);
    let output = data(&sync.response);
    assert_eq!(output["outputs"][0]["media"]["byteLength"], "3");
    assert!(workspace
        .root()
        .join("objects")
        .join(output["outputs"][0]["media"]["fileName"].as_str().unwrap())
        .is_file());
    assert_eq!(sink.0.lock().unwrap().len(), 1);
    let generic_result = host
        .invoke(
            invocation("submit", "generic", submit(&generic, &origin), "generic"),
            Some(generic.clone()),
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        data(&generic_result.response)["data"]["description"],
        "adapter-a"
    );
    let changed = workspace.root().join("changed-source.png");
    image::RgbImage::from_pixel(2, 3, image::Rgb([90u8, 80, 70]))
        .save(&changed)
        .unwrap();
    let bad_lease = host
        .import_media(
            "bad-upload-reference",
            "source",
            &changed,
            "image",
            control(),
        )
        .await
        .unwrap();
    let mut wrong = submit(&asynchronous, &origin);
    wrong["media"] = json!([{"id":"source","kind":"image","handle":bad_lease.handle,"mime":bad_lease.stat.r#mime,"fileName":bad_lease.file_name,"byteLength":bad_lease.stat.r#byte_length}]);
    wrong["input"]["source"] = json!({"mediaId":"source"});
    wrong["uploads"] = json!([{"mediaId":"source","uploadRef":"uploaded-source"}]);
    assert!(host
        .invoke(
            invocation("submit", "bad-upload-reference", wrong, "video"),
            Some(asynchronous.clone()),
            None,
            control(),
            None
        )
        .await
        .is_err());
    assert_eq!(calls.lock().unwrap().get("/submit"), Some(&2));
    let lease = host
        .import_media("async", "source", &image, "image", control())
        .await
        .unwrap();
    let mut async_input = submit(&asynchronous, &origin);
    async_input["media"] = json!([{"id":"source","kind":"image","handle":lease.handle,"mime":lease.stat.r#mime,"fileName":lease.file_name,"byteLength":lease.stat.r#byte_length}]);
    async_input["input"]["source"] = json!({"mediaId":"source"});
    async_input["uploads"] = json!([{"mediaId":"source","uploadRef":"uploaded-source"}]);
    let accepted = host
        .invoke(
            invocation("submit", "async", async_input, "video"),
            Some(asynchronous.clone()),
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(data(&accepted.response)["vendorTaskId"], "real-job-one");
    host.wait_idle().await.unwrap();
    drop(host);
    // Fresh host and credentials, no capability discovery, no old media handle.
    let resumed = make_host(workspace.clone(), &origin, f.package_a.path());
    let store = CheckpointStore::open(
        &workspace,
        SnapshotPolicy::new(vec![KEY.as_bytes().to_vec().into()]).unwrap(),
    )
    .unwrap();
    let sync_checkpoint = store.read("sync", "sync").unwrap().unwrap();
    drop(store);
    assert_eq!(
        sync_checkpoint["mediaIdentities"].as_array().unwrap().len(),
        1
    );
    let identity = serde_json::from_value(sync_checkpoint["mediaIdentities"][0].clone()).unwrap();
    let mut recovery_media =
        MediaBroker::new(workspace.clone(), Duration::from_secs(60), None).unwrap();
    let recovered = recovery_media
        .rehydrate(&resumed.scope("recovered-output"), &identity, &control())
        .unwrap();
    assert_ne!(
        sync_checkpoint["result"]["outputs"][0]["media"]["handle"],
        recovered.handle
    );
    assert_eq!(
        recovery_media
            .read(
                &resumed.scope("recovered-output"),
                &recovered.handle,
                0,
                10,
                &control()
            )
            .unwrap(),
        [165, 0, 1]
    );
    recovery_media.close().unwrap();
    let cancelled = resumed
        .invoke(
            invocation("cancel", "cancel", task(&asynchronous, &origin), "video"),
            None,
            Some("async".into()),
            control(),
            None,
        )
        .await
        .unwrap();
    assert!(cancelled.cancellation.unwrap().query_required());
    let queried = resumed
        .invoke(
            invocation("query", "query", task(&asynchronous, &origin), "video"),
            None,
            Some("async".into()),
            control(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(data(&queried.response)["status"], "succeeded");
    resumed.wait_idle().await.unwrap();
    let store = CheckpointStore::open(
        &workspace,
        SnapshotPolicy::new(vec![KEY.as_bytes().to_vec().into()]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store.read("async", "async").unwrap().unwrap()["state"],
        "terminal"
    );
    drop(store);
    let other_dir = tempfile::tempdir().unwrap();
    let other_ws = DevelopmentWorkspace::create(other_dir.path()).unwrap();
    let other = make_host(other_ws, &origin, f.package_b.path());
    other
        .invoke(
            invocation("capabilities", "b-caps", json!({}), "generic"),
            None,
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    let reference = other
        .catalog()
        .unwrap()
        .into_iter()
        .find(|v| v.reference.operation_id == "local.six/classify")
        .unwrap()
        .reference;
    let receipt = other
        .invoke(
            invocation("submit", "b-submit", submit(&reference, &origin), "generic"),
            Some(reference),
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(data(&receipt.response)["data"]["description"], "adapter-b");
    other.wait_idle().await.unwrap();
    assert_eq!(calls.lock().unwrap().get("/submit"), Some(&4));
    assert_eq!(calls.lock().unwrap().get("/query"), Some(&1));
    assert_eq!(calls.lock().unwrap().get("/cancel"), Some(&1));
    for entry in fs::read_dir(workspace.root().join("traces")).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        assert!(!bytes.windows(KEY.len()).any(|b| b == KEY.as_bytes()));
    }
    server.abort();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_input_and_missing_features_never_contact_remote_and_missing_consumer_fails() {
    let f = fixture();
    let (origin, calls, server) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let host = make_host(workspace.clone(), &origin, f.package_a.path());
    host.invoke(
        invocation("capabilities", "caps", json!({}), "generic"),
        None,
        None,
        control(),
        None,
    )
    .await
    .unwrap();
    let reference = host
        .catalog()
        .unwrap()
        .into_iter()
        .find(|v| v.reference.operation_id == "local.six/sync")
        .unwrap()
        .reference;
    let mut payload = submit(&reference, &origin);
    payload["input"]["strength"] = json!(2);
    assert!(host
        .invoke(
            invocation("submit", "bad-input", payload, "video"),
            Some(reference.clone()),
            None,
            control(),
            None
        )
        .await
        .is_err());
    let mut request = serde_json::to_value(invocation(
        "submit",
        "missing-feature",
        submit(&reference, &origin),
        "video",
    ))
    .unwrap();
    request["enabledFeatures"] = json!([]);
    assert!(host
        .invoke(
            serde_json::from_value(request).unwrap(),
            Some(reference.clone()),
            None,
            control(),
            None
        )
        .await
        .is_err());
    assert!(calls.lock().unwrap().is_empty());
    let result = timeout(
        Duration::from_secs(30),
        host.invoke(
            invocation(
                "submit",
                "no-consumer",
                submit(&reference, &origin),
                "video",
            ),
            Some(reference),
            None,
            control(),
            None,
        ),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    host.wait_idle().await.unwrap();
    assert_eq!(calls.lock().unwrap().get("/submit"), Some(&1));
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discovery_can_render_a_first_configuration_without_network_privileges() {
    let f = fixture();
    let (origin, calls, server) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut trust = TrustContext::default();
    trust.supported_features.insert("stream-events-v1".into());
    let package = LoadedPackage::load(f.package_a.path(), &trust).unwrap();
    let binding = Binding::new("binding".into(), package.verified(), 1, json!({})).unwrap();
    let host = ReferenceHost::new(config(workspace, &origin, f), package, binding).unwrap();
    let receipt = host
        .invoke(
            invocation("capabilities", "unconfigured", json!({}), "generic"),
            None,
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        data(&receipt.response)["capabilityContracts"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let views = host.catalog().unwrap();
    for v in &views {
        assert!(v.supported);
        assert!(!v.environment_ready);
        assert_eq!(v.readiness["issues"][0]["code"], "INVALID_CONFIGURATION");
    }
    assert!(host
        .invoke(
            invocation(
                "probe",
                "invalid-config-probe",
                json!({"schemaVersion":2,"kind":"connectivity","configuration":{}}),
                "generic"
            ),
            Some(views[0].reference.clone()),
            None,
            control(),
            None
        )
        .await
        .is_err());
    assert!(calls.lock().unwrap().is_empty());
    host.wait_idle().await.unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropped_import_uses_request_cleanup_and_wait_idle_without_affecting_another_request() {
    let f = fixture();
    let (origin, _, server) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let host = make_host(workspace.clone(), &origin, f.package_a.path());
    let path = workspace.root().join("large.bin");
    let mut file = fs::File::create(&path).unwrap();
    let block = vec![0xa5; 1048576];
    for _ in 0..64 {
        std::io::Write::write_all(&mut file, &block).unwrap();
    }
    drop(file);
    let result = timeout(
        Duration::from_millis(1),
        host.import_media("dropped-import", "large", &path, "file", Control::new(None)),
    )
    .await;
    assert!(result.is_err());
    timeout(Duration::from_secs(10), host.wait_idle())
        .await
        .unwrap()
        .unwrap();
    for session in fs::read_dir(workspace.root().join("staging")).unwrap() {
        assert_eq!(fs::read_dir(session.unwrap().path()).unwrap().count(), 0);
    }
    let small = workspace.root().join("small.bin");
    fs::write(&small, [0xa5, 0, 1]).unwrap();
    let lease = host
        .import_media("another-request", "small", &small, "file", control())
        .await
        .unwrap();
    assert_eq!(lease.stat.r#byte_length, "3");
    assert!(path.is_file());
    server.abort();
}
