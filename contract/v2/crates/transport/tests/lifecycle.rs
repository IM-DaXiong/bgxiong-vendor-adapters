use bgx_adapter_v2_ports::{resource_policy, write_frame};
use bgx_adapter_v2_transport::{
    wire::{
        host::HostError, HostPortCallV2, Invocation, WorkerEnvelopeV2,
        WorkerEnvelopeV2Branch1Package,
    },
    *,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::time::timeout;

fn invocation(id: &str) -> Invocation {
    serde_json::from_value(json!({"protocolVersion":2,"operation":"submit","profileId":"test","bindingId":"test","capabilitySlot":"generic","requestId":id,"budgetMs":1000,"negotiatedVersion":2,"enabledFeatures":[],"payloadJson":"{}"})).unwrap()
}
fn identity() -> WorkerEnvelopeV2Branch1Package {
    serde_json::from_value(
        json!({"pluginId":"local.transport","version":"1.0.0","digest":"0".repeat(64)}),
    )
    .unwrap()
}
fn response(id: &str) -> Value {
    json!({"schemaVersion":2,"kind":"response","requestId":id,"response":{"protocolVersion":2,"operation":"submit","dataJson":"{}"}})
}
fn options() -> Options {
    Options {
        handshake_timeout: Duration::from_secs(2),
        write_timeout: Duration::from_secs(2),
        shutdown_timeout: Duration::from_secs(2),
        cancel_grace: Duration::from_millis(100),
    }
}
struct Host {
    calls: Arc<AtomicUsize>,
    pending: bool,
}
impl ManagedHost for Host {
    fn call<'a>(&'a mut self, call: HostPortCallV2, _control: &'a Control) -> PortFuture<'a> {
        Box::pin(async move {
            assert_eq!(
                serde_json::to_value(call).unwrap()["method"],
                "host-log.log"
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.pending {
                std::future::pending::<Result<Value, HostError>>().await
            } else {
                Ok(Value::Null)
            }
        })
    }
}
fn host() -> Box<dyn ManagedHost> {
    Box::new(Host {
        calls: Arc::new(AtomicUsize::new(0)),
        pending: false,
    })
}
fn package(parent: &Path, mode: &str, frames: &[Value]) -> PathBuf {
    let root = parent.join(mode);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("mode"), mode).unwrap();
    let mut bytes = vec![];
    for value in frames {
        write_frame(
            &mut bytes,
            &serde_json::from_value::<WorkerEnvelopeV2>(value.clone()).unwrap(),
        )
        .unwrap();
    }
    fs::write(root.join("frames.bin"), bytes).unwrap();
    root
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("protocol")).unwrap();
    fs::write(
        root.path().join("protocol/schema-policy.json"),
        include_bytes!("../../../protocol/schema-policy.json"),
    )
    .unwrap();
    let source = r###"
use std::{fs,io::{self,Read,Write},path::PathBuf,thread,time::Duration};
fn read(r:&mut impl Read)->Vec<u8>{let mut h=[0;4];r.read_exact(&mut h).unwrap();let mut b=vec![0;u32::from_le_bytes(h) as usize];r.read_exact(&mut b).unwrap();b}
fn write(w:&mut impl Write,b:&[u8]){w.write_all(&(b.len() as u32).to_le_bytes()).unwrap();w.write_all(b).unwrap();w.flush().unwrap();}
fn main(){let mut args=std::env::args_os().skip(1);assert_eq!(args.next().unwrap(),"--package");let root=PathBuf::from(args.next().unwrap());assert!(args.next().is_none());assert_eq!(std::env::vars_os().count(),0,"worker inherited ambient secrets/environment");
let mode=fs::read_to_string(root.join("mode")).unwrap();let mut stdin=io::stdin();let mut stdout=io::stdout();let hello=read(&mut stdin);
if mode=="handshake-hang"{thread::sleep(Duration::from_secs(60));return;}
if mode=="wrong-handshake"{write(&mut stdout,br#"{}"#);return;}
if mode=="policy-mismatch"||mode=="version-mismatch"{write(&mut stdout,&fs::read(root.join("hello.json")).unwrap());return;}
write(&mut stdout,&hello);let _invoke=read(&mut stdin);fs::write(root.join("started"),std::process::id().to_string()).unwrap();
match mode.as_str(){"hang"=>{thread::sleep(Duration::from_secs(60));return;},"no-response"=>return,
"truncated"=>{stdout.write_all(&100u32.to_le_bytes()).unwrap();stdout.write_all(b"{").unwrap();return;},
"oversized"=>{stdout.write_all(&u32::MAX.to_le_bytes()).unwrap();return;},
"stderr"=>{eprint!("{}TAIL", "x".repeat(20000));},_=>{}}
stdout.write_all(&fs::read(root.join("frames.bin")).unwrap()).unwrap();stdout.flush().unwrap();
if mode=="port"||mode=="cancel"||mode=="drop"{let result=read(&mut stdin);fs::write(root.join("parent-result"),&result).unwrap();
if mode=="port"{stdout.write_all(&fs::read(root.join("final.bin")).unwrap()).unwrap();stdout.flush().unwrap();}}
if mode=="bad-exit"{std::process::exit(3);}
}
"###;
    fs::write(root.path().join("worker.rs"), source).unwrap();
    let mut command = std::process::Command::new("rustc");
    command
        .arg("--edition=2021")
        .arg(root.path().join("worker.rs"))
        .arg("-o")
        .arg(worker(root.path()));
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
    root
}
fn worker(root: &Path) -> PathBuf {
    root.join(if cfg!(windows) {
        "bgx-adapter-v2-worker.exe"
    } else {
        "bgx-adapter-v2-worker"
    })
}
fn client(root: &Path) -> WorkerClient {
    WorkerClient::new(root, &worker(root), options()).unwrap()
}
async fn idle(client: &WorkerClient) {
    timeout(Duration::from_secs(5), client.wait_idle())
        .await
        .unwrap();
    assert_eq!(client.active_workers(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_lifecycle_rejects_malformed_missing_duplicate_and_cross_request_frames() {
    let sdk = fixture();
    let client = client(sdk.path());
    let packages = tempfile::tempdir().unwrap();
    let root = package(packages.path(), "normal", &[response("request")]);
    let receipt = client
        .invoke(
            root,
            identity(),
            invocation("request"),
            host(),
            Control::new(None),
        )
        .await
        .unwrap();
    assert!(serde_json::to_value(receipt.response)
        .unwrap()
        .get("dataJson")
        .is_some());
    idle(&client).await;
    for (mode, frames, kind) in [
        (
            "cross-request",
            vec![response("another")],
            FaultKind::Protocol,
        ),
        (
            "duplicate",
            vec![response("request"), response("request")],
            FaultKind::Protocol,
        ),
        ("bad-exit", vec![response("request")], FaultKind::Exited),
        ("no-response", vec![], FaultKind::Exited),
        ("truncated", vec![], FaultKind::Protocol),
        ("oversized", vec![], FaultKind::Protocol),
        ("wrong-handshake", vec![], FaultKind::Protocol),
        ("handshake-hang", vec![], FaultKind::Handshake),
    ] {
        let root = package(packages.path(), mode, &frames);
        let error = client
            .invoke(
                root,
                identity(),
                invocation("request"),
                host(),
                Control::new(None),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, kind, "{mode}: {error:?}");
        assert_eq!(error.invocation_dispatched, !mode.contains("handshake"));
        idle(&client).await;
    }
    let root = package(packages.path(), "stderr", &[response("request")]);
    let receipt = client
        .invoke(
            root,
            identity(),
            invocation("request"),
            host(),
            Control::new(None),
        )
        .await
        .unwrap();
    assert_eq!(
        receipt.stderr_tail.len() as u64,
        resource_policy()["limits"]["maxDiagnosticBytes"]
            .as_u64()
            .unwrap()
    );
    assert!(receipt.stderr_tail.ends_with(b"TAIL"));
    idle(&client).await;
    for (mode, version, kind) in [
        ("policy-mismatch", 2, FaultKind::Handshake),
        ("version-mismatch", 3, FaultKind::Protocol),
    ] {
        let root = package(packages.path(), mode, &[]);
        fs::write(root.join("hello.json"),json!({"schemaVersion":2,"kind":"hello","protocolVersion":version,"witPackage":"bgxiong:vendor-adapter@2.0.0","policyDigest":"0".repeat(64)}).to_string()).unwrap();
        let fault = client
            .invoke(
                root,
                identity(),
                invocation("request"),
                host(),
                Control::new(None),
            )
            .await
            .unwrap_err();
        assert_eq!(fault.kind, kind);
        assert!(!fault.invocation_dispatched);
        idle(&client).await;
    }
    assert!(WorkerClient::new(sdk.path(), Path::new("bgx-adapter-v2-worker"), options()).is_err());
    let outside = tempfile::tempdir().unwrap();
    fs::copy(worker(sdk.path()), worker(outside.path())).unwrap();
    assert!(WorkerClient::new(sdk.path(), &worker(outside.path()), options()).is_err());
    assert!(WorkerClient::new(
        sdk.path(),
        &sdk.path().join("missing-worker.exe"),
        options()
    )
    .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_calls_cancel_deadline_and_dropped_caller_are_scoped_and_reaped() {
    let sdk = fixture();
    let client = client(sdk.path());
    let packages = tempfile::tempdir().unwrap();
    let call = json!({"schemaVersion":2,"kind":"port_call","requestId":"request","callId":"1","call":{"method":"host-log.log","args":{"level":"info","code":"test","message":"managed"}}});
    let root = package(packages.path(), "port", &[call.clone()]);
    let mut final_frame = vec![];
    write_frame(
        &mut final_frame,
        &serde_json::from_value(response("request")).unwrap(),
    )
    .unwrap();
    fs::write(root.join("final.bin"), &final_frame).unwrap();
    client
        .invoke(
            root.clone(),
            identity(),
            invocation("request"),
            host(),
            Control::new(None),
        )
        .await
        .unwrap();
    let port: Value =
        serde_json::from_slice(&fs::read(root.join("parent-result")).unwrap()).unwrap();
    assert_eq!(port["kind"], "port_result");
    assert_eq!(port["callId"], "1");
    assert_eq!(port["result"], json!({"ok":null}));
    idle(&client).await;
    let mut duplicate = call.clone();
    duplicate["callId"] = json!("2");
    let root = package(packages.path(), "bad-call", &[duplicate]);
    assert_eq!(
        client
            .invoke(
                root,
                identity(),
                invocation("request"),
                host(),
                Control::new(None)
            )
            .await
            .unwrap_err()
            .kind,
        FaultKind::Protocol
    );
    idle(&client).await;
    let root = package(
        packages.path(),
        "premature",
        &[call.clone(), response("request")],
    );
    assert_eq!(
        client
            .invoke(
                root,
                identity(),
                invocation("request"),
                Box::new(Host {
                    calls: Arc::new(AtomicUsize::new(0)),
                    pending: true
                }),
                Control::new(None)
            )
            .await
            .unwrap_err()
            .kind,
        FaultKind::Protocol
    );
    idle(&client).await;
    let root = package(packages.path(), "cancel", &[call.clone()]);
    let calls = Arc::new(AtomicUsize::new(0));
    let control = Control::new(None);
    let cancel = control.clone();
    let worker_client = client.clone();
    let count = calls.clone();
    let cancel_root = root.clone();
    let pending = tokio::spawn(async move {
        worker_client
            .invoke(
                cancel_root,
                identity(),
                invocation("request"),
                Box::new(Host {
                    calls: count,
                    pending: true,
                }),
                control,
            )
            .await
    });
    timeout(Duration::from_secs(3), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    let error = pending.await.unwrap().unwrap_err();
    assert_eq!(error.kind, FaultKind::Cancelled);
    assert!(error.invocation_dispatched);
    idle(&client).await;
    let parent: Value =
        serde_json::from_slice(&fs::read(root.join("parent-result")).unwrap()).unwrap();
    assert_eq!(parent["kind"], "cancel");
    let root = package(packages.path(), "hang", &[]);
    let error = client
        .invoke(
            root,
            identity(),
            invocation("request"),
            host(),
            Control::new(Some(Instant::now() + Duration::from_millis(500))),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, FaultKind::Deadline);
    idle(&client).await;
    let root = package(packages.path(), "drop", &[call]);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let worker_client = client.clone();
    let pending = tokio::spawn(async move {
        worker_client
            .invoke(
                root,
                identity(),
                invocation("request"),
                Box::new(Host {
                    calls: count,
                    pending: true,
                }),
                Control::new(None),
            )
            .await
    });
    timeout(Duration::from_secs(3), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    pending.abort();
    let _ = pending.await;
    idle(&client).await;
    // A cancelled/dropped request never poisons a later or concurrent worker.
    let a = package(packages.path(), "next-a", &[response("a")]);
    let b = package(packages.path(), "next-b", &[response("b")]);
    let (a, b) = tokio::join!(
        client.invoke(a, identity(), invocation("a"), host(), Control::new(None)),
        client.invoke(b, identity(), invocation("b"), host(), Control::new(None))
    );
    assert_ne!(a.unwrap().worker_pid, b.unwrap().worker_pid);
    idle(&client).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_wasm_component_runs_only_through_explicit_sdk_worker_and_managed_ports() {
    use sha2::{Digest, Sha256};
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    let sdk_crate = sdk.join("sdk/rust").to_str().unwrap().replace('\\', "/");
    let sdk_crate = sdk_crate.strip_prefix("//?/").unwrap_or(&sdk_crate);
    fs::write(root.path().join("Cargo.toml"),format!("[workspace]\n[package]\nname='transport-guest-fixture'\nversion='1.0.0'\nedition='2021'\n[lib]\ncrate-type=['cdylib']\n[dependencies]\nbgx-adapter-v2-sdk={{path='{sdk_crate}'}}\nserde_json={{version='=1.0.149',default-features=false,features=['alloc']}}\n[profile.release]\npanic='abort'\n")).unwrap();
    fs::write(root.path().join("src/lib.rs"),r#"
#![no_std]
extern crate alloc;
use bgx_adapter_v2_sdk::{*,host,guest};use serde_json::json;
struct Guest;
impl Adapter for Guest{fn invoke(request:Invocation,host:&mut dyn Host)->Response{
let _:()=host::call(host,"host-log.log",&json!({"level":"info","code":"transport","message":"actual WASM"})).unwrap();
guest::completed_response(&request,&json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[],"data":{"description":"actual managed guest"},"usage":null})).unwrap()
}}
bgx_adapter_v2_sdk::export_adapter!(Guest);
"#).unwrap();
    let build = |directory: &Path, args: &[&str]| {
        let output = std::process::Command::new("cargo")
            .current_dir(directory)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    build(root.path(), &["generate-lockfile", "--offline"]);
    build(
        root.path(),
        &[
            "build",
            "--offline",
            "--locked",
            "--release",
            "--target",
            "wasm32-wasip2",
        ],
    );
    build(
        &sdk,
        &[
            "build",
            "--offline",
            "--locked",
            "-p",
            "bgx-adapter-v2-runtime",
            "--bin",
            "bgx-adapter-v2-worker",
        ],
    );
    let bytes = fs::read(
        root.path()
            .join("target/wasm32-wasip2/release/transport_guest_fixture.wasm"),
    )
    .unwrap();
    let sha = |b: &[u8]| hex::encode(Sha256::digest(b));
    let package = tempfile::tempdir().unwrap();
    fs::write(package.path().join("adapter.wasm"), &bytes).unwrap();
    let manifest = json!({"schemaVersion":2,"pluginId":"local.transport","displayName":"Transport fixture","version":"1.0.0","protocolRange":{"min":2,"max":2},"requiredFeatures":[],"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":sha(&bytes),"publisherId":null,"publisherKeyId":null,
        "requestedCapabilities":{"endpointPolicy":["staticHosts"],"networkHosts":[],"allowPrivateNetwork":false,"auth":[],"mediaSlots":[],"stateBytes":0,"embeddedHeaders":[]},"limits":{"memoryBytes":33554432,"fuel":100000000,"wallMs":1000,"outputBytes":1048576}});
    let manifest = serde_json::to_vec(&manifest).unwrap();
    fs::write(package.path().join("manifest.json"), &manifest).unwrap();
    let inventory = format!(
        "{} adapter.wasm\n{} manifest.json\n",
        sha(&bytes),
        sha(&manifest)
    );
    fs::write(package.path().join("manifest.sha256"), &inventory).unwrap();
    let package_id = serde_json::from_value(
        json!({"pluginId":"local.transport","version":"1.0.0","digest":sha(inventory.as_bytes())}),
    )
    .unwrap();
    let client = WorkerClient::new(&sdk, &worker(&sdk.join("target/debug")), options()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let receipt = client
        .invoke(
            package.path().to_path_buf(),
            package_id,
            invocation("actual"),
            Box::new(Host {
                calls: calls.clone(),
                pending: false,
            }),
            Control::new(Some(Instant::now() + Duration::from_secs(30))),
        )
        .await
        .unwrap();
    let response = serde_json::to_value(receipt.response).unwrap();
    let result: Value = serde_json::from_str(response["dataJson"].as_str().unwrap()).unwrap();
    assert_eq!(result["data"]["description"], "actual managed guest");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    idle(&client).await;
    let tree = std::process::Command::new("cargo")
        .current_dir(&sdk)
        .args([
            "tree",
            "--locked",
            "--offline",
            "-p",
            "bgx-adapter-v2-transport",
            "--edges",
            "normal",
        ])
        .output()
        .unwrap();
    assert!(tree.status.success());
    assert!(
        !String::from_utf8_lossy(&tree.stdout).contains("wasmtime"),
        "transport must not link the VM kernel"
    );
}

#[tokio::test]
async fn framing_is_bounded_strict_and_handles_split_reads() {
    use tokio::io::AsyncWriteExt;
    let frame: WorkerEnvelopeV2 = serde_json::from_value(response("request")).unwrap();
    let mut expected = vec![];
    write_frame(&mut expected, &frame).unwrap();
    let (mut writer, mut reader) = tokio::io::duplex(2);
    let bytes = expected.clone();
    let send = tokio::spawn(async move {
        for byte in bytes {
            writer.write_all(&[byte]).await.unwrap();
        }
    });
    assert_eq!(
        serde_json::to_value(framing::read(&mut reader).await.unwrap().unwrap()).unwrap(),
        serde_json::to_value(frame).unwrap()
    );
    send.await.unwrap();
    assert!(framing::read(&mut reader).await.unwrap().is_none());
    for bytes in [
        vec![1],
        vec![0, 0, 0, 0],
        u32::MAX.to_le_bytes().to_vec(),
        vec![1, 0, 0, 0, b'{'],
    ] {
        assert!(framing::read(&mut bytes.as_slice()).await.is_err());
    }
}
