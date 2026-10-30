use bgx_adapter_v2_ports::wire::{
    CapabilityContractV2, Invocation, OperationSubmitV2, OperationTaskRefV2, Response,
};
use bgx_adapter_v2_reference_host::{cancel::*, checkpoint::CheckpointError, media::LeaseScope};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{
    identity::PackageIdentity, validate_input, ValidatedOperationInput,
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::time::{sleep, timeout};
fn context(declaration: &str) -> ValidatedOperationInput {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../examples/capabilities.json")).unwrap();
    let mut contract = fixture["capabilities"][1].clone();
    contract["operations"][0]["execution"]["cancellation"] = json!(declaration);
    let request:OperationSubmitV2=serde_json::from_value(json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"video.generate","operationId":"example/render","operationVersion":1,"inputSchemaDigest":contract["operations"][0]["inputSchema"]["digest"],"input":{"prompt":"Cancellation","strength":0.5},"media":[],"uploads":[],"configuration":{"endpointBaseUrl":"https://example.invalid"}})).unwrap();
    let contract: CapabilityContractV2 = serde_json::from_value(contract).unwrap();
    validate_input(&contract, &request).unwrap()
}
fn scope(request: &str) -> LeaseScope {
    LeaseScope {
        package: PackageIdentity {
            plugin_id: "local.cancel".into(),
            version: "1.0.0".into(),
            digest: "a".repeat(64),
        },
        binding_id: "binding".into(),
        request_id: request.into(),
    }
}
fn task() -> OperationTaskRefV2 {
    serde_json::from_value(json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":"video.generate","operationId":"example/render","operationVersion":1,"vendorTaskId":"task-one","resumeToken":null,"configuration":{"endpointBaseUrl":"https://example.invalid"}})).unwrap()
}
fn invocation(task: &OperationTaskRefV2, request_id: &str) -> Invocation {
    serde_json::from_value(json!({"protocolVersion":2,"operation":"cancel","profileId":"binding","bindingId":"binding","capabilitySlot":"video","requestId":request_id,"budgetMs":1000,"negotiatedVersion":2,"enabledFeatures":[],"payloadJson":serde_json::to_string(task).unwrap()})).unwrap()
}
fn response(outcome: &str) -> Response {
    serde_json::from_value(json!({"protocolVersion":2,"operation":"cancel","dataJson":json!({"outcome":outcome}).to_string()})).unwrap()
}
struct Dropped(Arc<AtomicUsize>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn only_one_request_stops_and_observer_drop_does_not_cancel() {
    let a_scope = scope("a");
    let b_scope = scope("b");
    let ctx = context("local_only");
    let a = RequestCancellation::new(a_scope.clone(), &ctx, Control::new(None)).unwrap();
    let b = RequestCancellation::new(b_scope.clone(), &ctx, Control::new(None)).unwrap();
    drop(a.control()); // Closing/detaching an observer has no cancellation action.
    assert!(!a.control().is_cancelled());
    assert!(a.stop_local(&b_scope).is_err());
    assert!(!a.control().is_cancelled());
    let receipt = a.local_outcome(&a_scope).unwrap().unwrap();
    assert_eq!(receipt.value(), &json!({"outcome":"local_only"}));
    assert!(!receipt.remote_confirmed());
    let one = guarded(&a.control(), async {
        sleep(Duration::from_secs(10)).await;
        Ok(())
    })
    .await;
    assert_eq!(one.unwrap_err().code, "CANCELLED");
    assert!(!b.control().is_cancelled());
    guarded(&b.control(), async {
        sleep(Duration::from_millis(20)).await;
        Ok(())
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn requested_and_confirmed_are_distinct_and_only_requested_requires_query() {
    let ctx = context("remote");
    let owner = scope("cancel-one");
    let request = task();
    let invocation = invocation(&request, "cancel-one");
    let cancel = RequestCancellation::new(owner.clone(), &ctx, Control::new(None)).unwrap();
    assert!(cancel.local_outcome(&owner).unwrap().is_none());
    for outcome in ["remote_requested", "remote_confirmed"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let receipt = cancel
            .request_remote(
                &owner,
                &ctx,
                &request,
                "task-one",
                &invocation,
                &Control::new(None),
                || async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok(response(outcome))
                },
            )
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(receipt.query_required(), outcome == "remote_requested");
        assert_eq!(receipt.remote_confirmed(), outcome == "remote_confirmed");
        assert!(!cancel.control().is_cancelled());
    }
    cancel.stop_local(&owner).unwrap();
    // Stopping a local wait does not destroy accepted identity or poison a fresh cancel RPC.
    let receipt = cancel
        .request_remote(
            &owner,
            &ctx,
            &request,
            "task-one",
            &invocation,
            &Control::new(None),
            || async { Ok(response("remote_requested")) },
        )
        .await
        .unwrap();
    assert!(receipt.query_required());
    assert_eq!(
        serde_json::to_value(request).unwrap()["vendorTaskId"],
        "task-one"
    );
}
#[tokio::test]
async fn unsupported_and_invalid_context_never_dispatch_remote_effects() {
    let request = task();
    let i = invocation(&request, "one");
    let owner = scope("one");
    let calls = AtomicUsize::new(0);
    for declaration in ["unsupported", "local_only"] {
        let ctx = context(declaration);
        let cancel = RequestCancellation::new(owner.clone(), &ctx, Control::new(None)).unwrap();
        let outcome = cancel.local_outcome(&owner).unwrap().unwrap();
        assert_eq!(outcome.value()["outcome"], declaration);
        assert!(cancel
            .request_remote(
                &owner,
                &ctx,
                &request,
                "task-one",
                &i,
                &Control::new(None),
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(response("remote_confirmed"))
                }
            )
            .await
            .is_err());
    }
    let ctx = context("remote");
    let cancel = RequestCancellation::new(owner.clone(), &ctx, Control::new(None)).unwrap();
    for (bad_scope, id, bad_invocation) in [
        (scope("foreign"), "task-one", invocation(&request, "one")),
        (owner.clone(), "another-task", invocation(&request, "one")),
        (owner.clone(), "task-one", invocation(&request, "wrong")),
    ] {
        assert!(cancel
            .request_remote(
                &bad_scope,
                &ctx,
                &request,
                id,
                &bad_invocation,
                &Control::new(None),
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(response("remote_confirmed"))
                }
            )
            .await
            .is_err());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for bad in ["local_only", "unsupported"] {
        assert!(cancel
            .request_remote(
                &owner,
                &ctx,
                &request,
                "task-one",
                &i,
                &Control::new(None),
                || async { Ok(response(bad)) }
            )
            .await
            .is_err());
    }
}
#[tokio::test]
async fn network_errors_timeouts_and_dropped_rpc_release_without_false_remote_confirmation() {
    let ctx = context("remote");
    let owner = scope("one");
    let request = task();
    let i = invocation(&request, "one");
    let cancel = RequestCancellation::new(owner.clone(), &ctx, Control::new(None)).unwrap();
    let calls = AtomicUsize::new(0);
    let error = cancel
        .request_remote(
            &owner,
            &ctx,
            &request,
            "task-one",
            &i,
            &Control::new(None),
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(CheckpointError {
                    code: "UPSTREAM_ERROR".into(),
                    message: "Synthetic network failure",
                })
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "UPSTREAM_ERROR");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!cancel.control().is_cancelled());
    let remote_error:Response=serde_json::from_value(json!({"protocolVersion":2,"operation":"cancel","error":{"code":"RATE_LIMITED","message":"Untrusted detail","retryable":true,"vendorCode":null,"vendorRequestId":null}})).unwrap();
    let error = cancel
        .request_remote(
            &owner,
            &ctx,
            &request,
            "task-one",
            &i,
            &Control::new(None),
            || async { Ok(remote_error) },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "RATE_LIMITED");
    assert!(!error.message.contains("Untrusted"));
    let dropped = Arc::new(AtomicUsize::new(0));
    let observed = dropped.clone();
    let deadline = Control::new(Some(Instant::now() + Duration::from_secs(2)));
    let error = cancel
        .request_remote(
            &owner,
            &ctx,
            &request,
            "task-one",
            &i,
            &deadline,
            || async {
                let _release = Dropped(observed);
                std::future::pending().await
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "TIMEOUT");
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(!cancel.control().is_cancelled());
    let observed = dropped.clone();
    assert!(timeout(
        Duration::from_millis(500),
        cancel.request_remote(
            &owner,
            &ctx,
            &request,
            "task-one",
            &i,
            &Control::new(None),
            || async {
                let _release = Dropped(observed);
                std::future::pending().await
            }
        )
    )
    .await
    .is_err());
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
    // Even an already-expired or cancelled control must not call the side effect.
    let expired = Control::new(Some(Instant::now()));
    let cancelled = Control::new(None);
    cancelled.cancel();
    for control in [&expired, &cancelled] {
        assert!(cancel
            .request_remote(&owner, &ctx, &request, "task-one", &i, control, || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(response("remote_confirmed")) }
            })
            .await
            .is_err());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
