use bgx_adapter_v2_ports::{
    resource_policy,
    wire::{host::HostError, Invocation, Response, StreamEventV1},
};
use bgx_adapter_v2_reference_host::{events::*, media::*};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{hash_bytes, package::*};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::time::{sleep, timeout};

fn package() -> VerifiedPackage {
    let manifest = json!({"schemaVersion":2,"pluginId":"local.events","displayName":"Event tests","version":"1.0.0","protocolRange":{"min":2,"max":2},"requiredFeatures":["stream-events-v1"],"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":hash_bytes(b"test"),"publisherId":null,"publisherKeyId":null,"requestedCapabilities":{"endpointPolicy":["staticHosts"],"networkHosts":[],"allowPrivateNetwork":false,"auth":["none"],"mediaSlots":["image"],"stateBytes":0,"embeddedHeaders":[]},"limits":{"memoryBytes":33554432,"fuel":1000,"wallMs":100,"outputBytes":1048576}});
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
    verify_package(
        &PackageContents {
            manifest_bytes: &bytes,
            hash_manifest_bytes: inventory.as_bytes(),
            signature: None,
            measured_file_hashes: &hashes,
        },
        &trust,
    )
    .unwrap()
}
fn setup() -> (EventPublisher, EventReceiver, LeaseScope) {
    let package = package();
    let scope = LeaseScope {
        package: package.identity().clone(),
        binding_id: "test".into(),
        request_id: "request".into(),
    };
    let (publisher, receiver) = channel(
        &package,
        scope.clone(),
        &BTreeSet::from(["stream-events-v1".into()]),
    )
    .unwrap();
    (publisher, receiver, scope)
}
fn event(sequence: u64, kind: &str, payload: Value) -> StreamEventV1 {
    serde_json::from_value(
        json!({"requestId":"request","sequence":sequence,"type":kind,"payload":payload}),
    )
    .unwrap()
}
fn text(sequence: u64, text: &str) -> StreamEventV1 {
    event(
        sequence,
        "text_delta",
        json!({"channel":"answer","text":text}),
    )
}
fn usage(sequence: u64, amount: &str) -> StreamEventV1 {
    event(
        sequence,
        "usage",
        json!({"measurements":[{"metric":"output_tokens","unit":"tokens","amount":amount,"source":"reported"}]}),
    )
}
fn invocation() -> Invocation {
    serde_json::from_value(json!({"protocolVersion":2,"operation":"probe","profileId":"test","bindingId":"test","capabilitySlot":"generic","requestId":"request","budgetMs":1000,"negotiatedVersion":2,"enabledFeatures":["stream-events-v1"],"payloadJson":"{\"kind\":\"package_validation\",\"configuration\":{}}"})).unwrap()
}
fn response() -> Response {
    serde_json::from_value(json!({"protocolVersion":2,"operation":"probe","dataJson":json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]}).to_string()})).unwrap()
}
fn failure() -> HostError {
    serde_json::from_value(
        json!({"code":"RESOURCE_UNAVAILABLE","message":"Synthetic event consumer failed"}),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn event_ack_order_and_usage_replacement_require_an_independent_final_response() {
    let (publisher, mut receiver, _) = setup();
    let source = publisher.clone();
    let control = Control::new(None);
    let consumer_control = control.clone();
    let consumer = tokio::spawn(async move {
        let mut sequences = vec![];
        while let Some(delivery) = receiver.receive(&consumer_control).await.unwrap() {
            sequences.push(
                serde_json::to_value(delivery.event()).unwrap()["sequence"]
                    .as_u64()
                    .unwrap(),
            );
            delivery.acknowledge().unwrap();
        }
        sequences
    });
    publisher
        .emit(text(1, "first"), None, &control)
        .await
        .unwrap();
    publisher
        .emit(
            event(
                3,
                "progress",
                json!({"fraction":1,"message":"all text arrived"}),
            ),
            None,
            &control,
        )
        .await
        .unwrap();
    assert!(!publisher.final_response_validated());
    publisher
        .emit(usage(4, "10"), None, &control)
        .await
        .unwrap();
    publisher
        .emit(usage(5, "12.5000000000000000001"), None, &control)
        .await
        .unwrap();
    assert!(!publisher.final_response_validated());
    let completion = publisher
        .finish(&invocation(), &response(), None, &control)
        .await
        .unwrap();
    assert!(publisher.final_response_validated());
    assert_eq!(completion.last_sequence, 5);
    assert_eq!(
        completion.usage.unwrap()["measurements"][0]["amount"],
        "12.5000000000000000001"
    );
    assert!(!completion.stream_resumable);
    assert_eq!(consumer.await.unwrap(), vec![1, 3, 4, 5]);
    assert_eq!(publisher.retained_event_bytes(), 0);
    assert!(source
        .finish(&invocation(), &response(), None, &control)
        .await
        .is_err());
    assert!(!source.final_response_validated());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_out_of_order_foreign_bad_payload_and_unreported_usage_are_fatal() {
    for mode in ["duplicate", "out-of-order", "foreign", "payload", "usage"] {
        let (publisher, mut receiver, _) = setup();
        let control = Control::new(None);
        for sequence in [1, 3] {
            let first = publisher.emit(text(sequence, "valid event"), None, &control);
            let consume = async {
                receiver
                    .receive(&control)
                    .await
                    .unwrap()
                    .unwrap()
                    .acknowledge()
                    .unwrap();
            };
            let (result, _) = tokio::join!(first, consume);
            result.unwrap();
        }
        let bad = match mode {
            "duplicate" => text(3, "duplicate"),
            "out-of-order" => text(2, "reordered"),
            "foreign" => {
                let mut value = serde_json::to_value(text(4, "foreign")).unwrap();
                value["requestId"] = json!("other");
                serde_json::from_value(value).unwrap()
            }
            "payload" => event(4, "progress", json!({"fraction":2,"message":null})),
            _ => event(
                4,
                "usage",
                json!({"measurements":[{"metric":"invented-metric","unit":"tokens","amount":"1","source":"reported"}]}),
            ),
        };
        assert!(
            publisher.emit(bad, None, &control).await.is_err(),
            "mode={mode}"
        );
        assert!(publisher
            .finish(&invocation(), &response(), None, &control)
            .await
            .is_err());
        assert!(receiver.receive(&control).await.is_err());
        assert!(!publisher.final_response_validated());
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn byte_budget_backpressure_and_dropped_emission_do_not_lose_events_silently() {
    let (publisher, mut receiver, _) = setup();
    let control = Control::new(None);
    let first_source = publisher.clone();
    let c = control.clone();
    let first = tokio::spawn(async move {
        first_source
            .emit(text(1, &"x".repeat(700000)), None, &c)
            .await
    });
    let delivery = receiver.receive(&control).await.unwrap().unwrap();
    assert!(publisher.retained_event_bytes() > 700000);
    let pending = publisher.emit(text(2, &"y".repeat(700000)), None, &control);
    tokio::pin!(pending);
    tokio::select! {result=&mut pending=>panic!("byte queue did not apply backpressure: {result:?}"),_=sleep(Duration::from_millis(30))=>{}}
    assert!(
        publisher.retained_event_bytes()
            <= resource_policy()["limits"]["maxEventQueueBytes"]
                .as_u64()
                .unwrap() as usize
    );
    delivery.acknowledge().unwrap();
    first.await.unwrap().unwrap();
    let receive = async {
        receiver
            .receive(&control)
            .await
            .unwrap()
            .unwrap()
            .acknowledge()
            .unwrap();
    };
    let (result, _) = tokio::join!(&mut pending, receive);
    result.unwrap();
    assert_eq!(publisher.retained_event_bytes(), 0);
    let huge = text(
        3,
        &"z".repeat(
            resource_policy()["limits"]["maxEventQueueBytes"]
                .as_u64()
                .unwrap() as usize,
        ),
    );
    assert_eq!(
        serde_json::to_value(publisher.emit(huge, None, &control).await.unwrap_err()).unwrap()
            ["code"],
        "BUDGET_EXCEEDED"
    );
    assert!(publisher
        .finish(&invocation(), &response(), None, &control)
        .await
        .is_err());
    let (source, mut receiver, _) = setup();
    {
        let waiting = source.emit(text(1, "unacknowledged"), None, &control);
        tokio::pin!(waiting);
        tokio::select! {result=&mut waiting=>panic!("unacknowledged emit finished: {result:?}"),_=sleep(Duration::from_millis(20))=>{}}
    }
    assert!(source
        .finish(&invocation(), &response(), None, &control)
        .await
        .is_err());
    assert!(receiver.receive(&control).await.is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consumer_failure_drop_eof_and_cancel_never_report_success() {
    let (source, _receiver, _) = setup();
    assert!(source
        .emit(
            text(2, "invalid initial sequence"),
            None,
            &Control::new(None)
        )
        .await
        .is_err());
    for mode in ["reject", "drop", "receiver"] {
        let (publisher, mut receiver, _) = setup();
        let c = Control::new(None);
        let source = publisher.clone();
        let owned = c.clone();
        let emit = tokio::spawn(async move { source.emit(text(1, "data"), None, &owned).await });
        let delivery = receiver.receive(&c).await.unwrap().unwrap();
        match mode {
            "reject" => delivery.reject(failure()),
            "drop" => drop(delivery),
            _ => {
                drop(receiver);
                drop(delivery);
            }
        }
        assert!(timeout(Duration::from_secs(1), emit)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(publisher
            .finish(&invocation(), &response(), None, &c)
            .await
            .is_err());
        assert!(!publisher.final_response_validated());
    }
    let (publisher, mut receiver, _) = setup();
    let c = Control::new(None);
    publisher.missing_response();
    assert!(publisher
        .finish(&invocation(), &response(), None, &c)
        .await
        .is_err());
    assert!(receiver.receive(&c).await.is_err());
    let (publisher, mut receiver, _) = setup();
    drop(publisher);
    assert!(receiver.receive(&c).await.is_err());
    let (publisher, _receiver, _) = setup();
    let cancelled = Control::new(None);
    cancelled.cancel();
    assert!(publisher
        .emit(text(1, "stopped"), None, &cancelled)
        .await
        .is_err());
    assert!(publisher
        .finish(&invocation(), &response(), None, &c)
        .await
        .is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_events_use_the_canonical_live_lease_check_and_final_waits_for_consumption() {
    let (publisher, mut receiver, scope) = setup();
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut broker = MediaBroker::new(workspace, Duration::from_secs(60), None).unwrap();
    let control = Control::new(None);
    let path = directory.path().join("input.png");
    image::RgbImage::from_pixel(2, 3, image::Rgb([1, 2, 3]))
        .save(&path)
        .unwrap();
    let leased = broker
        .import_file(&scope, "image", &path, "image", &control)
        .unwrap();
    let media = json!({"id":"image","handle":leased.handle,"kind":"image","mime":leased.stat.r#mime,"fileName":leased.file_name,"byteLength":leased.stat.r#byte_length});
    let event = event(1, "media_chunk", json!({"role":"preview","media":media}));
    let context = broker.validation_context(&scope);
    let emit = publisher.emit(event.clone(), Some(&context), &control);
    tokio::pin!(emit);
    let delivery = tokio::select! {result=&mut emit=>panic!("media event prematurely returned: {result:?}"),delivery=receiver.receive(&control)=>delivery.unwrap().unwrap()};
    let invoke = invocation();
    let final_response = response();
    let finalizing = publisher.finish(&invoke, &final_response, None, &control);
    tokio::pin!(finalizing);
    tokio::select! {result=&mut finalizing=>panic!("final response skipped outstanding event acknowledgement: {result:?}"),_=sleep(Duration::from_millis(20))=>{}}
    delivery.acknowledge().unwrap();
    let (emitted, finished) = tokio::join!(&mut emit, &mut finalizing);
    emitted.unwrap();
    finished.unwrap();
    let (publisher, _receiver, _) = setup();
    let mut foreign = scope.clone();
    foreign.request_id = "foreign".into();
    let foreign_context = broker.validation_context(&foreign);
    assert!(publisher
        .emit(event, Some(&foreign_context), &control)
        .await
        .is_err());
    broker.close().unwrap();
}
