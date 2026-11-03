use bgx_adapter_v2_ports::{
    resource_policy,
    wire::{CapabilityContractV2, OperationResultV2, OperationSubmitV2},
};
use bgx_adapter_v2_reference_host::{checkpoint::*, media::*, session::*};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{capabilities::Binding, hash_bytes, package::*, validate_input};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path, time::Duration};
use zeroize::Zeroizing;

const SECRET: &str = "test-only-raw-key-should-never-be-stored";
#[test]
fn reviewed_resolution_is_request_bound_atomic_and_never_replays_media_or_document_work() {
    use bgx_adapter_v2_reference_host::resolution::apply_operator_reviewed;
    for capability in ["video.generate", "document.convert"] {
        let directory = tempfile::tempdir().unwrap();
        let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
        let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
        let mut media = broker(&workspace);
        let package = package("1.0.0");
        let binding = binding(&package);
        let scope = scope(&package, "original");
        let mut c = serde_json::to_value(contract()).unwrap();
        c["capabilityKey"] = json!(capability);
        let c: CapabilityContractV2 = serde_json::from_value(c).unwrap();
        let mut r = serde_json::to_value(request(&c, json!([]), binding.configuration().clone())).unwrap();
        r["capabilityKey"] = json!(capability);
        let r: OperationSubmitV2 = serde_json::from_value(r).unwrap();
        let ctx = validate_input(&c, &r).unwrap();
        let mut session = Session::prepare(&mut store, &package, &binding, &scope, &ctx,
            &media.validation_context(&scope), &media, "configuration-one").unwrap();
        session.begin_submit(&mut store).unwrap();
        let original = store.read("original", "original").unwrap().unwrap();
        let mut evidence = json!({"schemaVersion":1,"requestId":"original",
            "requestDigest":original["requestDigest"],"outcome":"unresolved",
            "authority":"operator_reviewed","reviewedBy":"maintenance-operator",
            "evidence":[{"reference":"evidence/original.json","sha256":hash_bytes(b"reviewed evidence")}],
            "reason":"Available evidence is insufficient"});
        evidence["requestId"] = json!("other");
        assert!(apply_operator_reviewed(&mut store, "original", "2", &evidence).is_err());
        evidence["requestId"] = json!("original");
        evidence["outcome"] = json!("accepted");
        assert!(apply_operator_reviewed(&mut store, "original", "2", &evidence).is_err());
        evidence["outcome"] = json!("unresolved");
        let unknown = apply_operator_reviewed(&mut store, "original", "2", &evidence).unwrap();
        assert_eq!(unknown["state"], "acceptance_unknown");
        assert_eq!(unknown["revision"], "3");
        assert_eq!(apply_operator_reviewed(&mut store, "original", "2", &evidence).unwrap_err().code, "CHECKPOINT_CONFLICT");
        evidence["outcome"] = json!("rejected");
        evidence["reason"] = json!("Operator verified original request was not accepted");
        let rejected = apply_operator_reviewed(&mut store, "original", "3", &evidence).unwrap();
        assert_eq!(rejected["state"], "rejected");
        assert!(rejected["result"].is_null());
        assert!(rejected["vendorTaskId"].is_null());
        assert_eq!(session.record_result(&mut store, &ctx, &accepted(1)).unwrap_err().code, "CHECKPOINT_CONFLICT");
        drop(store);
        let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
        assert_eq!(store.read("original", "original").unwrap().unwrap(), rejected);
        let mut recovered = Session::recover(&store, "original", &package, &binding, "configuration-one").unwrap();
        assert!(recovered.begin_submit(&mut store).is_err());
        assert!(recovered.query_context(&mut media, &scope, &Control::new(None)).is_err());
        assert!(apply_operator_reviewed(&mut store, "original", "4", &evidence).is_err());
    }
}
#[test]
fn unresolved_review_does_not_prevent_a_later_valid_original_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
    let mut media = broker(&workspace);
    let (mut session, ctx) = prepare(&workspace, &mut store, &media, "original");
    session.begin_submit(&mut store).unwrap();
    let evidence = json!({"schemaVersion":1,"requestId":"original",
        "requestDigest":session.checkpoint()["requestDigest"],"outcome":"unresolved",
        "authority":"operator_reviewed","reviewedBy":"maintenance-operator",
        "evidence":[{"reference":"evidence/original.json","sha256":hash_bytes(b"review")}],
        "reason":"Not enough evidence at review time"});
    bgx_adapter_v2_reference_host::resolution::apply_operator_reviewed(&mut store, "original", "2", &evidence).unwrap();
    let package = package("1.0.0");
    let binding = binding(&package);
    let mut recovered = Session::recover(&store, "original", &package, &binding, "configuration-one").unwrap();
    recovered.record_result(&mut store, &ctx, &accepted(1)).unwrap();
    let query = recovered.query_context(&mut media, &scope(&package, "query"), &Control::new(None)).unwrap();
    recovered.record_result(&mut store, &query.context, &queried("succeeded", 2)).unwrap();
    assert_eq!(recovered.checkpoint()["state"], "terminal");
    assert_eq!(recovered.checkpoint()["resolution"], evidence);
}
#[test]
fn submission_rejection_requires_negotiation_original_context_and_exact_cas() {
    use bgx_adapter_v2_ports::wire::submission_rejection::checkpoint_candidate;
    for negotiated in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
        let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
        let media = broker(&workspace);
        let package = package_with_rejection("1.0.0", negotiated);
        let binding = binding(&package);
        let scope = scope(&package, "original");
        let c = contract();
        let r = request(&c, json!([]), binding.configuration().clone());
        let ctx = validate_input(&c, &r).unwrap();
        let mut session = Session::prepare(&mut store, &package, &binding, &scope, &ctx,
            &media.validation_context(&scope), &media, "configuration-one").unwrap();
        session.begin_submit(&mut store).unwrap();
        let evidence = json!({"phase":"input_validation","detail":"required reference invalid"});
        let candidate = checkpoint_candidate(session.checkpoint(), "before_submission", "No submission was made", &evidence).unwrap();
        if !negotiated {
            assert_eq!(session.write(&mut store,"original","2",candidate,&ctx).unwrap_err().code,"PERMISSION_DENIED");
            assert_eq!(store.read("original","original").unwrap().unwrap()["state"],"acceptance_unknown");
            continue;
        }
        let mut wrong = candidate.clone();
        wrong["submissionRejection"]["requestDigest"] = json!(hash_bytes(b"wrong"));
        assert!(session.write(&mut store,"original","2",wrong,&ctx).is_err());
        let mut changed = r.clone();
        let mut changed_value = serde_json::to_value(&changed).unwrap();
        changed_value["input"]["prompt"] = json!("different request");
        changed = serde_json::from_value(changed_value).unwrap();
        let wrong_ctx = validate_input(&c,&changed).unwrap();
        assert!(session.write(&mut store,"original","2",candidate.clone(),&wrong_ctx).is_err());
        assert_eq!(session.write(&mut store,"original","1",candidate.clone(),&ctx).unwrap_err().code,"CHECKPOINT_CONFLICT");
        assert_eq!(session.write(&mut store,"original","2",candidate.clone(),&ctx).unwrap(),"3");
        assert_eq!(store.read("original","original").unwrap().unwrap(),candidate);
        assert!(session.record_result(&mut store,&ctx,&accepted(1)).is_err());
        assert!(session.begin_submit(&mut store).is_err());
        let accepted_scope = self::scope(&package, "accepted-original");
        let mut accepted_session = Session::prepare(&mut store, &package, &binding, &accepted_scope, &ctx,
            &media.validation_context(&accepted_scope), &media, "configuration-one").unwrap();
        accepted_session.begin_submit(&mut store).unwrap();
        let mut stale_rejection = checkpoint_candidate(accepted_session.checkpoint(), "service_rejected", "Explicit service rejection", &evidence).unwrap();
        accepted_session.record_result(&mut store, &ctx, &accepted(1)).unwrap();
        stale_rejection["revision"] = json!("4");
        assert!(accepted_session.write(&mut store,"accepted-original","3",stale_rejection,&ctx).is_err());
        assert_eq!(store.read("accepted-original","accepted-original").unwrap().unwrap()["state"],"accepted");
    }
}
fn package(version: &str) -> VerifiedPackage {
    package_with_rejection(version, false)
}
fn package_with_rejection(version: &str, rejection: bool) -> VerifiedPackage {
    let features = if rejection { vec!["submission-rejection-v1"] } else { vec![] };
    let manifest = json!({"schemaVersion":2,"pluginId":"local.recovery","displayName":"Recovery tests","version":version,"protocolRange":{"min":2,"max":2},"requiredFeatures":features,"optionalFeatures":[],"language":"rust","entry":"adapter.wasm","artifactSha256":hash_bytes(b"test"),"publisherId":null,"publisherKeyId":null,"requestedCapabilities":{"endpointPolicy":["staticHosts"],"networkHosts":[],"allowPrivateNetwork":false,"auth":["none"],"mediaSlots":["image"],"stateBytes":0,"embeddedHeaders":[]},"limits":{"memoryBytes":33554432,"fuel":1000,"wallMs":100,"outputBytes":1048576}});
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let hashes = BTreeMap::from([
        ("manifest.json".into(), hash_bytes(&bytes)),
        ("adapter.wasm".into(), hash_bytes(b"test")),
    ]);
    let inventory = hashes
        .iter()
        .map(|(k, v)| format!("{v} {k}\n"))
        .collect::<String>();
    verify_package(
        &PackageContents {
            manifest_bytes: &bytes,
            hash_manifest_bytes: inventory.as_bytes(),
            signature: None,
            measured_file_hashes: &hashes,
        },
        &TrustContext { supported_features: features.into_iter().map(str::to_owned).collect(), ..TrustContext::default() },
    )
    .unwrap()
}
fn contract() -> CapabilityContractV2 {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
    let mut c = fixtures["capabilities"][1].clone();
    // A real, closed resume schema rather than an untyped continuation blob.
    let schema=json!({"type":"object","properties":{"cursor":{"type":"integer","minimum":1}},"required":["cursor"],"additionalProperties":false}).to_string();
    for s in c["schemas"].as_array_mut().unwrap() {
        if s["id"] == "resume" {
            s["schemaJson"] = json!(schema);
            s["digest"] = json!(hash_bytes(schema.as_bytes()));
        }
    }
    c["operations"][0]["resumeSchema"]["digest"] = json!(hash_bytes(schema.as_bytes()));
    serde_json::from_value(c).unwrap()
}
fn binding(package: &VerifiedPackage) -> Binding {
    Binding::new(
        "binding".into(),
        package,
        1,
        json!({"endpointBaseUrl":"https://example.invalid"}),
    )
    .unwrap()
}
fn scope(package: &VerifiedPackage, id: &str) -> LeaseScope {
    LeaseScope {
        package: package.identity().clone(),
        binding_id: "binding".into(),
        request_id: id.into(),
    }
}
fn policy() -> SnapshotPolicy {
    SnapshotPolicy::new(vec![Zeroizing::new(SECRET.as_bytes().to_vec())]).unwrap()
}
fn broker(workspace: &DevelopmentWorkspace) -> MediaBroker {
    MediaBroker::new(workspace.clone(), Duration::from_secs(60), None).unwrap()
}
fn request(c: &CapabilityContractV2, media: Value, config: Value) -> OperationSubmitV2 {
    serde_json::from_value(json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"video.generate","operationId":"example/render","operationVersion":1,"inputSchemaDigest":serde_json::to_value(c).unwrap()["operations"][0]["inputSchema"]["digest"],"input":{"prompt":"Recovery","strength":0.4},"media":media,"uploads":[],"configuration":config})).unwrap()
}
fn accepted(cursor: u64) -> OperationResultV2 {
    serde_json::from_value(json!({"kind":"accepted","vendorTaskId":"real-task-731","appliedParams":null,"resumeToken":{"cursor":cursor},"usage":null})).unwrap()
}
fn queried(status: &str, cursor: u64) -> OperationResultV2 {
    serde_json::from_value(json!({"status":status,"outputs":[],"data":if status=="succeeded" {json!({"description":"Done"})} else {Value::Null},"usage":null,"progress":null,"terminalFailure":null,"resumeToken":{"cursor":cursor}})).unwrap()
}
fn prepare(
    workspace: &DevelopmentWorkspace,
    store: &mut CheckpointStore,
    media: &MediaBroker,
    id: &str,
) -> (Session, bgx_adapter_v2_validator::ValidatedOperationInput) {
    let package = package("1.0.0");
    let binding = binding(&package);
    let scope = scope(&package, id);
    let c = contract();
    let request = request(&c, json!([]), binding.configuration().clone());
    let ctx = validate_input(&c, &request).unwrap();
    let session = Session::prepare(
        store,
        &package,
        &binding,
        &scope,
        &ctx,
        &media.validation_context(&scope),
        media,
        "configuration-one",
    )
    .unwrap();
    assert!(workspace
        .root()
        .join("checkpoint-fixtures.sqlite")
        .is_file());
    (session, ctx)
}

#[test]
#[ignore = "Invoked in a separate process by durable_recovery_across_real_processes"]
fn recovery_child() {
    let root = std::env::var_os("BGX_SDK_RECOVERY_ROOT").unwrap();
    let phase = std::env::var("BGX_SDK_RECOVERY_PHASE").unwrap();
    let workspace = DevelopmentWorkspace::open(Path::new(&root)).unwrap();
    let package = package("1.0.0");
    let binding = binding(&package);
    let mut media = broker(&workspace);
    let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
    if phase == "submit" || phase == "lost" {
        let owner = scope(&package, "original");
        let path = workspace.root().join("source.png");
        image::RgbImage::from_pixel(3, 2, image::Rgb([20u8, 30, 40]))
            .save(&path)
            .unwrap();
        let leased = media
            .import_file(&owner, "source", &path, "image", &Control::new(None))
            .unwrap();
        let c = contract();
        let request = request(
            &c,
            json!([{"id":"source","kind":"image","handle":leased.handle,"mime":leased.stat.r#mime,"fileName":leased.file_name,"byteLength":leased.stat.r#byte_length}]),
            binding.configuration().clone(),
        );
        let ctx = validate_input(&c, &request).unwrap();
        let mut session = Session::prepare(
            &mut store,
            &package,
            &binding,
            &owner,
            &ctx,
            &media.validation_context(&owner),
            &media,
            "configuration-one",
        )
        .unwrap();
        session.begin_submit(&mut store).unwrap();
        // A stand-in remote side effect counter; create_new makes a second submit fail.
        let mut counter = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(workspace.root().join("submit-count"))
            .unwrap();
        std::io::Write::write_all(&mut counter, b"1").unwrap();
        counter.sync_all().unwrap();
        if phase == "submit" {
            session
                .record_result(&mut store, &ctx, &accepted(1))
                .unwrap();
        }
        // Terminate without dropping SQLite/media/session. Recovery must use disk.
        std::process::exit(0);
    }
    let session =
        Session::recover(&store, "original", &package, &binding, "configuration-one").unwrap();
    if phase == "unknown" {
        let error = session
            .query_context(
                &mut media,
                &scope(&package, "new-query"),
                &Control::new(None),
            )
            .err()
            .unwrap();
        assert_eq!(error.code, "ACCEPTANCE_UNKNOWN");
    } else {
        let mut session = session;
        let query = session
            .query_context(
                &mut media,
                &scope(&package, "new-query"),
                &Control::new(None),
            )
            .unwrap();
        let payload = serde_json::to_value(&query.input).unwrap();
        assert_eq!(payload["vendorTaskId"], "real-task-731");
        assert_eq!(payload["resumeToken"], json!({"cursor":1}));
        assert!(payload.get("media").is_none());
        assert!(query.context.request()["media"][0]["handle"]
            .as_str()
            .is_some());
        session
            .record_result(&mut store, &query.context, &queried("running", 2))
            .unwrap();
        std::fs::write(
            workspace.root().join("query-receipt.json"),
            serde_json::to_vec(&payload).unwrap(),
        )
        .unwrap();
    }
}
fn child(workspace: &DevelopmentWorkspace, phase: &str) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "recovery_child", "--ignored", "--nocapture"])
        .env_clear()
        .env("BGX_SDK_RECOVERY_ROOT", workspace.root())
        .env("BGX_SDK_RECOVERY_PHASE", phase);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    assert!(command.status().unwrap().success());
}
#[test]
fn durable_recovery_across_real_processes_and_lost_response_never_resubmits() {
    for lost in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
        child(&workspace, if lost { "lost" } else { "submit" });
        child(&workspace, if lost { "unknown" } else { "query" });
        assert_eq!(
            std::fs::read(workspace.root().join("submit-count")).unwrap(),
            b"1"
        );
        let store = CheckpointStore::open(&workspace, policy()).unwrap();
        let cp = store.read("original", "original").unwrap().unwrap();
        assert_eq!(
            cp["state"],
            if lost {
                "acceptance_unknown"
            } else {
                "accepted"
            }
        );
        if !lost {
            assert_eq!(cp["resumeToken"], json!({"cursor":2}));
        }
        assert!(store.read("foreign", "original").is_err());
        let db = std::fs::read(workspace.root().join("checkpoint-fixtures.sqlite")).unwrap();
        assert!(!db.windows(SECRET.len()).any(|b| b == SECRET.as_bytes()));
        let inspect =
            rusqlite::Connection::open(workspace.root().join("checkpoint-fixtures.sqlite"))
                .unwrap();
        let frozen: Vec<u8> = inspect
            .query_row(
                "SELECT frozen FROM checkpoint_fixture WHERE request_id='original'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let frozen: Value = serde_json::from_slice(&frozen).unwrap();
        assert!(frozen["request"]["media"][0].get("handle").is_none());
    }
}
#[test]
fn failed_commit_duplicate_prepare_and_corrupt_snapshot_do_not_create_a_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
    let media = broker(&workspace);
    let (mut session, ctx) = prepare(&workspace, &mut store, &media, "one");
    let package = package("1.0.0");
    let binding = binding(&package);
    let scope = scope(&package, "one");
    assert_eq!(
        Session::prepare(
            &mut store,
            &package,
            &binding,
            &scope,
            &ctx,
            &media.validation_context(&scope),
            &media,
            "configuration-one"
        )
        .err()
        .unwrap()
        .code,
        "CHECKPOINT_CONFLICT"
    );
    session.begin_submit(&mut store).unwrap();
    let database =
        rusqlite::Connection::open(workspace.root().join("checkpoint-fixtures.sqlite")).unwrap();
    database.execute_batch("BEGIN IMMEDIATE;").unwrap();
    assert_eq!(
        session
            .record_result(&mut store, &ctx, &accepted(1))
            .unwrap_err()
            .code,
        "RESOURCE_UNAVAILABLE"
    );
    assert_eq!(session.checkpoint()["state"], "acceptance_unknown");
    database.execute_batch("ROLLBACK;").unwrap();
    session
        .record_result(&mut store, &ctx, &accepted(1))
        .unwrap();
    let mut cp = session.checkpoint().clone();
    cp["resumeToken"] = json!({"cursor":999});
    database
        .execute(
            "UPDATE checkpoint_fixture SET checkpoint=?1 WHERE request_id='one'",
            [serde_json::to_vec(&cp).unwrap()],
        )
        .unwrap();
    assert!(store.read("one", "one").is_err());
}

#[test]
fn asynchronous_query_does_not_require_stream_resumption_and_keeps_delivered_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
    let mut media = broker(&workspace);
    let package = package("1.0.0");
    let binding = binding(&package);
    let scope_original = scope(&package, "one");
    let mut c = serde_json::to_value(contract()).unwrap();
    c["operations"][0]["execution"]["resumable"] = json!(false);
    c["operations"][0]["outputSlots"] =
        json!([{"role":"answer","kind":"text","minCount":0,"maxCount":null}]);
    let c = serde_json::from_value(c).unwrap();
    let request = request(&c, json!([]), binding.configuration().clone());
    let ctx = validate_input(&c, &request).unwrap();
    let mut session = Session::prepare(
        &mut store,
        &package,
        &binding,
        &scope_original,
        &ctx,
        &media.validation_context(&scope_original),
        &media,
        "configuration-one",
    )
    .unwrap();
    session.begin_submit(&mut store).unwrap();
    session
        .record_result(&mut store, &ctx, &accepted(1))
        .unwrap();
    let mut recovered =
        Session::recover(&store, "one", &package, &binding, "configuration-one").unwrap();
    let query = recovered
        .query_context(&mut media, &scope(&package, "query"), &Control::new(None))
        .unwrap();
    let mut partial = serde_json::to_value(queried("running", 2)).unwrap();
    partial["outputs"] = json!([{"kind":"text","role":"answer","text":"delivered"}]);
    recovered
        .record_result(
            &mut store,
            &query.context,
            &serde_json::from_value(partial.clone()).unwrap(),
        )
        .unwrap();
    assert!(recovered
        .record_result(&mut store, &query.context, &queried("running", 3))
        .is_err());
    partial["outputs"][0]["text"] = json!("replacement");
    assert!(recovered
        .record_result(
            &mut store,
            &query.context,
            &serde_json::from_value(partial).unwrap()
        )
        .is_err());
    assert_eq!(
        recovered.checkpoint()["result"]["outputs"][0]["text"],
        "delivered"
    );
}
#[test]
fn atomic_cas_and_state_gates_preserve_real_identity_and_terminal_result() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
    let mut media = broker(&workspace);
    let (mut session, ctx) = prepare(&workspace, &mut store, &media, "one");
    assert!(session
        .record_result(&mut store, &ctx, &accepted(1))
        .is_err());
    let package = package("1.0.0");
    let binding = binding(&package);
    let mut stale =
        Session::recover(&store, "one", &package, &binding, "configuration-one").unwrap();
    session.begin_submit(&mut store).unwrap();
    assert_eq!(
        stale.begin_submit(&mut store).unwrap_err().code,
        "ACCEPTANCE_UNKNOWN"
    );
    assert!(session.begin_submit(&mut store).is_err());
    let mut next = session.checkpoint().clone();
    next["revision"] = json!("3");
    next["state"] = json!("accepted");
    next["vendorTaskId"] = json!("real-task-731");
    next["resumeToken"] = json!({"cursor":1});
    next["result"] = serde_json::to_value(accepted(1)).unwrap();
    let mut tampered = next.clone();
    tampered["package"]["version"] = json!("2.0.0");
    assert!(session
        .write(&mut store, "one", "2", tampered, &ctx)
        .is_err());
    assert!(session
        .write(&mut store, "foreign", "2", next.clone(), &ctx)
        .is_err());
    assert!(session
        .write(&mut store, "one", "1", next.clone(), &ctx)
        .is_err());
    assert_eq!(
        session.write(&mut store, "one", "2", next, &ctx).unwrap(),
        "3"
    );
    let mut stale =
        Session::recover(&store, "one", &package, &binding, "configuration-one").unwrap();
    let query = session
        .query_context(&mut media, &scope(&package, "query"), &Control::new(None))
        .unwrap();
    session
        .record_result(&mut store, &query.context, &queried("succeeded", 2))
        .unwrap();
    assert_eq!(
        stale
            .record_result(&mut store, &query.context, &queried("running", 2))
            .unwrap_err()
            .code,
        "CHECKPOINT_CONFLICT"
    );
    assert_eq!(session.checkpoint()["state"], "terminal");
    assert_eq!(session.checkpoint()["vendorTaskId"], "real-task-731");
    assert_eq!(
        session.checkpoint()["result"]["data"],
        json!({"description":"Done"})
    );
    assert!(session
        .query_context(&mut media, &scope(&package, "again"), &Control::new(None))
        .is_err());
    assert!(session
        .record_result(&mut store, &query.context, &queried("running", 3))
        .is_err());
}
#[test]
fn package_upgrade_changed_configuration_and_missing_original_fail_explicitly() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
    let media = broker(&workspace);
    let (mut session, ctx) = prepare(&workspace, &mut store, &media, "one");
    session.begin_submit(&mut store).unwrap();
    session
        .record_result(&mut store, &ctx, &accepted(1))
        .unwrap();
    let upgraded = package("2.0.0");
    let new_binding = binding(&upgraded);
    assert_eq!(
        Session::recover(&store, "one", &upgraded, &new_binding, "configuration-one")
            .err()
            .unwrap()
            .code,
        "RESOURCE_UNAVAILABLE"
    );
    let original = package("1.0.0");
    let old_binding = binding(&original);
    let changed = Binding::new(
        "binding".into(),
        &original,
        2,
        json!({"endpointBaseUrl":"https://changed.invalid"}),
    )
    .unwrap();
    assert!(Session::recover(&store, "one", &original, &changed, "configuration-one").is_err());
    assert!(Session::recover(&store, "one", &original, &old_binding, "configuration-two").is_err());
    assert!(Session::recover(
        &store,
        "missing",
        &original,
        &old_binding,
        "configuration-one"
    )
    .is_err());
    assert!(Session::recover(&store, "one", &original, &old_binding, "configuration-one").is_ok());
}
#[test]
fn secrets_invalid_tokens_oversized_state_and_schema_corruption_never_become_accepted() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
    let media = broker(&workspace);
    let package = package("1.0.0");
    let mut config = json!({"endpointBaseUrl":SECRET});
    let c = contract();
    for (id, endpoint) in [
        ("secret", SECRET.to_owned()),
        (
            "oversize",
            "x".repeat(
                resource_policy()["limits"]["maxStateBytes"]
                    .as_u64()
                    .unwrap() as usize,
            ),
        ),
    ] {
        config["endpointBaseUrl"] = json!(endpoint);
        let binding = Binding::new("binding".into(), &package, 1, config.clone()).unwrap();
        let request = request(&c, json!([]), config.clone());
        let ctx = validate_input(&c, &request).unwrap();
        let scope = scope(&package, id);
        assert!(Session::prepare(
            &mut store,
            &package,
            &binding,
            &scope,
            &ctx,
            &media.validation_context(&scope),
            &media,
            "config"
        )
        .is_err());
        assert!(store.read(id, id).unwrap().is_none());
    }
    let (mut session, ctx) = prepare(&workspace, &mut store, &media, "one");
    session.begin_submit(&mut store).unwrap();
    let mut bad = serde_json::to_value(accepted(1)).unwrap();
    bad["resumeToken"] = json!({"cursor":0});
    assert!(session
        .record_result(&mut store, &ctx, &serde_json::from_value(bad).unwrap())
        .is_err());
    let secret_result:OperationResultV2=serde_json::from_value(json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,"outputs":[],"data":{"description":SECRET},"usage":null})).unwrap();
    assert_eq!(
        session
            .record_result(&mut store, &ctx, &secret_result)
            .unwrap_err()
            .code,
        "PERMISSION_DENIED"
    );
    assert_eq!(session.checkpoint()["state"], "acceptance_unknown");
    drop(store);
    let path = workspace.root().join("checkpoint-fixtures.sqlite");
    assert!(!std::fs::read(&path)
        .unwrap()
        .windows(SECRET.len())
        .any(|b| b == SECRET.as_bytes()));
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA user_version=1;").unwrap();
    drop(db);
    assert!(CheckpointStore::open(&workspace, policy()).is_err());
}

#[test]
fn reviewed_completion_preserves_frozen_contract_and_requires_real_media() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(directory.path()).unwrap();
    let mut store = CheckpointStore::open(&workspace, policy()).unwrap();
    let mut media = broker(&workspace);
    let package = package("1.0.0");
    let binding = binding(&package);
    let owner = scope(&package, "completed-original");
    let mut declaration = serde_json::to_value(contract()).unwrap();
    declaration["operations"][0]["outputSlots"] = json!([{"role":"mask","kind":"image","minCount":1,"maxCount":1}]);
    let declaration: CapabilityContractV2 = serde_json::from_value(declaration).unwrap();
    let request = request(&declaration, json!([]), binding.configuration().clone());
    let ctx = validate_input(&declaration, &request).unwrap();
    let mut session = Session::prepare(&mut store, &package, &binding, &owner, &ctx,
        &media.validation_context(&owner), &media, "configuration-one").unwrap();
    session.begin_submit(&mut store).unwrap();
    let original = store.read("completed-original","completed-original").unwrap().unwrap();
    let frozen = || -> Vec<u8> { rusqlite::Connection::open(workspace.root().join("checkpoint-fixtures.sqlite")).unwrap()
        .query_row("SELECT frozen FROM checkpoint_fixture WHERE request_id='completed-original'",[],|r|r.get(0)).unwrap() };
    let original_frozen = frozen();
    let evidence = json!({"schemaVersion":1,"requestId":"completed-original","requestDigest":original["requestDigest"],
        "outcome":"completed","authority":"operator_reviewed","reviewedBy":"maintenance",
        "evidence":[{"reference":"original-history.json","sha256":hash_bytes(b"original history")}],"reason":"Original task completed"});
    assert!(bgx_adapter_v2_reference_host::resolution::prepare_operator_reviewed(&original,&serde_json::from_slice::<Value>(&original_frozen).unwrap(),"2",&evidence).is_err());
    let path = workspace.root().join("mask.png");
    image::RgbImage::from_pixel(3,2,image::Rgb([20u8,30,40])).save(&path).unwrap();
    let lease = media.import_file(&owner,"mask-0",&path,"image",&Control::new(None)).unwrap();
    let result = json!({"kind":"completed","vendorTaskId":"real-task-731","appliedParams":null,"data":{"description":"Done"},"usage":null,
        "outputs":[{"kind":"media","role":"mask","media":{"id":"mask-0","kind":"image","handle":lease.handle,"mime":lease.stat.r#mime,"fileName":lease.file_name,"byteLength":lease.stat.r#byte_length}}]});
    let mut forged = result.clone(); forged["outputs"][0]["media"]["handle"] = json!("not-a-lease");
    assert!(session.prepare_reviewed_completion("2",&evidence,&serde_json::from_value(forged).unwrap(),&mut media,&owner,&Control::new(None)).is_err());
    let typed = serde_json::from_value(result).unwrap();
    assert!(session.prepare_reviewed_completion("1",&evidence,&typed,&mut media,&owner,&Control::new(None)).is_err());
    let mut foreign = evidence.clone(); foreign["requestId"] = json!("other");
    assert!(session.prepare_reviewed_completion("2",&foreign,&typed,&mut media,&owner,&Control::new(None)).is_err());
    let next = session.prepare_reviewed_completion("2",&evidence,&typed,&mut media,&owner,&Control::new(None)).unwrap();
    assert_eq!(next["state"],"terminal"); assert!(next["resumeToken"].is_null());
    assert_eq!(store.read("completed-original","completed-original").unwrap().unwrap(),original);
    let mut stale = Session::recover(&store,"completed-original",&package,&binding,"configuration-one").unwrap();
    session.apply_reviewed_completion(&mut store,"2",&evidence,&typed,&mut media,&Control::new(None)).unwrap();
    assert!(stale.apply_reviewed_completion(&mut store,"2",&evidence,&typed,&mut media,&Control::new(None)).is_err());
    assert_eq!(frozen(),original_frozen);
    drop(store);
    let store = CheckpointStore::open(&workspace,policy()).unwrap();
    let recovered = Session::recover(&store,"completed-original",&package,&binding,"configuration-one").unwrap();
    assert_eq!(recovered.checkpoint(),&next);
    assert!(recovered.query_context(&mut media,&owner,&Control::new(None)).is_err());
    assert!(recovered.prepare_reviewed_completion("3",&evidence,&typed,&mut media,&owner,&Control::new(None)).is_err());
}
