//! The CLI is exercised as separate OS processes against an actual WASM guest.
use bgx_adapter_v2_reference_host::{
    checkpoint::{CheckpointStore, SnapshotPolicy},
    media::DevelopmentWorkspace,
};
use bgx_adapter_v2_validator::{hash_bytes, SchemaRegistry};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};

struct Fixture {
    _source: tempfile::TempDir,
    package: tempfile::TempDir,
    sdk: PathBuf,
    contracts: Vec<Value>,
}
fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
fn build(path: &Path, args: &[&str]) {
    let output = command("cargo")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
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
        assert!(sdk.join(format!("target/debug/bgx-adapter-v2-worker{}",std::env::consts::EXE_SUFFIX)).is_file(),"Build the explicit SDK worker before the CLI integration suite");
        let source=tempfile::tempdir().unwrap();fs::create_dir(source.path().join("src")).unwrap();
        let sdk_path=sdk.join("sdk/rust").to_str().unwrap().replace('\\',"/");let sdk_path=sdk_path.strip_prefix("//?/").unwrap_or(&sdk_path);
        fs::write(source.path().join("Cargo.toml"),format!("[workspace]\n[package]\nname='cli-operation-guest'\nversion='1.0.0'\nedition='2021'\n[lib]\ncrate-type=['cdylib']\n[dependencies]\nbgx-adapter-v2-sdk={{path='{sdk_path}'}}\nserde_json={{version='=1.0.149',default-features=false,features=['alloc']}}\n[profile.release]\npanic='abort'\n")).unwrap();
        let examples:Value=serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
        let config=json!({"type":"object","properties":{},"required":[],"additionalProperties":false}).to_string();
        let input=json!({"type":"object","properties":{"prompt":{"type":"string"},"strength":{"type":"number","minimum":0,"maximum":1},"source":{"type":"object","properties":{"mediaId":{"type":"string"}},"required":["mediaId"],"additionalProperties":false}},"required":["prompt","strength"],"additionalProperties":false}).to_string();
        let mut contracts=Vec::new();
        for (index,name) in ["sync","async"].iter().enumerate() {
            let mut c=examples["capabilities"][index].clone();c["localId"]=json!(name);c["transportSlot"]=json!("generic");c["capabilityKey"]=json!("local.cli/run");c["operations"][0]["id"]=json!(format!("local.cli/{name}"));
            c["operations"][0]["execution"]["streaming"]=json!(index==0);c["operations"][0]["execution"]["cancellation"]=json!("remote");
            c["operations"][0]["mediaSlots"]=json!([{"role":"source","inputPointer":"/source","mediaKind":"file","minCount":0,"maxCount":null}]);
            for s in c["schemas"].as_array_mut().unwrap() { if s["id"]=="config" { s["schemaJson"]=json!(config);s["digest"]=json!(hash_bytes(config.as_bytes())); }if s["id"]=="input" {s["schemaJson"]=json!(input);s["digest"]=json!(hash_bytes(input.as_bytes()));} }
            c["configurationSchema"]["digest"]=json!(hash_bytes(config.as_bytes()));c["operations"][0]["inputSchema"]["digest"]=json!(hash_bytes(input.as_bytes()));contracts.push(c);
        }
        fs::write(source.path().join("src/capabilities.json"),json!({"schemaVersion":2,"pluginId":"local.cli","pluginVersion":"1.0.0","capabilityContracts":contracts}).to_string()).unwrap();
        fs::write(source.path().join("src/lib.rs"),r###"
#![no_std]
extern crate alloc;
use alloc::{string::ToString,vec::Vec};
use bgx_adapter_v2_sdk::{*,guest,host};use serde_json::{json,Value};
struct Guest;
impl Adapter for Guest { fn invoke(i:Invocation,h:&mut dyn Host)->Response {
 let v=serde_json::to_value(&i).unwrap();let p:Value=serde_json::from_str(v["payloadJson"].as_str().unwrap()).unwrap();
 let result=match v["operation"].as_str().unwrap() {
 "capabilities"=>serde_json::from_str(include_str!("capabilities.json")).unwrap(),
 "probe"=>json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]}),
 "upload"=>{let bytes:Vec<u8>=host::call(h,"host-media.read",&json!({"handle":p["media"]["handle"],"offset":"0","maxBytes":64})).unwrap();assert_eq!(bytes,b"SDK upload");json!({"mediaId":p["media"]["id"],"uploadRef":"cli-upload-proof"})},
 "submit"=>{
  if p["input"]["prompt"]=="fail" { return serde_json::from_value(json!({"protocolVersion":2,"operation":"submit","error":{"code":"UPSTREAM_ERROR","message":"Explicit fixture failure","retryable":false,"vendorCode":null,"vendorRequestId":null}})).unwrap(); }
  if p["operationId"]=="local.cli/async" {
   let saved:Value=host::call(h,"host-checkpoint.read",&json!({"requestId":v["requestId"]})).unwrap();let mut c:Value=serde_json::from_str(saved["valueJson"].as_str().unwrap()).unwrap();let revision=c["revision"].as_str().unwrap().parse::<u64>().unwrap();
   let accepted=json!({"kind":"accepted","vendorTaskId":"cli-job-one","appliedParams":null,"resumeToken":null,"usage":null});c["revision"]=json!((revision+1).to_string());c["state"]=json!("accepted");c["vendorTaskId"]=json!("cli-job-one");c["result"]=accepted.clone();
   let _:Value=host::call(h,"host-checkpoint.write",&json!({"requestId":v["requestId"],"expectedRevision":revision.to_string(),"valueJson":c.to_string()})).unwrap();accepted
  }else{
   let _:()=host::call(h,"host-stream.emit-event",&json!({"event":{"requestId":v["requestId"],"sequence":1,"type":"text_delta","payload":{"channel":"answer","text":"persist before ACK"}}})).unwrap();
   json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[],"data":{"description":"actual WASM result"},"usage":null})
  }
 },
 "query"=>{assert_eq!(p["vendorTaskId"],"cli-job-one");json!({"status":"succeeded","outputs":[],"data":{"description":"same durable task"},"usage":null,"progress":null,"terminalFailure":null,"resumeToken":null})},
 "cancel"=>json!({"outcome":"remote_requested"}),_=>panic!("closed transport")
 };guest::completed_response(&i,&result).unwrap()
}}
bgx_adapter_v2_sdk::export_adapter!(Guest);
"###).unwrap();
        build(source.path(),&["generate-lockfile","--offline"]);build(source.path(),&["build","--release","--locked","--offline","--target","wasm32-wasip2"]);
        let artifact=fs::read(source.path().join("target/wasm32-wasip2/release/cli_operation_guest.wasm")).unwrap();
        let package=tempfile::tempdir().unwrap();fs::write(package.path().join("adapter.wasm"),&artifact).unwrap();
        let manifest=json!({"schemaVersion":2,"pluginId":"local.cli","displayName":"CLI real guest","version":"1.0.0","protocolRange":{"min":2,"max":2},"requiredFeatures":["stream-events-v1"],"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":hash_bytes(&artifact),"publisherId":null,"publisherKeyId":null,"requestedCapabilities":{"endpointPolicy":["userConfigurable"],"networkHosts":[],"allowPrivateNetwork":false,"auth":[],"mediaSlots":["file"],"stateBytes":1024,"embeddedHeaders":[]},"limits":{"memoryBytes":67108864,"fuel":1000000000,"wallMs":5000,"outputBytes":1048576}});
        let bytes=serde_json::to_vec(&manifest).unwrap();fs::write(package.path().join("manifest.json"),&bytes).unwrap();fs::write(package.path().join("manifest.sha256"),format!("{} adapter.wasm\n{} manifest.json\n",hash_bytes(&artifact),hash_bytes(&bytes))).unwrap();
        Fixture{_source:source,package,sdk,contracts}
    })
}
fn config() -> Value {
    json!({"bindingId":"binding","bindingRevision":1,"configurationRef":"explicit-cli-config","configuration":{},"trust":{"publishers":[],"revokedPlugins":[],"revokedVersions":[],"revokedKeys":[],"denylistedHosts":[],"isFresh":false,"supportedFeatures":["stream-events-v1"]},"network":{"origins":[],"allowPrivate":false,"allowEmbeddedAuthorization":false,"deniedHosts":[],"deniedNetworks":[]},"credentials":[],"media":[],"mediaTtlMs":60000,"transport":{"handshakeTimeoutMs":5000,"writeTimeoutMs":5000,"shutdownTimeoutMs":5000,"cancelGraceMs":100},"probe":null})
}
fn invocation(op: &str, id: &str, payload: Value) -> Value {
    json!({"protocolVersion":2,"operation":op,"profileId":"binding","bindingId":"binding","capabilitySlot":"generic","requestId":id,"budgetMs":15000,"negotiatedVersion":2,"enabledFeatures":["stream-events-v1"],"payloadJson":payload.to_string()})
}
fn submit(index: usize) -> Value {
    let c = &fixture().contracts[index];
    json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":c["capabilityKey"],"operationId":c["operations"][0]["id"],"operationVersion":1,"inputSchemaDigest":c["operations"][0]["inputSchema"]["digest"],"input":{"prompt":"actual CLI execution","strength":0.5},"media":[],"uploads":[],"configuration":{}})
}
fn task() -> Value {
    json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"local.cli/run","operationId":"local.cli/async","operationVersion":1,"vendorTaskId":"cli-job-one","resumeToken":null,"configuration":{}})
}
fn run(
    workspace: &Path,
    configuration: Value,
    input: Value,
    root: Option<&str>,
    create: bool,
) -> (Output, PathBuf) {
    let f = fixture();
    let id = input["requestId"].as_str().unwrap();
    // Config/input are intentionally explicit and kept outside the workspace that
    // must still be empty at first creation.
    let files = tempfile::tempdir().unwrap();
    let config = files.path().join("config.json");
    let request = files.path().join("request.json");
    fs::write(&config, configuration.to_string()).unwrap();
    fs::write(&request, input.to_string()).unwrap();
    let output = workspace
        .join("receipts")
        .join(format!("{}.json", hash_bytes(id.as_bytes())));
    let mut c = command(env!("CARGO_BIN_EXE_bgx-adapter-v2"));
    c.arg(input["operation"].as_str().unwrap())
        .arg("--sdk")
        .arg(&f.sdk)
        .arg("--worker")
        .arg(f.sdk.join(format!(
            "target/debug/bgx-adapter-v2-worker{}",
            std::env::consts::EXE_SUFFIX
        )))
        .arg("--package")
        .arg(f.package.path())
        .arg("--workspace")
        .arg(workspace)
        .arg("--config")
        .arg(config)
        .arg("--input")
        .arg(request)
        .arg("--output")
        .arg(&output);
    if create {
        c.arg("--create-workspace");
    }
    if let Some(root) = root {
        c.arg("--request-id").arg(root);
    }
    // Ambient application settings and secret-like variables are irrelevant.
    c.env("BGX_API_KEY", "ambient-secret-must-never-be-discovered");
    (c.output().unwrap(), output)
}
fn data(output: &Output, path: &Path) -> Value {
    assert!(
        output.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(printed, saved);
    SchemaRegistry::validate_public("response", &saved).unwrap();
    serde_json::from_str(saved["dataJson"].as_str().unwrap()).unwrap()
}

#[test]
fn six_operations_and_fresh_process_query_use_standard_wire_and_durable_workspace() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("development");
    let (out, path) = run(
        &workspace,
        config(),
        invocation("capabilities", "caps", json!({})),
        None,
        true,
    );
    let caps = data(&out, &path);
    assert_eq!(caps["pluginId"], "local.cli");
    let (out, path) = run(
        &workspace,
        config(),
        invocation(
            "probe",
            "probe",
            json!({"schemaVersion":2,"kind":"package_validation","configuration":{}}),
        ),
        None,
        false,
    );
    assert_eq!(data(&out, &path)["packageValid"], true);
    let source = workspace.join("source.bin");
    fs::write(&source, b"SDK upload").unwrap();
    let mut upload_config = config();
    upload_config["media"] = json!([{"id":"source","path":source,"kind":"file"}]);
    let media = json!({"id":"source","kind":"file","handle":"explicit-development-placeholder","mime":"application/octet-stream","fileName":format!("{}.bin",hash_bytes(b"SDK upload")),"byteLength":"10"});
    let upload = json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"local.cli/run","operationId":"local.cli/async","operationVersion":1,"role":"source","media":media,"configuration":{}});
    let (out, path) = run(
        &workspace,
        upload_config,
        invocation("upload", "upload", upload),
        None,
        false,
    );
    assert_eq!(data(&out, &path)["uploadRef"], "cli-upload-proof");
    let directory = workspace.join("cli").join(hash_bytes(b"upload"));
    let original: Value =
        serde_json::from_slice(&fs::read(directory.join("input.invocation.json")).unwrap())
            .unwrap();
    let executed: Value =
        serde_json::from_slice(&fs::read(directory.join("execution.invocation.json")).unwrap())
            .unwrap();
    let a: Value = serde_json::from_str(original["payloadJson"].as_str().unwrap()).unwrap();
    let b: Value = serde_json::from_str(executed["payloadJson"].as_str().unwrap()).unwrap();
    assert_ne!(a["media"]["handle"], b["media"]["handle"]);
    let mut expected = a;
    expected["media"]["handle"] = b["media"]["handle"].clone();
    assert_eq!(expected, b);
    let (out, path) = run(
        &workspace,
        config(),
        invocation("submit", "sync", submit(0)),
        None,
        false,
    );
    assert_eq!(
        data(&out, &path)["data"]["description"],
        "actual WASM result"
    );
    let event_log = fs::read_to_string(
        workspace
            .join("cli")
            .join(hash_bytes(b"sync"))
            .join("events.jsonl"),
    )
    .unwrap();
    let events: Vec<Value> = event_log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["sequence"], 1);
    assert_eq!(events[0]["payload"]["text"], "persist before ACK");
    let (out, path) = run(
        &workspace,
        config(),
        invocation("submit", "async", submit(1)),
        None,
        false,
    );
    assert_eq!(data(&out, &path)["kind"], "accepted");
    let opened = DevelopmentWorkspace::open(&workspace).unwrap();
    let store = CheckpointStore::open(&opened, SnapshotPolicy::new(vec![]).unwrap()).unwrap();
    let checkpoint = store.read("async", "async").unwrap().unwrap();
    assert_eq!(checkpoint["state"], "accepted");
    assert_eq!(checkpoint["vendorTaskId"], "cli-job-one");
    drop(store);
    let (out, path) = run(
        &workspace,
        config(),
        invocation("cancel", "cancel", task()),
        Some("async"),
        false,
    );
    assert_eq!(data(&out, &path)["outcome"], "remote_requested");
    let (out, path) = run(
        &workspace,
        config(),
        invocation("query", "query", task()),
        Some("async"),
        false,
    );
    assert_eq!(data(&out, &path)["status"], "succeeded");
    let store = CheckpointStore::open(&opened, SnapshotPolicy::new(vec![]).unwrap()).unwrap();
    assert_eq!(
        store.read("async", "async").unwrap().unwrap()["state"],
        "terminal"
    );
    let traces = fs::read_dir(workspace.join("traces"))
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!traces.contains("ambient-secret-must-never-be-discovered"));
}

#[test]
fn errors_are_nonzero_and_standard_receipts_are_preserved_without_retry() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("development");
    let (out, path) = run(
        &workspace,
        config(),
        invocation("capabilities", "caps", json!({})),
        None,
        true,
    );
    data(&out, &path);
    let mut request = submit(0);
    request["input"]["prompt"] = json!("fail");
    let (out, path) = run(
        &workspace,
        config(),
        invocation("submit", "failed", request),
        None,
        false,
    );
    assert_eq!(out.status.code(), Some(1));
    let response: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(response["error"]["code"], "UPSTREAM_ERROR");
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        response
    );
    SchemaRegistry::validate_public("response", &response).unwrap();
    let (out, path) = run(
        &workspace,
        config(),
        invocation("submit", "invalid", {
            let mut r = submit(0);
            r["input"]["strength"] = json!(2);
            r
        }),
        None,
        false,
    );
    assert_eq!(out.status.code(), Some(1));
    let response: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(response["error"]["code"], "INVALID_INPUT");
    let mut wrong = task();
    wrong["vendorTaskId"] = json!("foreign-job");
    let (out, path) = run(
        &workspace,
        config(),
        invocation("query", "no-checkpoint", wrong),
        Some("missing"),
        false,
    );
    assert_eq!(out.status.code(), Some(1));
    let response: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(response["error"]["code"], "RESOURCE_UNAVAILABLE");
    let before = fs::read_dir(workspace.join("cli")).unwrap().count();
    let (out, _) = run(
        &workspace,
        config(),
        invocation("submit", "failed", submit(0)),
        None,
        false,
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read_dir(workspace.join("cli")).unwrap().count(), before);
}

#[test]
fn no_arguments_or_implicit_configuration_and_unknown_provisioning_fail_closed() {
    let out = command(env!("CARGO_BIN_EXE_bgx-adapter-v2"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let diagnostic: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(diagnostic["code"], "INVALID_ARGUMENT");
    let out = command(env!("CARGO_BIN_EXE_bgx-adapter-v2"))
        .args(["submit"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let diagnostic: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(diagnostic["code"], "INVALID_ARGUMENT");
    for args in [
        vec!["submit", "--unknown"],
        vec!["submit", "--workspace", "relative"],
        vec!["submit", "--create-workspace", "--create-workspace"],
        vec!["submit", "--sdk", "C:/explicit", "--sdk", "C:/duplicate"],
    ] {
        let out = command(env!("CARGO_BIN_EXE_bgx-adapter-v2"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
    }
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("uncreated");
    let mut unknown = config();
    unknown["ambientCredentials"] = json!(true);
    let (out, path) = run(
        &workspace,
        unknown,
        invocation("capabilities", "caps", json!({})),
        None,
        true,
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(!workspace.exists());
    assert!(!path.exists());
}

#[test]
fn escaped_explicit_secret_and_protected_workspace_output_are_rejected_before_writes() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("development");
    let secret = "explicit-quote\"backslash\\secret";
    let secret_file = temporary.path().join("development-secret");
    fs::write(&secret_file, secret).unwrap();
    let mut provisioning = config();
    provisioning["credentials"] = json!([{"reference":"explicit-secret-reference","secretFile":secret_file,"publicId":null,"customHeader":null,"modes":["bearer"],"origins":[],"ttlMs":60000}]);
    let (out, path) = run(
        &workspace,
        provisioning,
        invocation("capabilities", "secret-input", json!({"leak":secret})),
        None,
        true,
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(!path.exists());
    assert!(!workspace.join("cli").exists());
    let diagnostic: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(diagnostic["code"], "PERMISSION_DENIED");
    assert!(!String::from_utf8_lossy(&out.stderr).contains(secret));

    let f = fixture();
    let configuration = temporary.path().join("config.json");
    let input = temporary.path().join("input.json");
    fs::write(&configuration, config().to_string()).unwrap();
    fs::write(
        &input,
        invocation("capabilities", "protected-output", json!({})).to_string(),
    )
    .unwrap();
    let marker = workspace.join(".bgx-sdk-workspace.json");
    let original = fs::read(&marker).unwrap();
    let out = command(env!("CARGO_BIN_EXE_bgx-adapter-v2"))
        .arg("caps")
        .arg("--sdk")
        .arg(&f.sdk)
        .arg("--worker")
        .arg(f.sdk.join(format!(
            "target/debug/bgx-adapter-v2-worker{}",
            std::env::consts::EXE_SUFFIX
        )))
        .arg("--package")
        .arg(f.package.path())
        .arg("--workspace")
        .arg(&workspace)
        .arg("--config")
        .arg(&configuration)
        .arg("--input")
        .arg(&input)
        .arg("--output")
        .arg(&marker)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read(&marker).unwrap(), original);
    assert!(!workspace.join("cli").exists());
}
