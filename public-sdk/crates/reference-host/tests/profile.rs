use bgx_adapter_v2_ports::wire::{
    host::HostError, Adapter, Host, HostPortCallV2, Invocation, OperationSubmitV2,
    OperationTaskRefV2, Response,
};
use bgx_adapter_v2_reference_host::{
    credential::{CredentialVault, DevelopmentSecret},
    http::{HttpBroker, MediaAccess},
    media::{DevelopmentWorkspace, LeaseScope, MediaBroker},
    policy::{DevelopmentNetworkGrant, NetworkPolicy},
    profile::Profile,
};
use bgx_adapter_v2_transport::{Control, ManagedHost, PortFuture};
use bgx_adapter_v2_validator::{
    hash_bytes,
    package::{verify_package, PackageContents, TrustContext, VerifiedPackage},
    validate_input, validate_response, validate_task_input,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

// Compile the actual public Rust author source, rather than duplicate its echo.
// This is native parity; the full WASM equivalence case remains separate.
#[path = "../../../templates/rust/src/lib.rs"]
mod rust_echo;

fn document() -> Value {
    serde_json::from_str(include_str!("../../../templates/profile/profile.json")).unwrap()
}
fn capture(name: &str, value: Value) {
    if let Some(root) = std::env::var_os("BGX_PROFILE_CAPTURE_ROOT") {
        use std::{fs::File, io::Write, path::PathBuf};
        let root = PathBuf::from(root);
        assert!(root.is_absolute() && root.is_dir());
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(root.join(name))
            .unwrap();
        file.write_all(&serde_json::to_vec_pretty(&value).unwrap())
            .unwrap();
        file.sync_all().unwrap();
    }
}
fn encoded(profile: &Value, auth: &[&str]) -> (VerifiedPackage, Vec<u8>) {
    encoded_with_wall(profile, auth, None)
}
fn encoded_with_wall(
    profile: &Value,
    auth: &[&str],
    wall_ms: Option<u64>,
) -> (VerifiedPackage, Vec<u8>) {
    let bytes = serde_json::to_vec(profile).unwrap();
    let mut manifest: Value =
        serde_json::from_str(include_str!("../../../templates/profile/manifest.json")).unwrap();
    manifest["version"] = json!(env!("CARGO_PKG_VERSION"));
    manifest["artifactSha256"] = json!(hash_bytes(&bytes));
    manifest["requestedCapabilities"]["endpointPolicy"] =
        json!(["userConfigurable", "allowPrivateNetwork"]);
    manifest["requestedCapabilities"]["networkHosts"] = json!([]);
    manifest["requestedCapabilities"]["allowPrivateNetwork"] = json!(true);
    manifest["requestedCapabilities"]["auth"] = json!(auth);
    if let Some(wall_ms) = wall_ms {
        manifest["limits"]["wallMs"] = json!(wall_ms);
    }
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let hashes = BTreeMap::from([
        ("manifest.json".into(), hash_bytes(&manifest_bytes)),
        ("profile.json".into(), hash_bytes(&bytes)),
    ]);
    let inventory = hashes
        .iter()
        .map(|(path, hash)| format!("{hash} {path}\n"))
        .collect::<String>();
    let verified = verify_package(
        &PackageContents {
            manifest_bytes: &manifest_bytes,
            hash_manifest_bytes: inventory.as_bytes(),
            signature: None,
            measured_file_hashes: &hashes,
        },
        &TrustContext::default(),
    )
    .unwrap();
    (verified, bytes)
}

#[test]
fn required_sources_resolve_root_schema_references_through_the_shared_registry() {
    let mut raw = document();
    for id in ["input", "configuration"] {
        let schema = if id == "input" {
            json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false})
        } else {
            json!({"type":"object","properties":{"endpoint":{"type":"string"}},"required":["endpoint"],"additionalProperties":false})
        };
        let source = json!({"$ref":"#/$defs/Fields","$defs":{"Fields":schema}}).to_string();
        let digest = hash_bytes(source.as_bytes());
        let entry = raw["capabilities"][0]["schemas"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["id"] == id)
            .unwrap();
        entry["schemaJson"] = json!(source);
        entry["digest"] = json!(digest);
        if id == "input" {
            raw["capabilities"][0]["operations"][0]["inputSchema"]["digest"] = json!(digest);
        } else {
            raw["capabilities"][0]["configurationSchema"]["digest"] = json!(digest);
        }
    }
    raw["operations"][0]["request"]["urlTemplate"] = json!("{{configuration:/endpoint}}/echo");
    let _ = load(&raw, &[]);

    let mut bad = raw.clone();
    bad["operations"][0]["request"]["mappings"][0]["sourcePointer"] = json!("/missing");
    let (package, bytes) = encoded(&bad, &[]);
    assert!(Profile::load(&package, &bytes).is_err());
}

struct PendingCredential {
    entered: Arc<tokio::sync::Notify>,
    calls: Vec<String>,
}
impl ManagedHost for PendingCredential {
    fn call<'a>(&'a mut self, call: HostPortCallV2, _: &'a Control) -> PortFuture<'a> {
        Box::pin(async move {
            let call = serde_json::to_value(call).unwrap();
            let method = call["method"].as_str().unwrap().to_owned();
            assert_eq!(method, "host-credential.capabilities");
            self.calls.push(method);
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}

#[tokio::test]
async fn profile_budgets_cover_credentials_and_keep_parent_cancellation() {
    let mut raw = document();
    configuration(
        &mut raw,
        json!({"credentialRef":{"type":"string"}}),
        json!(["credentialRef"]),
    );
    raw["operations"][0]["request"]["credentialMode"] = json!("bearer");
    raw["operations"][0]["request"]["credentialSlot"] = json!("/credentialRef");
    for (wall_ms, budget_ms, parent_ms) in
        [(100, 5000, None), (1000, 30, None), (1000, 5000, Some(30))]
    {
        let (package, bytes) = encoded_with_wall(&raw, &["bearer"], Some(wall_ms));
        let profile = Profile::load(&package, &bytes).unwrap();
        let mut request = submit(&raw, json!({"credentialRef":"explicit-reference"}), "text");
        request.r#budget_ms = budget_ms;
        let control = Control::new(parent_ms.map(|ms| Instant::now() + Duration::from_millis(ms)));
        let mut host = PendingCredential {
            entered: Arc::new(tokio::sync::Notify::new()),
            calls: vec![],
        };
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            profile.invoke(&request, &mut host, &control),
        )
        .await
        .expect("A profile budget must expire before the test watchdog")
        .unwrap_err();
        assert_eq!(serde_json::to_value(error).unwrap()["code"], "TIMEOUT");
        assert!(
            host.calls.len() <= 1,
            "No HTTP call may follow an expired credential wait"
        );
        assert!(host
            .calls
            .iter()
            .all(|method| method == "host-credential.capabilities"));
    }
    let profile = load(&raw, &["bearer"]);
    let request = submit(&raw, json!({"credentialRef":"explicit-reference"}), "text");
    let control = Control::new(None);
    let entered = Arc::new(tokio::sync::Notify::new());
    let mut host = PendingCredential {
        entered: entered.clone(),
        calls: vec![],
    };
    let cancel = async {
        entered.notified().await;
        control.cancel();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(profile.invoke(&request, &mut host, &control), cancel)
    })
    .await
    .expect("The derived budget must retain the parent's cancellation token");
    assert_eq!(
        serde_json::to_value(result.unwrap_err()).unwrap()["code"],
        "CANCELLED"
    );
    assert_eq!(host.calls, ["host-credential.capabilities"]);
}
fn load(profile: &Value, auth: &[&str]) -> Profile {
    let (package, bytes) = encoded(profile, auth);
    Profile::load(&package, &bytes).unwrap_or_else(|e| panic!("Profile installation failed: {e}"))
}
fn invocation(operation: &str, payload: Value) -> Invocation {
    serde_json::from_value(json!({"protocolVersion":2,"operation":operation,"profileId":"profile-binding","bindingId":"profile-binding","capabilitySlot":"generic","requestId":format!("profile-{operation}"),"budgetMs":5000,"negotiatedVersion":2,"enabledFeatures":[],"payloadJson":payload.to_string()})).unwrap()
}
fn submit(profile: &Value, configuration: Value, text: &str) -> Invocation {
    invocation(
        "submit",
        json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"local.sdk.echo/echo","operationId":"local.sdk.echo/echo","operationVersion":1,"inputSchemaDigest":profile["capabilities"][0]["operations"][0]["inputSchema"]["digest"],"input":{"text":text},"media":[],"uploads":[],"configuration":configuration}),
    )
}
fn data(response: &Response) -> Value {
    let response = serde_json::to_value(response).unwrap();
    serde_json::from_str(
        response["dataJson"]
            .as_str()
            .unwrap_or_else(|| panic!("Expected successful public response: {response}")),
    )
    .unwrap()
}
struct NoEffects;
impl Host for NoEffects {
    fn call(&mut self, _: HostPortCallV2) -> Result<Value, HostError> {
        panic!("The actual Rust echo must not invoke host ports")
    }
}

#[derive(Default)]
struct MockHost {
    calls: Vec<Value>,
    reply: Option<Value>,
    status: Option<u16>,
    modes: Vec<String>,
    pending: bool,
}
impl ManagedHost for MockHost {
    fn call<'a>(&'a mut self, call: HostPortCallV2, _: &'a Control) -> PortFuture<'a> {
        Box::pin(async move {
            let call = serde_json::to_value(call).unwrap();
            self.calls.push(call.clone());
            match call["method"].as_str().unwrap() {
                "host-credential.capabilities" => Ok(json!(self.modes)),
                "host-http.execute" => {
                    if self.pending {
                        std::future::pending::<()>().await;
                    }
                    let body = self.reply.clone().unwrap_or_else(|| {
                        let bytes: Vec<u8> =
                            serde_json::from_value(call["args"]["plan"]["body"]["bytes"].clone())
                                .unwrap();
                        serde_json::from_slice(&bytes).unwrap()
                    });
                    Ok(
                        json!({"status":self.status.unwrap_or(200),"headers":[],"payload":{"kind":"buffered","bytes":serde_json::to_vec(&body).unwrap()}}),
                    )
                }
                _ => panic!(
                    "Profile must use the existing managed ports, not a private lifecycle: {call}"
                ),
            }
        })
    }
}
fn configuration(profile: &mut Value, properties: Value, required: Value) {
    let schema=json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}).to_string();
    let digest = hash_bytes(schema.as_bytes());
    profile["capabilities"][0]["configurationSchema"]["digest"] = json!(digest);
    for resource in profile["capabilities"][0]["schemas"]
        .as_array_mut()
        .unwrap()
    {
        if resource["id"] == "configuration" {
            resource["schemaJson"] = json!(schema);
            resource["digest"] = json!(digest);
        }
    }
}

#[tokio::test]
async fn native_rust_author_and_profile_share_capabilities_input_and_result() {
    let raw = document();
    let profile = load(&raw, &[]);
    assert_eq!(profile.runtime(), "profile");
    let mut host = MockHost::default();
    let control = Control::new(None);
    let mut observations = Vec::new();
    for request in [
        invocation("capabilities", json!({})),
        invocation(
            "probe",
            json!({"schemaVersion":2,"kind":"package_validation","configuration":{}}),
        ),
        submit(&raw, json!({}), "Hello，世界 👋"),
    ] {
        let actual = profile.invoke(&request, &mut host, &control).await.unwrap();
        let expected = rust_echo::Echo::invoke(request.clone(), &mut NoEffects);
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let context = if serde_json::to_value(&request.r#operation).unwrap() == "submit" {
            let input: OperationSubmitV2 = serde_json::from_str(&request.r#payload_json).unwrap();
            Some(validate_input(&profile.capabilities()[0], &input).unwrap())
        } else {
            None
        };
        validate_response(&request, &actual, context.as_ref()).unwrap();
        observations
            .push(json!({"invocation":request,"profileResponse":actual,"codeResponse":expected}));
    }
    assert_eq!(
        host.calls.len(),
        1,
        "Capabilities and package validation must stay completely local"
    );
    assert_eq!(host.calls[0]["method"], "host-http.execute");
    let input: OperationSubmitV2 =
        serde_json::from_str(&submit(&raw, json!({}), "x").r#payload_json).unwrap();
    assert_eq!(
        input.r#input_schema_digest,
        raw["capabilities"][0]["operations"][0]["inputSchema"]["digest"]
    );
    capture(
        "native-profile-parity.json",
        json!({"schemaVersion":1,"kind":"PROFILE_NATIVE_PUBLIC_SOURCE_PARITY",
        "codeRuntime":"NATIVE_PUBLIC_RUST_SOURCE","profileRuntime":"profile","profileDocument":raw,
        "rustSourceSha256":hash_bytes(include_bytes!("../../../templates/rust/src/lib.rs")),
        "observations":observations,"profileHostCalls":host.calls,"codeHostCalls":[],
        "networkScope":"MOCK_HOST_FOR_PARITY_NOT_LIVE_HTTP"}),
    );
}

#[test]
fn installation_rejects_missing_sources_targets_types_and_unsupported_programs() {
    let original = document();
    let mut cases = Vec::new();
    let mut bad = original.clone();
    bad["operations"][0]["request"]["mappings"][0]["sourcePointer"] = json!("/absent");
    cases.push(("unknown source", bad));
    let mut bad = original.clone();
    bad["operations"][0]["request"]["mappings"][0]["sourcePointer"] = json!("/text~9");
    cases.push(("invalid pointer escape", bad));
    let mut bad = original.clone();
    bad["operations"][0]["request"]["mappings"][0]["targetPointer"] = json!("/absent");
    cases.push(("missing target", bad));
    let mut bad = original.clone();
    bad["operations"][0]["request"]["mappings"][0]["value"] = json!(5);
    cases.push(("wrong source type witness", bad));
    let mut bad = original.clone();
    let duplicate = bad["operations"][0]["request"]["mappings"][0].clone();
    bad["operations"][0]["request"]["mappings"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    cases.push(("duplicate target", bad));
    let mut bad = original.clone();
    bad["operations"][0]["response"]["mappings"]
        .as_array_mut()
        .unwrap()
        .pop();
    cases.push(("required output left null", bad));
    let mut bad = original.clone();
    bad["operations"][0]["request"]["urlTemplate"] =
        json!("https://example.invalid/{{input:/text}}");
    cases.push(("unauthorized URL source", bad));
    let mut bad = original.clone();
    bad["operations"][0]["request"]["urlTemplate"] =
        json!("https://example.invalid/{{configuration:/endpoint}");
    cases.push(("unclosed expression", bad));
    let mut bad = original.clone();
    configuration(&mut bad, json!({"endpoint":{"type":"string"}}), json!([]));
    bad["operations"][0]["request"]["urlTemplate"] = json!("{{configuration:/endpoint}}/echo");
    cases.push(("optional URL field", bad));
    let mut bad = original.clone();
    bad["capabilities"][0]["operations"][0]["execution"]["streaming"] = json!(true);
    cases.push(("streaming", bad));
    let mut bad = original.clone();
    bad["capabilities"][0]["operations"][0]["outputSlots"][0]["kind"] = json!("file");
    cases.push(("unleased media output", bad));
    let mut bad = original.clone();
    bad["operations"][0]["transportOperation"] = json!("upload");
    cases.push(("binary upload/provenance", bad));
    let mut bad = original.clone();
    bad["operations"][0]["request"]["headers"] =
        json!([{"name":"Authorization","value":"embedded-secret"}]);
    cases.push(("protected literal header", bad));
    for (label, bad) in cases {
        let (package, bytes) = encoded(&bad, &[]);
        let error = Profile::load(&package, &bytes)
            .err()
            .unwrap_or_else(|| panic!("Installation incorrectly accepted {label}"));
        assert!(!error.reason.is_empty(), "{label}");
    }
    let (package, mut bytes) = encoded(&original, &[]);
    bytes.push(b' ');
    assert_eq!(
        Profile::load(&package, &bytes).err().unwrap().path,
        "/artifactSha256"
    );
}

#[test]
fn installation_checks_declared_result_semantics_after_materializing_witnesses() {
    let mut synchronous_accepted = document();
    synchronous_accepted["operations"][0]["response"] = json!({
        "resultTemplate":{"kind":"accepted","vendorTaskId":"real-task-shape","appliedParams":null,"resumeToken":null,"usage":null},
        "mappings":[]
    });
    let mut null_data = document();
    null_data["operations"][0]["response"]["resultTemplate"]["data"] = Value::Null;
    null_data["operations"][0]["response"]["mappings"]
        .as_array_mut()
        .unwrap()
        .retain(|m| m["targetPointer"] != "/data/text");
    let mut missing_required_output = document();
    missing_required_output["operations"][0]["response"]["resultTemplate"]["outputs"] = json!([]);
    missing_required_output["operations"][0]["response"]["mappings"]
        .as_array_mut()
        .unwrap()
        .retain(|m| m["targetPointer"] != "/outputs/0/text");
    for (label, raw) in [
        ("synchronous accepted", synchronous_accepted),
        ("required data left null", null_data),
        ("declared output minimum omitted", missing_required_output),
    ] {
        let (package, bytes) = encoded(&raw, &[]);
        let error = Profile::load(&package, &bytes)
            .err()
            .unwrap_or_else(|| panic!("Invalid declared result installed: {label}"));
        assert_eq!(error.code, "INVALID_INPUT", "{label}: {error}");
    }
}

#[tokio::test]
async fn response_sources_are_not_fallbacks_and_http_errors_keep_protocol_identity() {
    let raw = document();
    let profile = load(&raw, &[]);
    let request = submit(
        &raw,
        json!({}),
        "input must not replace a missing remote field",
    );
    for remote in [json!({}), json!({"text":42})] {
        let mut host = MockHost {
            reply: Some(remote),
            ..Default::default()
        };
        let receipt = profile
            .invoke(&request, &mut host, &Control::new(None))
            .await
            .unwrap();
        let receipt = serde_json::to_value(receipt).unwrap();
        assert_eq!(receipt["error"]["code"], "UPSTREAM_ERROR");
        assert_eq!(receipt["operation"], "submit");
        assert!(receipt.get("dataJson").is_none());
        assert_eq!(host.calls.len(), 1);
    }
    let mut host = MockHost {
        status: Some(429),
        ..Default::default()
    };
    let receipt = serde_json::to_value(
        profile
            .invoke(&request, &mut host, &Control::new(None))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["error"]["code"], "UPSTREAM_ERROR");
    assert_eq!(receipt["error"]["vendorCode"], "429");
    assert_eq!(receipt["error"]["retryable"], false);
    assert_eq!(
        host.calls.len(),
        1,
        "No automatic retry or author runtime fallback"
    );
    let control = Control::new(None);
    control.cancel();
    let mut host = MockHost::default();
    assert_eq!(
        serde_json::to_value(
            profile
                .invoke(&request, &mut host, &control)
                .await
                .err()
                .unwrap()
        )
        .unwrap()["code"],
        "CANCELLED"
    );
    assert!(host.calls.is_empty());
    let mut host = MockHost {
        pending: true,
        ..Default::default()
    };
    let deadline = Control::new(Some(Instant::now() + Duration::from_secs(1)));
    assert_eq!(
        serde_json::to_value(
            profile
                .invoke(&request, &mut host, &deadline)
                .await
                .err()
                .unwrap()
        )
        .unwrap()["code"],
        "TIMEOUT"
    );
    assert_eq!(host.calls.len(), 1);
}

#[tokio::test]
async fn multiple_probe_entries_require_the_same_plan_and_routes_are_unambiguous() {
    let mut raw = document();
    let mut other = raw["capabilities"][0]["operations"][0].clone();
    other["id"] = json!("local.sdk.echo/other");
    raw["capabilities"][0]["operations"]
        .as_array_mut()
        .unwrap()
        .push(other);
    let mut submit = raw["operations"][0].clone();
    submit["operationId"] = json!("local.sdk.echo/other");
    raw["operations"].as_array_mut().unwrap().push(submit);
    let mut probe = raw["operations"][0].clone();
    probe["transportOperation"] = json!("probe");
    probe["request"]["bodyTemplate"] = json!({});
    probe["request"]["mappings"] = json!([]);
    probe["response"] = json!({"resultTemplate":{"packageValid":true,"connection":"ready","authentication":"ready","resources":"ready","issues":[]},"mappings":[]});
    raw["operations"]
        .as_array_mut()
        .unwrap()
        .push(probe.clone());
    probe["operationId"] = json!("local.sdk.echo/other");
    raw["operations"].as_array_mut().unwrap().push(probe);
    let profile = load(&raw, &[]);
    let mut host = MockHost::default();
    let response = profile
        .invoke(
            &invocation(
                "probe",
                json!({"schemaVersion":2,"kind":"connectivity","configuration":{}}),
            ),
            &mut host,
            &Control::new(None),
        )
        .await
        .unwrap();
    assert_eq!(data(&response)["connection"], "ready");
    assert_eq!(host.calls.len(), 1);
    let mut ambiguous = raw.clone();
    ambiguous["operations"][3]["request"]["urlTemplate"] =
        json!("https://example.invalid/different");
    let (package, bytes) = encoded(&ambiguous, &[]);
    assert_eq!(
        Profile::load(&package, &bytes).err().unwrap().code,
        "UNSUPPORTED_FEATURE"
    );
    let mut ambiguous = document();
    let mut duplicate = ambiguous["capabilities"][0].clone();
    duplicate["localId"] = json!("different-local-id");
    ambiguous["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    let (package, bytes) = encoded(&ambiguous, &[]);
    assert_eq!(
        Profile::load(&package, &bytes).err().unwrap().code,
        "UNSUPPORTED_FEATURE"
    );
}

#[tokio::test]
async fn url_substitution_is_single_pass_and_custom_header_marker_is_explicit() {
    let mut raw = document();
    configuration(
        &mut raw,
        json!({"path":{"type":"string"},"credentialRef":{"type":"string"}}),
        json!(["path", "credentialRef"]),
    );
    raw["operations"][0]["request"]["urlTemplate"] =
        json!("https://example.invalid/{{configuration:/path}}");
    raw["operations"][0]["request"]["credentialMode"] = json!("custom-header");
    raw["operations"][0]["request"]["credentialSlot"] = json!("/credentialRef");
    let semantics: Value =
        serde_json::from_str(include_str!("../../../protocol/profile-semantics.json")).unwrap();
    let marker = semantics["customHeader"]["placeholder"].as_str().unwrap();
    raw["operations"][0]["request"]["headers"] = json!([{"name":"X-Profile-Key","value":marker}]);
    let profile = load(&raw, &["custom-header"]);
    let request = submit(
        &raw,
        json!({"path":"{{configuration:/must-not-run}}","credentialRef":"nonsecret-test-ref"}),
        "text",
    );
    let mut host = MockHost {
        modes: vec!["custom-header".into()],
        ..Default::default()
    };
    profile
        .invoke(&request, &mut host, &Control::new(None))
        .await
        .unwrap();
    assert_eq!(host.calls[0]["method"], "host-credential.capabilities");
    let call = &host.calls[1];
    assert_eq!(call["args"]["plan"]["headers"], json!([]));
    assert_eq!(
        call["args"]["plan"]["generatedHeaders"],
        json!([{"name":"X-Profile-Key","kind":"custom-header","targetField":"X-Profile-Key"}])
    );
    assert_eq!(call["args"]["credential"], "nonsecret-test-ref");
    assert_eq!(
        call["args"]["plan"]["url"],
        "https://example.invalid/{{configuration:/must-not-run}}"
    );
    for bad_value in [format!("prefix{marker}"), String::new()] {
        let mut bad = raw.clone();
        bad["operations"][0]["request"]["headers"][0]["value"] = json!(bad_value);
        let (package, bytes) = encoded(&bad, &["custom-header"]);
        assert!(Profile::load(&package, &bytes).is_err());
    }
    let mut bad = raw.clone();
    bad["operations"][0]["request"]["credentialMode"] = json!("bearer");
    let (package, bytes) = encoded(&bad, &["bearer"]);
    assert!(Profile::load(&package, &bytes).is_err());
}

#[tokio::test]
async fn asynchronous_json_profile_is_stateless_and_uses_the_same_task_reference() {
    let raw = asynchronous_document();
    let profile = load(&raw, &[]);
    let request = submit(&raw, json!({}), "async input");
    let mut host = MockHost {
        reply: Some(json!({"job":"job-42"})),
        ..Default::default()
    };
    let accepted = profile
        .invoke(&request, &mut host, &Control::new(None))
        .await
        .unwrap();
    let input: OperationSubmitV2 = serde_json::from_str(&request.r#payload_json).unwrap();
    let context = validate_input(&profile.capabilities()[0], &input).unwrap();
    validate_response(&request, &accepted, Some(&context)).unwrap();
    assert_eq!(data(&accepted)["vendorTaskId"], "job-42");
    let task = json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"local.sdk.echo/echo","operationId":"local.sdk.echo/echo","operationVersion":1,"vendorTaskId":"job-42","resumeToken":null,"configuration":{}});
    let request = invocation("query", task.clone());
    let typed: OperationTaskRefV2 = serde_json::from_value(task).unwrap();
    let query_context = validate_task_input(&context, &typed, "query", "job-42").unwrap();
    host.reply = Some(json!({"text":"completed on original remote task"}));
    let completed = profile
        .invoke(&request, &mut host, &Control::new(None))
        .await
        .unwrap();
    validate_response(&request, &completed, Some(&query_context)).unwrap();
    let bytes: Vec<u8> =
        serde_json::from_value(host.calls[1]["args"]["plan"]["body"]["bytes"].clone()).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"job":"job-42"})
    );
    assert_eq!(
        host.calls.len(),
        2,
        "Profile owns no alternate checkpoint or resubmission state machine"
    );
}

struct BrokerHost {
    broker: HttpBroker,
    calls: Vec<Value>,
}
impl ManagedHost for BrokerHost {
    fn call<'a>(&'a mut self, call: HostPortCallV2, control: &'a Control) -> PortFuture<'a> {
        Box::pin(async move {
            let call = serde_json::to_value(call).unwrap();
            self.calls.push(call.clone());
            match call["method"].as_str().unwrap() {
                "host-http.execute" => self
                    .broker
                    .execute(
                        &serde_json::from_value(call["args"]["plan"].clone()).unwrap(),
                        call["args"]["credential"].as_str(),
                        control,
                    )
                    .await
                    .map(|r| serde_json::to_value(r).unwrap()),
                "host-credential.capabilities" => self
                    .broker
                    .credentials
                    .lock()
                    .unwrap()
                    .capabilities(
                        &self.broker.policy,
                        call["args"]["handle"].as_str().unwrap(),
                    )
                    .map(|m| json!(m)),
                _ => panic!("Unexpected managed port {call}"),
            }
        })
    }
}
async fn read_http_json(socket: &mut tokio::net::TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let (headers, body_start, length) = loop {
        let n = socket.read(&mut buffer).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            break (headers, end + 4, length);
        }
    };
    while bytes.len() - body_start < length {
        let n = socket.read(&mut buffer).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    let body: Value = serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap();
    (headers, body)
}

async fn echo_server() -> (String, tokio::task::JoinHandle<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (headers, body) = read_http_json(&mut socket).await;
        let encoded = serde_json::to_vec(&body).unwrap();
        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",encoded.len());
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.write_all(&encoded).await.unwrap();
        (headers, body)
    });
    (origin, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_loopback_http_and_vault_authentication_use_existing_brokers() {
    let semantics: Value =
        serde_json::from_str(include_str!("../../../protocol/profile-semantics.json")).unwrap();
    for (mode, header) in [
        ("bearer", "Authorization"),
        ("custom-header", "X-Profile-Key"),
    ] {
        let (origin, server) = echo_server().await;
        let mut raw = document();
        configuration(
            &mut raw,
            json!({"endpoint":{"type":"string"},"credentialRef":{"type":"string"}}),
            json!(["endpoint", "credentialRef"]),
        );
        raw["operations"][0]["request"]["urlTemplate"] = json!("{{configuration:/endpoint}}/echo");
        raw["operations"][0]["request"]["credentialMode"] = json!(mode);
        raw["operations"][0]["request"]["credentialSlot"] = json!("/credentialRef");
        if mode == "custom-header" {
            raw["operations"][0]["request"]["headers"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":header,"value":semantics["customHeader"]["placeholder"]}));
        }
        let (package, bytes) = encoded(&raw, &[mode]);
        let profile = Profile::load(&package, &bytes).unwrap();
        let workspace_dir = tempfile::tempdir().unwrap();
        let workspace = DevelopmentWorkspace::create(workspace_dir.path()).unwrap();
        let scope = LeaseScope {
            package: package.identity().clone(),
            binding_id: "profile-binding".into(),
            request_id: "profile-submit".into(),
        };
        let network = NetworkPolicy::new(
            &package,
            scope.clone(),
            DevelopmentNetworkGrant {
                origins: vec![origin.clone()],
                allow_private: true,
                allow_embedded_authorization: false,
                denied_hosts: vec![],
                denied_networks: vec![],
            },
            true,
        )
        .unwrap();
        let media = Arc::new(Mutex::new(
            MediaBroker::new(workspace, Duration::from_secs(60), None).unwrap(),
        ));
        let mut credentials = CredentialVault::new().unwrap();
        credentials
            .insert_reference(
                &network,
                "test-reference".into(),
                DevelopmentSecret {
                    bytes: b"only-the-vault-knows".to_vec().into(),
                    public_id: None,
                    custom_header: (mode == "custom-header").then(|| header.into()),
                },
                BTreeSet::from([mode.into()]),
                vec![origin.clone()],
                Duration::from_secs(60),
            )
            .unwrap();
        let broker = HttpBroker::new(
            network,
            Arc::new(Mutex::new(credentials)),
            MediaAccess {
                broker: media,
                scope,
            },
        )
        .unwrap();
        let mut host = BrokerHost {
            broker,
            calls: vec![],
        };
        let request = submit(
            &raw,
            json!({"endpoint":origin,"credentialRef":"test-reference"}),
            "Actual HTTP，真实回显",
        );
        let result = profile
            .invoke(
                &request,
                &mut host,
                &Control::new(Some(Instant::now() + Duration::from_secs(5))),
            )
            .await
            .unwrap();
        assert_eq!(data(&result)["data"]["text"], "Actual HTTP，真实回显");
        let (headers, body) = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(body, json!({"text":"Actual HTTP，真实回显"}));
        let expected = if mode == "bearer" {
            "authorization: bearer only-the-vault-knows"
        } else {
            "x-profile-key: only-the-vault-knows"
        };
        assert!(headers.to_ascii_lowercase().contains(expected));
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("only-the-vault-knows"));
        assert!(!serde_json::to_string(&host.calls)
            .unwrap()
            .contains("only-the-vault-knows"));
        assert_eq!(host.calls.len(), 2);
        capture(
            &format!("profile-http-{mode}.json"),
            json!({"schemaVersion":1,"kind":"PROFILE_ACTUAL_HTTP_BROKER",
            "runtime":"profile","invocation":request,"response":result,"profileDocument":raw,
            "hostCalls":host.calls,"service":{"requestCount":1,"body":body,"authMode":mode,"headerName":header,
            "authorizationVerified":true},"networkScope":"ACTUAL_LOOPBACK_PROFILE_ONLY_NOT_NATIVE_PARITY"}),
        );
    }
}

/// Both author runtimes pass through the same catalog, task/session, canonical
/// validation and cleanup in ReferenceHost; no worker PID is invented for JSON.
#[tokio::test]
async fn declarative_author_uses_the_common_reference_host_lifecycle() {
    let (origin, remote) = echo_server().await;
    let mut raw = document();
    raw["operations"][0]["request"]["urlTemplate"] = json!(format!("{origin}/echo"));
    let package_root = install_reference_profile(&raw);
    let workspace_parent = tempfile::tempdir().unwrap();
    let workspace =
        DevelopmentWorkspace::create(&workspace_parent.path().join("workspace")).unwrap();
    let host = reference_profile_host(package_root.path(), workspace, vec![origin]);
    let control = || Control::new(Some(Instant::now() + Duration::from_secs(5)));
    let caps = host
        .invoke(
            invocation("capabilities", json!({})),
            None,
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert!(caps.worker_pid.is_none());
    assert_eq!(
        data(&caps.response)["capabilityContracts"],
        raw["capabilities"]
    );
    let probe = host
        .invoke(
            invocation(
                "probe",
                json!({"schemaVersion":2,"kind":"package_validation","configuration":{}}),
            ),
            None,
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert!(probe.worker_pid.is_none());
    assert_eq!(data(&probe.response)["packageValid"], true);
    let selected = host.catalog().unwrap()[0].reference.clone();
    let receipt = host
        .invoke(
            submit(&raw, json!({}), "Common kernel，统一生命周期"),
            Some(selected),
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert!(receipt.worker_pid.is_none());
    assert_eq!(
        data(&receipt.response)["data"]["text"],
        "Common kernel，统一生命周期"
    );
    let (_, body) = remote.await.unwrap();
    assert_eq!(body, json!({"text":"Common kernel，统一生命周期"}));
    host.wait_idle().await.unwrap();
}

fn install_reference_profile(raw: &Value) -> tempfile::TempDir {
    use std::fs;
    let bytes = serde_json::to_vec(&raw).unwrap();
    let mut manifest: Value =
        serde_json::from_str(include_str!("../../../templates/profile/manifest.json")).unwrap();
    manifest["artifactSha256"] = json!(hash_bytes(&bytes));
    manifest["requestedCapabilities"]["endpointPolicy"] =
        json!(["userConfigurable", "allowPrivateNetwork"]);
    manifest["requestedCapabilities"]["networkHosts"] = json!([]);
    manifest["requestedCapabilities"]["allowPrivateNetwork"] = json!(true);
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let package_root = tempfile::tempdir().unwrap();
    fs::write(package_root.path().join("profile.json"), &bytes).unwrap();
    fs::write(package_root.path().join("manifest.json"), &manifest_bytes).unwrap();
    let protocol: Value = serde_json::from_str(include_str!(
        "../../../protocol/vendor-adapter-protocol.json"
    ))
    .unwrap();
    fs::write(
        package_root
            .path()
            .join(protocol["signature"]["hashManifest"].as_str().unwrap()),
        format!(
            "{} manifest.json\n{} profile.json\n",
            hash_bytes(&manifest_bytes),
            hash_bytes(&bytes)
        ),
    )
    .unwrap();
    package_root
}

fn reference_profile_host(
    package_root: &std::path::Path,
    workspace: DevelopmentWorkspace,
    origins: Vec<String>,
) -> bgx_adapter_v2_reference_host::ReferenceHost {
    use bgx_adapter_v2_reference_host::{
        config::{LoadedPackage, ReferenceConfig},
        ReferenceHost,
    };
    use bgx_adapter_v2_transport::Options;
    use bgx_adapter_v2_validator::capabilities::Binding;
    use std::path::Path;
    let package = LoadedPackage::load(package_root, &TrustContext::default()).unwrap();
    let binding = Binding::new("profile-binding".into(), package.verified(), 1, json!({})).unwrap();
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let host = ReferenceHost::new(
        ReferenceConfig {
            workspace,
            sdk_root: sdk.clone(),
            worker: sdk.join("target/debug").join(format!(
                "bgx-adapter-v2-worker{}",
                std::env::consts::EXE_SUFFIX
            )),
            transport: Options {
                handshake_timeout: Duration::from_secs(5),
                write_timeout: Duration::from_secs(5),
                shutdown_timeout: Duration::from_secs(5),
                cancel_grace: Duration::from_millis(100),
            },
            network: DevelopmentNetworkGrant {
                origins,
                allow_private: true,
                allow_embedded_authorization: false,
                denied_hosts: vec![],
                denied_networks: vec![],
            },
            credentials: vec![],
            media_ttl: Duration::from_secs(60),
            probe: None,
            configuration_ref: "profile-test-config".into(),
        },
        package,
        binding,
    )
    .unwrap();
    host
}

fn asynchronous_document() -> Value {
    let mut raw = document();
    raw["capabilities"][0]["operations"][0]["execution"]["mode"] = json!("asynchronous");
    raw["operations"][0]["response"] = json!({"resultTemplate":{"kind":"accepted","vendorTaskId":null,"appliedParams":null,"resumeToken":null,"usage":null},"mappings":[{"source":"task","sourcePointer":"/job","targetPointer":"/vendorTaskId","value":"witness-job"}]});
    let mut query = document()["operations"][0].clone();
    query["transportOperation"] = json!("query");
    query["request"]["bodyTemplate"] = json!({"job":null});
    query["request"]["mappings"] = json!([{"source":"task","sourcePointer":"/vendorTaskId","targetPointer":"/job","value":"witness-job"}]);
    query["response"]["resultTemplate"] = json!({"status":"succeeded","outputs":[{"kind":"text","role":"answer","text":null}],"data":{"text":null},"usage":null,"progress":null,"terminalFailure":null,"resumeToken":null});
    raw["operations"].as_array_mut().unwrap().push(query);
    raw
}

#[tokio::test]
async fn profile_acceptance_is_durable_before_receipt_and_query_survives_host_restart() {
    use bgx_adapter_v2_reference_host::checkpoint::{CheckpointStore, SnapshotPolicy};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let remote = tokio::spawn(async move {
        let mut requests = vec![];
        for reply in [
            json!({"job":"durable-profile-job"}),
            json!({"text":"recovered profile output"}),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.push(read_http_json(&mut socket).await.1);
            let bytes = serde_json::to_vec(&reply).unwrap();
            let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
        }
        requests
    });
    let mut raw = asynchronous_document();
    for definition in raw["operations"].as_array_mut().unwrap() {
        definition["request"]["urlTemplate"] = json!(format!("{origin}/task"));
    }
    let package = install_reference_profile(&raw);
    let parent = tempfile::tempdir().unwrap();
    let workspace = DevelopmentWorkspace::create(&parent.path().join("workspace")).unwrap();
    let host = reference_profile_host(package.path(), workspace.clone(), vec![origin.clone()]);
    let control = || Control::new(Some(Instant::now() + Duration::from_secs(5)));
    host.invoke(
        invocation("capabilities", json!({})),
        None,
        None,
        control(),
        None,
    )
    .await
    .unwrap();
    let selected = host.catalog().unwrap()[0].reference.clone();
    let accepted = host
        .invoke(
            submit(&raw, json!({}), "one original submission"),
            Some(selected),
            None,
            control(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        data(&accepted.response)["vendorTaskId"],
        "durable-profile-job"
    );
    // Read the actual shared store immediately after receipt delivery: the
    // interpreter owns no private checkpoint and invokes no alternate submit.
    {
        let store =
            CheckpointStore::open(&workspace, SnapshotPolicy::new(vec![]).unwrap()).unwrap();
        let checkpoint = store
            .read("profile-submit", "profile-submit")
            .unwrap()
            .unwrap();
        assert_eq!(checkpoint["state"], "accepted");
        assert_eq!(checkpoint["result"]["vendorTaskId"], "durable-profile-job");
    }
    host.wait_idle().await.unwrap();
    drop(host);
    let reopened = DevelopmentWorkspace::open(workspace.root()).unwrap();
    let host = reference_profile_host(package.path(), reopened, vec![origin]);
    let task = json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"local.sdk.echo/echo","operationId":"local.sdk.echo/echo","operationVersion":1,"vendorTaskId":"durable-profile-job","resumeToken":null,"configuration":{}});
    let queried = host
        .invoke(
            invocation("query", task),
            None,
            Some("profile-submit".into()),
            control(),
            None,
        )
        .await
        .unwrap();
    assert!(queried.worker_pid.is_none());
    assert_eq!(
        data(&queried.response)["data"]["text"],
        "recovered profile output"
    );
    host.wait_idle().await.unwrap();
    let requests = tokio::time::timeout(Duration::from_secs(2), remote)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        requests,
        [
            json!({"text":"one original submission"}),
            json!({"job":"durable-profile-job"})
        ]
    );
}
