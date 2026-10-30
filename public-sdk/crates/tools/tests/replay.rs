//! Real CLI/worker/guest record and replay. The mock HTTP service counts requests
//! so successful replay cannot hide a fresh broker/network execution.
use bgx_adapter_v2_validator::hash_bytes;
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    thread,
    time::Duration,
};

const SEED: u64 = 41;
const NOW: u64 = 1_730_000_000_000;
fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
struct Fixture {
    _source: tempfile::TempDir,
    package: tempfile::TempDir,
    sdk: PathBuf,
    input_digest: String,
}
fn build(path: &Path, args: &[&str]) {
    let out = command("cargo")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(||{
        let sdk=Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        assert!(sdk.join(format!("target/debug/bgx-adapter-v2-worker{}",std::env::consts::EXE_SUFFIX)).is_file(),"Build the explicit SDK worker before replay tests");
        let source=tempfile::tempdir().unwrap();fs::create_dir(source.path().join("src")).unwrap();
        let path=sdk.join("sdk/rust").to_str().unwrap().replace('\\',"/");let path=path.strip_prefix("//?/").unwrap_or(&path);
        fs::write(source.path().join("Cargo.toml"),format!("[workspace]\n[package]\nname='record-replay-guest'\nversion='1.0.0'\nedition='2021'\n[lib]\ncrate-type=['cdylib']\n[dependencies]\nbgx-adapter-v2-sdk={{path='{path}'}}\nserde_json={{version='=1.0.149',default-features=false,features=['alloc']}}\n[profile.release]\npanic='abort'\n")).unwrap();
        let input=json!({"type":"object","properties":{"text":{"type":"string"},"seed":{"type":"integer","minimum":0},"nowUnixMs":{"type":"integer","minimum":0},"mode":{"type":"string","enum":["normal","sign"]}},"required":["text","seed","nowUnixMs","mode"],"additionalProperties":false}).to_string();
        let output=json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}).to_string();
        let config=json!({"type":"object","properties":{"endpointBaseUrl":{"type":"string","minLength":1}},"required":["endpointBaseUrl"],"additionalProperties":false}).to_string();
        let input_digest=hash_bytes(input.as_bytes());let output_digest=hash_bytes(output.as_bytes());let config_digest=hash_bytes(config.as_bytes());
        let caps=json!({"schemaVersion":2,"pluginId":"local.replay","pluginVersion":"1.0.0","capabilityContracts":[{"schemaVersion":2,"localId":"echo","capabilityKey":"local.replay/echo","transportSlot":"generic","modelIds":["fixture-model"],"configurationSchema":{"id":"config","digest":config_digest},"operations":[{"id":"local.replay/echo","version":1,"label":"Mock echo","inputSchema":{"id":"input","digest":input_digest},"outputSchema":{"id":"output","digest":output_digest},"resumeSchema":null,"uiSchema":{"controls":[{"fieldPointer":"/text","widget":"multiline","label":"Text","description":"","group":"Input"},{"fieldPointer":"/seed","widget":"number","label":"Seed","description":"","group":"Input"},{"fieldPointer":"/nowUnixMs","widget":"number","label":"Mock time","description":"","group":"Input"},{"fieldPointer":"/mode","widget":"select","label":"Fixture mode","description":"","group":"Input"}]},"mediaSlots":[],"execution":{"mode":"synchronous","streaming":true,"cancellation":"unsupported","resumable":false,"remoteIdempotency":false},"outputSlots":[]}],"schemas":[{"id":"input","digest":input_digest,"schemaJson":input},{"id":"output","digest":output_digest,"schemaJson":output},{"id":"config","digest":config_digest,"schemaJson":config}]}]});
        fs::write(source.path().join("src/capabilities.json"),caps.to_string()).unwrap();
        fs::write(source.path().join("src/lib.rs"),r###"
#![no_std]
extern crate alloc;
use alloc::{format,string::ToString,vec::Vec};use bgx_adapter_v2_sdk::{*,guest,host};use serde_json::{json,Value};
struct Guest;
impl Adapter for Guest{fn invoke(i:Invocation,h:&mut dyn Host)->Response{
 let v=serde_json::to_value(&i).unwrap();let p:Value=serde_json::from_str(v["payloadJson"].as_str().unwrap()).unwrap();
 let _:()=host::call(h,"host-log.log",&json!({"level":"info","code":"fixture.invoke","message":v["operation"]})).unwrap();
 let result=match v["operation"].as_str().unwrap(){
  "capabilities"=>serde_json::from_str(include_str!("capabilities.json")).unwrap(),
  "submit"=>{
   if p["input"]["mode"]=="sign" {
    let _:Result<Value,_>=host::call(h,"host-credential.sign",&json!({"handle":"explicit-fake-handle","specJson":json!({"version":1,"kind":"hmac-sha256","message":"fixture","encoding":"hex"}).to_string()}));
    return guest::completed_response(&i,&json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[],"data":{"text":"handled missing fake credential"},"usage":null})).unwrap();
   }
   let _:()=host::call(h,"host-state.put",&json!({"key":"text","value":p["input"]["text"]})).unwrap();
   let saved:Option<alloc::string::String>=host::call(h,"host-state.get",&json!({"key":"text"})).unwrap();assert_eq!(saved.as_deref(),p["input"]["text"].as_str());
   let response:Value=host::call(h,"host-http.execute",&json!({"plan":{"method":"GET","url":format!("{}/fixture?seed={}&now={}",p["configuration"]["endpointBaseUrl"].as_str().unwrap(),p["input"]["seed"],p["input"]["nowUnixMs"]),"headers":[],"generatedHeaders":[],"body":{"kind":"none"},"sink":{"kind":"buffer","maxBytes":4096},"timeoutMs":5000},"credential":null})).unwrap();assert_eq!(response["status"],200);
   let bytes:Vec<u8>=serde_json::from_value(response["payload"]["bytes"].clone()).unwrap();let remote:Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(remote["seed"],p["input"]["seed"]);assert_eq!(remote["nowUnixMs"],p["input"]["nowUnixMs"]);
   let _:()=host::call(h,"host-stream.emit-event",&json!({"event":{"requestId":v["requestId"],"sequence":1,"type":"text_delta","payload":{"channel":"answer","text":"actual recorded event"}}})).unwrap();
   json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[],"data":{"text":remote["text"]},"usage":null})
  },_=>panic!("fixture only supports discovery and submit")
 };guest::completed_response(&i,&result).unwrap()
}}
bgx_adapter_v2_sdk::export_adapter!(Guest);
"###).unwrap();
        build(source.path(),&["generate-lockfile","--offline"]);build(source.path(),&["build","--release","--locked","--offline","--target","wasm32-wasip2"]);
        let artifact=fs::read(source.path().join("target/wasm32-wasip2/release/record_replay_guest.wasm")).unwrap();let package=tempfile::tempdir().unwrap();fs::write(package.path().join("adapter.wasm"),&artifact).unwrap();
        let manifest=json!({"schemaVersion":2,"pluginId":"local.replay","displayName":"Deterministic mock replay fixture","version":"1.0.0","protocolRange":{"min":2,"max":2},"requiredFeatures":["stream-events-v1"],"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":hash_bytes(&artifact),"publisherId":null,"publisherKeyId":null,"requestedCapabilities":{"endpointPolicy":["userConfigurable","allowPrivateNetwork"],"networkHosts":[],"allowPrivateNetwork":true,"auth":[],"mediaSlots":[],"stateBytes":1024,"embeddedHeaders":[]},"limits":{"memoryBytes":67108864,"fuel":1000000000,"wallMs":5000,"outputBytes":1048576}});
        let bytes=serde_json::to_vec(&manifest).unwrap();fs::write(package.path().join("manifest.json"),&bytes).unwrap();fs::write(package.path().join("manifest.sha256"),format!("{} adapter.wasm\n{} manifest.json\n",hash_bytes(&artifact),hash_bytes(&bytes))).unwrap();
        Fixture{_source:source,package,sdk,input_digest}
    })
}
struct Server {
    origin: String,
    count: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let quit = stop.clone();
        let thread = thread::spawn(move || {
            while !quit.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        loop {
                            let mut byte = [0u8];
                            stream.read_exact(&mut byte).unwrap();
                            bytes.push(byte[0]);
                            if bytes.ends_with(b"\r\n\r\n") {
                                break;
                            }
                            assert!(bytes.len() < 32768);
                        }
                        let request = String::from_utf8(bytes).unwrap();
                        assert_eq!(
                            request.lines().next().unwrap(),
                            format!("GET /fixture?seed={SEED}&now={NOW} HTTP/1.1")
                        );
                        observed.fetch_add(1, Ordering::SeqCst);
                        let body =
                            json!({"text":"fixed mock service answer","seed":SEED,"nowUnixMs":NOW})
                                .to_string();
                        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                        stream.write_all(response.as_bytes()).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("mock service: {e}"),
                }
            }
        });
        Self {
            origin,
            count,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.thread.take() {
            if let Err(cause) = worker.join() {
                if thread::panicking() {
                    eprintln!("Mock server also failed; preserving the original test failure");
                } else {
                    std::panic::resume_unwind(cause);
                }
            }
        }
    }
}
struct Environment {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    config: PathBuf,
    input: PathBuf,
    transcript: PathBuf,
}
impl Environment {
    fn new(origin: &str, mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("development");
        let config = root.path().join("config.json");
        let input = root.path().join("request.json");
        let transcript = workspace.join("receipts/record.transcript.json");
        fs::write(&config,json!({"bindingId":"binding","bindingRevision":1,"configurationRef":"replay-fixture-config","configuration":{"endpointBaseUrl":origin},"trust":{"publishers":[],"revokedPlugins":[],"revokedVersions":[],"revokedKeys":[],"denylistedHosts":[],"isFresh":false,"supportedFeatures":["stream-events-v1"]},"network":{"origins":[origin],"allowPrivate":true,"allowEmbeddedAuthorization":false,"deniedHosts":[],"deniedNetworks":[]},"credentials":[],"media":[],"mediaTtlMs":60000,"transport":{"handshakeTimeoutMs":5000,"writeTimeoutMs":5000,"shutdownTimeoutMs":5000,"cancelGraceMs":100},"probe":null}).to_string()).unwrap();
        let payload = json!({"schemaVersion":2,"modelId":"fixture-model","capabilityKey":"local.replay/echo","operationId":"local.replay/echo","operationVersion":1,"inputSchemaDigest":fixture().input_digest,"input":{"text":"deterministic input","seed":SEED,"nowUnixMs":NOW,"mode":mode},"media":[],"uploads":[],"configuration":{"endpointBaseUrl":origin}});
        fs::write(&input,json!({"protocolVersion":2,"operation":"submit","profileId":"binding","bindingId":"binding","capabilitySlot":"generic","requestId":"recorded-submit","budgetMs":15000,"negotiatedVersion":2,"enabledFeatures":["stream-events-v1"],"payloadJson":payload.to_string()}).to_string()).unwrap();
        Self {
            _root: root,
            workspace,
            config,
            input,
            transcript,
        }
    }
    fn cli(&self, operation: &str, output: &str) -> Command {
        let f = fixture();
        let mut c = command(env!("CARGO_BIN_EXE_bgx-adapter-v2"));
        c.arg(operation)
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
            .arg(&self.workspace)
            .arg("--config")
            .arg(&self.config)
            .arg("--output")
            .arg(self.workspace.join("receipts").join(output));
        c
    }
    fn record(&self) -> Output {
        self.cli("record", "record.response.json")
            .arg("--input")
            .arg(&self.input)
            .arg("--transcript")
            .arg(&self.transcript)
            .arg("--seed")
            .arg(SEED.to_string())
            .arg("--now-unix-ms")
            .arg(NOW.to_string())
            .arg("--create-workspace")
            .output()
            .unwrap()
    }
    fn replay(&self, path: &Path, output: &str) -> Output {
        self.cli("replay", output)
            .arg("--transcript")
            .arg(path)
            .output()
            .unwrap()
    }
    fn conform(&self, manifest: &Path, output: &str) -> Output {
        self.cli("conform", output)
            .arg("--manifest")
            .arg(manifest)
            .arg("--evidence-root")
            .arg(self.workspace.join("receipts"))
            .output()
            .unwrap()
    }
}
fn success(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn rehash(value: &mut Value) {
    value["sha256"] = json!(hash_bytes(
        &serde_json::to_vec(&value["transcript"]).unwrap()
    ));
}
fn mutation(env: &Environment, name: &str, edit: impl FnOnce(&mut Value), hash: bool) -> PathBuf {
    let mut value: Value = serde_json::from_slice(&fs::read(&env.transcript).unwrap()).unwrap();
    edit(&mut value);
    if hash {
        rehash(&mut value);
    }
    let path = env.workspace.join("receipts").join(format!("{name}.json"));
    fs::write(&path, value.to_string()).unwrap();
    path
}

#[test]
fn actual_record_replay_uses_the_guest_port_tape_without_calling_the_live_mock_again() {
    let server = Server::new();
    let env = Environment::new(&server.origin, "normal");
    let response = success(&env.record());
    assert_eq!(response["operation"], "submit");
    let data: Value = serde_json::from_str(response["dataJson"].as_str().unwrap()).unwrap();
    assert_eq!(data["data"]["text"], "fixed mock service answer");
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    let original_tape = fs::read(&env.transcript).unwrap();
    let duplicate = env
        .cli("record", "duplicate-record.response.json")
        .arg("--input")
        .arg(&env.input)
        .arg("--transcript")
        .arg(&env.transcript)
        .arg("--seed")
        .arg(SEED.to_string())
        .arg("--now-unix-ms")
        .arg(NOW.to_string())
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(1));
    let diagnostic: Value = serde_json::from_slice(&duplicate.stderr).unwrap();
    assert_eq!(diagnostic["code"], "NO_OVERWRITE");
    assert_eq!(fs::read(&env.transcript).unwrap(), original_tape);
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    let tape: Value = serde_json::from_slice(&fs::read(&env.transcript).unwrap()).unwrap();
    assert_eq!(tape["transcript"]["scope"], "MOCK");
    assert_eq!(
        tape["transcript"]["mockInputs"],
        json!({"seed":SEED,"nowUnixMs":NOW})
    );
    assert_eq!(
        tape["transcript"]["invocations"].as_array().unwrap().len(),
        2
    );
    let replay = success(&env.replay(&env.transcript, "replay.report.json"));
    assert_eq!(replay["scope"], "MOCK");
    assert_eq!(replay["passed"], true);
    assert_eq!(replay["invocations"].as_array().unwrap().len(), 2);
    assert!(replay["invocations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["workerPid"].as_u64().unwrap() > 0));
    assert_eq!(
        server.count.load(Ordering::SeqCst),
        1,
        "Offline replay must never send a second HTTP request"
    );
}

#[test]
fn exact_port_order_count_response_hash_and_package_identity_are_enforced() {
    let server = Server::new();
    let env = Environment::new(&server.origin, "normal");
    success(&env.record());
    let variations: [(&str, Box<dyn FnOnce(&mut Value)>, bool); 7] = [
        (
            "unknown-field",
            Box::new(|v| v["unexpected"] = json!(true)),
            false,
        ),
        (
            "bad-hash",
            Box::new(|v| v["sha256"] = json!("0".repeat(64))),
            false,
        ),
        (
            "other-package",
            Box::new(|v| v["transcript"]["package"]["version"] = json!("9.9.9")),
            true,
        ),
        (
            "missing-port",
            Box::new(|v| {
                v["transcript"]["invocations"][1]["ports"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }),
            true,
        ),
        (
            "additional-port",
            Box::new(|v| {
                let ports = v["transcript"]["invocations"][1]["ports"]
                    .as_array_mut()
                    .unwrap();
                ports.push(ports[0].clone());
            }),
            true,
        ),
        (
            "changed-call",
            Box::new(|v| {
                let ports = v["transcript"]["invocations"][1]["ports"]
                    .as_array_mut()
                    .unwrap();
                let call = ports
                    .iter_mut()
                    .find(|p| p["call"]["method"] == "host-state.put")
                    .unwrap();
                call["call"]["args"]["key"] = json!("different-key");
            }),
            true,
        ),
        (
            "changed-response",
            Box::new(|v| {
                let mut data: Value = serde_json::from_str(
                    v["transcript"]["invocations"][1]["response"]["dataJson"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                data["data"]["text"] = json!("changed receipt");
                v["transcript"]["invocations"][1]["response"]["dataJson"] = json!(data.to_string());
            }),
            true,
        ),
    ];
    for (name, edit, hash) in variations {
        let path = mutation(&env, name, edit, hash);
        let out = env.replay(&path, &format!("{name}.report.json"));
        assert_eq!(
            out.status.code(),
            Some(1),
            "{name}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
}

fn manifest(env: &Environment, transcript: bool) -> Value {
    let evidence = env.workspace.join("receipts/evidence.json");
    fs::write(&evidence,json!({"kind":"SYNTHETIC_DECLARATION_FOR_CONFORM_TEST","doesNotAssertRealCasesWereExecuted":true}).to_string()).unwrap();
    let evidence_hash = hash_bytes(&fs::read(evidence).unwrap());
    let catalog: Value =
        serde_json::from_str(include_str!("../../../protocol/conformance-cases.json")).unwrap();
    json!({"schemaVersion":1,"cases":catalog["cases"].as_array().unwrap().iter().map(|c|{
        let artifacts=if transcript&&c["id"]=="C28"{json!([{"path":"record.transcript.json","sha256":hash_bytes(&fs::read(&env.transcript).unwrap()),"kind":"TRANSCRIPT"}])}else{json!([{"path":"evidence.json","sha256":evidence_hash,"kind":"EVIDENCE"}])};
        json!({"id":c["id"],"status":"PASS","mode":"MOCK","command":["synthetic-external-evidence-declaration"],"exitCode":0,"artifacts":artifacts,"reason":"Only the replay case is actually executed by this tool; other statuses are fixture declarations"})
    }).collect::<Vec<_>>()})
}
#[test]
fn conform_requires_the_fixed_cases_and_actual_replay_and_labels_declared_evidence() {
    let server = Server::new();
    let env = Environment::new(&server.origin, "normal");
    success(&env.record());
    let path = env._root.path().join("evidence-manifest.json");
    fs::write(&path, manifest(&env, false).to_string()).unwrap();
    let out = env.conform(&path, "without-runtime.json");
    assert_eq!(out.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["integrityComplete"], true);
    assert_eq!(report["conformant"], false);
    fs::write(&path, manifest(&env, true).to_string()).unwrap();
    let report = success(&env.conform(&path, "with-runtime.json"));
    assert_eq!(report["conformant"], true);
    assert_eq!(report["liveActuallyVerified"], false);
    assert_eq!(report["requiredCases"], 36);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(
        cases.iter().find(|c| c["id"] == "C28").unwrap()["source"],
        "ACTUALLY_EXECUTED"
    );
    assert!(cases
        .iter()
        .filter(|c| c["id"] != "C28")
        .all(|c| c["source"] == "EVIDENCE_DECLARATION"));
    let mut missing = manifest(&env, true);
    missing["cases"].as_array_mut().unwrap().remove(0);
    fs::write(&path, missing.to_string()).unwrap();
    let out = env.conform(&path, "missing-case.json");
    assert_eq!(out.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["integrityComplete"], false);
    assert_eq!(report["conformant"], false);
    assert_eq!(report["cases"][0]["declaredStatus"], "NOT_RUN");
    assert_eq!(
        server.count.load(Ordering::SeqCst),
        1,
        "Conformance replay also has no real HTTP broker"
    );
}
#[test]
fn credential_signing_is_explicitly_unrecordable_and_cannot_leave_a_successful_tape() {
    let server = Server::new();
    let env = Environment::new(&server.origin, "sign");
    let out = env.record();
    assert_eq!(out.status.code(), Some(1));
    assert!(!env.transcript.exists());
    assert_eq!(server.count.load(Ordering::SeqCst), 0);
}
