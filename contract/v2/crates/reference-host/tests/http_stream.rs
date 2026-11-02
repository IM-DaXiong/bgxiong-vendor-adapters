use bgx_adapter_v2_ports::{resource_policy, wire::HostPortCallV2HttpRequest};
use bgx_adapter_v2_reference_host::{credential::*, http::*, http_stream::*, media::*, policy::*};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{hash_bytes, package::*};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::JoinSet,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Service {
    origin: String,
    gate: Arc<Notify>,
    requests: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    produced: Arc<AtomicUsize>,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
const FIRST: &[u8] = b"\0\xff\r\n{\"opaque\":true}\n";
const SECOND: &[u8] = b"second arbitrary bytes";
const LARGE: usize = 268435456;
impl Service {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let gate = Arc::new(Notify::new());
        let pass = gate.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let active = Arc::new(AtomicUsize::new(0));
        let open = active.clone();
        let produced = Arc::new(AtomicUsize::new(0));
        let pushed = produced.clone();
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {_=stopping.cancelled()=>break,Some(_)=clients.join_next(),if !clients.is_empty()=>{},accepted=listener.accept()=>{let (mut socket,_)=accepted.unwrap();count.fetch_add(1,Ordering::SeqCst);open.fetch_add(1,Ordering::SeqCst);let active=Active(open.clone());let gate=pass.clone();let produced=pushed.clone();clients.spawn(async move{let _active=active;
                    let mut header=vec![];let mut byte=[0;1];while !header.ends_with(b"\r\n\r\n"){if socket.read_exact(&mut byte).await.is_err(){return;}header.push(byte[0]);assert!(header.len()<65536);}let text=String::from_utf8(header).unwrap();let path=text.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                    match path{
                        "/audio"=>{let length=text.lines().filter_map(|line| line.split_once(':')).find(|(name,_)| name.eq_ignore_ascii_case("content-length")).map(|(_,value)| value.trim().parse::<usize>().unwrap()).unwrap_or(0);let mut body=vec![0;length];if socket.read_exact(&mut body).await.is_err(){return;}let wav = test_wav();let header=format!("HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",wav.len());let _=socket.write_all(header.as_bytes()).await;let _=socket.write_all(&wav).await;},
                        "/gated"=>{let headers=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",FIRST.len()+SECOND.len());if socket.write_all(headers.as_bytes()).await.is_err(){return;}if socket.write_all(FIRST).await.is_err(){return;}gate.notified().await;let _=socket.write_all(SECOND).await;},
                        "/broken"=>{let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\npartial").await;},
                        "/large"=>{let headers=format!("HTTP/1.1 200 OK\r\nContent-Length: {LARGE}\r\nConnection: close\r\n\r\n");if socket.write_all(headers.as_bytes()).await.is_err(){return;}let bytes=vec![0x58;65536];for _ in 0..LARGE/bytes.len(){if socket.write_all(&bytes).await.is_err(){return;}produced.fetch_add(bytes.len(),Ordering::SeqCst);}},
                        _=>{if socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n").await.is_err(){return;}let _=socket.read(&mut byte).await;},
                    }
                });}}
            }
            clients.abort_all();
            while clients.join_next().await.is_some() {}
        });
        Self {
            origin,
            gate,
            requests,
            active,
            produced,
            stop,
            task,
        }
    }
    async fn closed(&self) {
        timeout(Duration::from_secs(2), async {
            while self.active.load(Ordering::SeqCst) > 0 {
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn stop(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
fn broker(origin: &str) -> (tempfile::TempDir, Arc<HttpBroker>) {
    let manifest = json!({"schemaVersion":2,"pluginId":"local.stream","displayName":"Stream test","version":"1.0.0","protocolRange":{"min":2,"max":2},"requiredFeatures":["stream-events-v1"],"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":hash_bytes(b"test"),"publisherId":null,"publisherKeyId":null,"requestedCapabilities":{"endpointPolicy":["userConfigurable","allowPrivateNetwork"],"networkHosts":[],"allowPrivateNetwork":true,"auth":["none"],"mediaSlots":[],"stateBytes":0,"embeddedHeaders":[]},"limits":{"memoryBytes":33554432,"fuel":1000,"wallMs":100,"outputBytes":1048576}});
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let hashes = BTreeMap::from([
        ("manifest.json".into(), hash_bytes(&bytes)),
        ("adapter.wasm".into(), hash_bytes(b"test")),
    ]);
    let inventory = hashes
        .iter()
        .map(|(k, v)| format!("{v} {k}\n"))
        .collect::<String>();
    let mut trust = TrustContext::default();
    trust.supported_features.insert("stream-events-v1".into());
    let package = verify_package(
        &PackageContents {
            manifest_bytes: &bytes,
            hash_manifest_bytes: inventory.as_bytes(),
            signature: None,
            measured_file_hashes: &hashes,
        },
        &trust,
    )
    .unwrap();
    let scope = LeaseScope {
        package: package.identity().clone(),
        binding_id: "test".into(),
        request_id: "request".into(),
    };
    let policy = NetworkPolicy::new(
        &package,
        scope.clone(),
        DevelopmentNetworkGrant {
            origins: vec![origin.into()],
            allow_private: true,
            allow_embedded_authorization: false,
            denied_hosts: vec![],
            denied_networks: vec![],
        },
        true,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let media = MediaAccess {
        broker: Arc::new(Mutex::new(
            MediaBroker::new(workspace, Duration::from_secs(60), None).unwrap(),
        )),
        scope,
    };
    let http = HttpBroker::new(
        policy,
        Arc::new(Mutex::new(CredentialVault::new().unwrap())),
        media,
    )
    .unwrap();
    (directory, Arc::new(http))
}
fn request(url: &str, timeout_ms: u64) -> HostPortCallV2HttpRequest {
    serde_json::from_value(json!({"method":"GET","url":url,"headers":[],"generatedHeaders":[],"body":{"kind":"none"},"sink":{"kind":"buffer","maxBytes":1048576},"timeoutMs":timeout_ms})).unwrap()
}
fn features() -> BTreeSet<String> {
    BTreeSet::from(["stream-events-v1".into()])
}
fn id(response: impl serde::Serialize) -> String {
    serde_json::to_value(response).unwrap()["streamId"]
        .as_str()
        .unwrap()
        .into()
}
fn chunk(result: impl serde::Serialize) -> (Vec<u8>, bool) {
    let value = serde_json::to_value(result).unwrap();
    (
        serde_json::from_value(value["bytes"].clone()).unwrap(),
        value["eof"].as_bool().unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_chunk_precedes_eof_and_opaque_bytes_survive_partial_reads() {
    let service = Service::start().await;
    let (_directory, http) = broker(&service.origin);
    let scope = http.policy.scope().clone();
    let control = Control::new(None);
    let mut streams = HttpStreams::new(http.clone(), features());
    let stream = id(streams
        .open(
            &request(&format!("{}/gated", service.origin), 2000),
            None,
            &control,
        )
        .await
        .unwrap());
    let (first, eof) = chunk(
        timeout(
            Duration::from_millis(500),
            streams.read(&scope, &stream, 2, &control),
        )
        .await
        .unwrap()
        .unwrap(),
    );
    assert_eq!(first, FIRST[..2]);
    assert!(!eof);
    assert_eq!(service.active.load(Ordering::SeqCst), 1);
    service.gate.notify_one();
    let mut all = first;
    loop {
        let (bytes, eof) = chunk(streams.read(&scope, &stream, 3, &control).await.unwrap());
        assert!(bytes.len() <= 3);
        all.extend(bytes);
        if eof {
            break;
        }
    }
    assert_eq!(all, [FIRST, SECOND].concat());
    assert_eq!(
        chunk(streams.read(&scope, &stream, 3, &control).await.unwrap()),
        (vec![], true)
    );
    streams.close(&scope, &stream).unwrap();
    assert_eq!(streams.active_streams(), 0);
    assert_eq!(service.requests.load(Ordering::SeqCst), 1);
    service.closed().await;
    service.stop().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_consumer_has_bounded_retention_and_closing_releases_the_connection() {
    let service = Service::start().await;
    let (_directory, http) = broker(&service.origin);
    let scope = http.policy.scope().clone();
    let control = Control::new(None);
    let mut streams = HttpStreams::new(http, features());
    let stream = id(streams
        .open(
            &request(&format!("{}/large", service.origin), 5000),
            None,
            &control,
        )
        .await
        .unwrap());
    for _ in 0..12 {
        let (bytes, eof) = chunk(streams.read(&scope, &stream, 1, &control).await.unwrap());
        assert_eq!(bytes, vec![0x58]);
        assert!(!eof);
        assert!(
            streams.retained_bytes() as u64
                <= resource_policy()["limits"]["maxBufferedHttpBytes"]
                    .as_u64()
                    .unwrap()
        );
        sleep(Duration::from_millis(2)).await;
    }
    assert!(
        service.produced.load(Ordering::SeqCst) < LARGE,
        "producer unexpectedly drained the complete response without consumer reads"
    );
    streams.close(&scope, &stream).unwrap();
    assert_eq!(streams.retained_bytes(), 0);
    service.closed().await;
    assert_eq!(service.requests.load(Ordering::SeqCst), 1);
    service.stop().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interruptions_cancel_timeout_drop_and_foreign_handles_never_turn_into_eof_or_retry() {
    let service = Service::start().await;
    let (_directory, http) = broker(&service.origin);
    let scope = http.policy.scope().clone();
    let mut streams = HttpStreams::new(http.clone(), features());
    let control = Control::new(None);
    let stream = id(streams
        .open(
            &request(&format!("{}/broken", service.origin), 2000),
            None,
            &control,
        )
        .await
        .unwrap());
    loop {
        match streams.read(&scope, &stream, 10, &control).await {
            Ok(part) => {
                let (_, eof) = chunk(part);
                assert!(!eof, "truncated response falsely returned EOF");
            }
            Err(error) => {
                assert_eq!(
                    serde_json::to_value(error).unwrap()["code"],
                    "RESOURCE_UNAVAILABLE"
                );
                break;
            }
        }
    }
    assert_eq!(streams.active_streams(), 0);
    assert!(streams.read(&scope, &stream, 1, &control).await.is_err());
    assert_eq!(service.requests.load(Ordering::SeqCst), 1);
    for mode in ["cancel", "timeout", "drop"] {
        let c = Control::new(None);
        let stream = id(streams
            .open(
                &request(
                    &format!("{}/hang", service.origin),
                    if mode == "timeout" { 80 } else { 0 },
                ),
                None,
                &c,
            )
            .await
            .unwrap());
        let mut foreign = scope.clone();
        foreign.request_id = "foreign".into();
        assert!(streams.close(&foreign, &stream).is_err());
        assert!(streams.read(&foreign, &stream, 1, &c).await.is_err());
        assert_eq!(streams.active_streams(), 1);
        assert!(streams.read(&scope, &stream, 0, &c).await.is_err());
        if mode == "cancel" {
            let cancel = c.clone();
            let interruption = async move {
                sleep(Duration::from_millis(30)).await;
                cancel.cancel();
            };
            let (result, _) = tokio::join!(streams.read(&scope, &stream, 1, &c), interruption);
            assert_eq!(
                serde_json::to_value(result.unwrap_err()).unwrap()["code"],
                "CANCELLED"
            );
        } else if mode == "timeout" {
            assert_eq!(
                serde_json::to_value(streams.read(&scope, &stream, 1, &c).await.unwrap_err())
                    .unwrap()["code"],
                "TIMEOUT"
            );
        } else {
            {
                let pending = streams.read(&scope, &stream, 1, &c);
                tokio::pin!(pending);
                tokio::select! {result=&mut pending=>panic!("pending stream finished: {result:?}"),_=sleep(Duration::from_millis(30))=>{}}
            }
        }
        assert_eq!(streams.active_streams(), 0);
        service.closed().await;
    }
    assert_eq!(service.requests.load(Ordering::SeqCst), 4);
    let mut no_feature = HttpStreams::new(http, BTreeSet::new());
    assert!(no_feature
        .open(
            &request(&format!("{}/hang", service.origin), 100),
            None,
            &control
        )
        .await
        .is_err());
    assert_eq!(service.requests.load(Ordering::SeqCst), 4);
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_stream_budget_and_live_permission_revocation_release_owned_handles() {
    let service = Service::start().await;
    let (_directory, http) = broker(&service.origin);
    let scope = http.policy.scope().clone();
    let control = Control::new(None);
    let mut streams = HttpStreams::new(http.clone(), features());
    let mut tiny =
        serde_json::to_value(request(&format!("{}/gated", service.origin), 2000)).unwrap();
    tiny["sink"]["maxBytes"] = json!(1);
    let stream = id(streams
        .open(&serde_json::from_value(tiny).unwrap(), None, &control)
        .await
        .unwrap());
    assert_eq!(
        serde_json::to_value(
            streams
                .read(&scope, &stream, 1, &control)
                .await
                .unwrap_err()
        )
        .unwrap()["code"],
        "BUDGET_EXCEEDED"
    );
    assert_eq!(streams.active_streams(), 0);
    service.gate.notify_one();
    service.closed().await;
    let stream = id(streams
        .open(
            &request(&format!("{}/hang", service.origin), 0),
            None,
            &control,
        )
        .await
        .unwrap());
    let revoke = http.policy.clone();
    let interrupt = async move {
        sleep(Duration::from_millis(30)).await;
        revoke.revoke();
    };
    let (result, _) = tokio::join!(streams.read(&scope, &stream, 1, &control), interrupt);
    assert_eq!(
        serde_json::to_value(result.unwrap_err()).unwrap()["code"],
        "PERMISSION_DENIED"
    );
    assert_eq!(streams.active_streams(), 0);
    service.closed().await;
    service.stop().await;
}

fn test_wav() -> Vec<u8> {
    // Actual PCM WAV bytes; an SDK transport fixture, not model-generated speech.
    let mut bytes = b"RIFF".to_vec(); bytes.extend(52u32.to_le_bytes()); bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());bytes.extend(1u16.to_le_bytes());bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());bytes.extend(16000u32.to_le_bytes());bytes.extend(2u16.to_le_bytes());bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");bytes.extend(16u32.to_le_bytes());bytes.extend([0u8;16]);bytes
}
struct AuditGate { ready: Arc<Notify>, allow: Arc<Notify>, reject: bool, records: Arc<Mutex<Vec<serde_json::Value>>> }
impl bgx_adapter_v2_reference_host::before_send::BeforeSendSink for AuditGate {
    fn persist<'a>(&'a self, record: &'a serde_json::Value) -> bgx_adapter_v2_reference_host::before_send::PersistFuture<'a> {
        Box::pin(async move {
            self.records.lock().unwrap().push(record.clone());self.ready.notify_one();self.allow.notified().await;
            if self.reject { Err(serde_json::from_value(json!({"code":"RESOURCE_UNAVAILABLE","message":"fixture persistence refused"})).unwrap()) } else { Ok(()) }
        })
    }
}
#[tokio::test]
async fn before_send_ack_precedes_audio_http_and_rejection_sends_nothing() {
    let service=Service::start().await;
    for reject in [false,true] {
        let (_workspace,http)=broker(&service.origin);
        let ready=Arc::new(Notify::new());let allow=Arc::new(Notify::new());let records=Arc::new(Mutex::new(vec![]));
        let sink=Arc::new(AuditGate{ready:ready.clone(),allow:allow.clone(),reject,records:records.clone()});
        let http=Arc::new(Arc::try_unwrap(http).ok().unwrap().with_before_send(sink,"std/speech.synthesize".into()));
        let mut plan=serde_json::to_value(request(&format!("{}/audio",service.origin),3000)).unwrap();
        plan["method"]=json!("POST");plan["body"]=json!({"kind":"bytes","bytes":json!({"text":"原文，不截断"}).to_string().into_bytes()});
        plan["sink"]=json!({"kind":"media"});
        let plan=serde_json::from_value(plan).unwrap();let cloned=http.clone();let before=service.requests.load(Ordering::SeqCst);
        let job=tokio::spawn(async move {cloned.execute(&plan,None,&Control::new(Some(std::time::Instant::now()+Duration::from_secs(5)))).await});
        timeout(Duration::from_secs(3),ready.notified()).await.unwrap();
        assert_eq!(service.requests.load(Ordering::SeqCst),before);
        assert_eq!(records.lock().unwrap()[0]["text"],json!(["原文，不截断"]));
        allow.notify_one();let result=job.await.unwrap();
        if reject {assert!(result.is_err());assert_eq!(service.requests.load(Ordering::SeqCst),before);} else {
            let value=serde_json::to_value(result.unwrap()).unwrap();
            assert_eq!(value["status"],200);
            let handle=value["payload"]["handle"].as_str().unwrap().to_owned();
            let actual=http.media.run(move |b,s| b.read(s,&handle,0,1024,&Control::new(None))).await.unwrap();
            assert_eq!(actual,test_wav());assert_eq!(service.requests.load(Ordering::SeqCst),before+1);
        }
    }
    service.stop().await;
}
