//! Per-invocation local interruption and truthful remote cancellation receipts.
//! This module never changes a durable task into a terminal state, retries a
//! request, or cancels a second request. Dropping subscriptions has no effect.
use crate::{
    checkpoint::{fail, CheckpointError, Result},
    media::LeaseScope,
};
use bgx_adapter_v2_ports::wire::{Invocation, OperationTaskRefV2, Response};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{
    validate_response, validate_task_input, SchemaRegistry, ValidatedOperationInput,
};
use serde_json::{json, Value};
use std::future::Future;

/// The receipt is exactly the public cancelResult value. Query scheduling is a
/// native presentation decision derived from it, not another task state enum.
#[derive(Debug)]
pub struct CancellationReceipt {
    value: Value,
}
impl CancellationReceipt {
    pub fn value(&self) -> &Value {
        &self.value
    }
    pub fn query_required(&self) -> bool {
        self.value["outcome"] == "remote_requested"
    }
    pub fn remote_confirmed(&self) -> bool {
        self.value["outcome"] == "remote_confirmed"
    }
    pub fn from_public(value: Value) -> Result<Self> {
        SchemaRegistry::public()
            .and_then(|r| r.validate_dispatch("cancel", "result", &value))
            .map_err(|_| {
                fail(
                    "INVALID_INPUT",
                    "Cancellation receipt differs from public schema",
                )
            })?;
        Ok(Self { value })
    }
}
/// Guard a single effect. The losing future is dropped, so managed HTTP/worker
/// implementations release their own resources. No retry or remote claim.
pub async fn guarded<T, F: Future<Output = Result<T>>>(control: &Control, effect: F) -> Result<T> {
    if control.is_cancelled() {
        return Err(fail("CANCELLED", "Local request was interrupted"));
    }
    if control
        .deadline
        .is_some_and(|d| std::time::Instant::now() >= d)
    {
        return Err(fail("TIMEOUT", "Local request deadline exhausted"));
    }
    tokio::select! {
        biased;
        _=control.cancelled()=>Err(fail("CANCELLED","Local request was interrupted")),
        _=control.expired()=>Err(fail("TIMEOUT","Local request deadline exhausted")),
        result=effect=>result,
    }
}
pub struct RequestCancellation {
    scope: LeaseScope,
    control: Control,
    declaration: String,
    contract_digest: String,
    anchors: Value,
}
impl RequestCancellation {
    pub fn new(
        scope: LeaseScope,
        context: &ValidatedOperationInput,
        control: Control,
    ) -> Result<Self> {
        if scope.request_id.is_empty() || scope.binding_id.is_empty() {
            return Err(fail("INVALID_INPUT", "Cancellation scope is invalid"));
        }
        let declaration = context.operation()["execution"]["cancellation"]
            .as_str()
            .ok_or_else(|| fail("INVALID_INPUT", "Cancellation declaration is missing"))?;
        if !matches!(declaration, "remote" | "local_only" | "unsupported") {
            return Err(fail(
                "INVALID_INPUT",
                "Cancellation declaration is unsupported",
            ));
        }
        let r = context.request();
        Ok(Self {
            scope,
            control,
            declaration: declaration.into(),
            contract_digest: context.contract_digest().into(),
            anchors: json!([
                r["modelId"],
                r["capabilityKey"],
                r["operationId"],
                r["operationVersion"],
                r["configuration"]
            ]),
        })
    }
    fn authorize(&self, scope: &LeaseScope) -> Result<()> {
        if scope != &self.scope {
            return Err(fail(
                "PERMISSION_DENIED",
                "Cancellation belongs to another request",
            ));
        }
        Ok(())
    }
    /// Observers may drop this clone without cancelling work. Only explicit
    /// stop_local/request calls can request local interruption.
    pub fn control(&self) -> Control {
        self.control.clone()
    }
    pub fn stop_local(&self, scope: &LeaseScope) -> Result<CancellationReceipt> {
        self.authorize(scope)?;
        self.control.cancel();
        CancellationReceipt::from_public(json!({"outcome":"local_only"}))
    }
    /// Resolve declarations that require no remote invocation. None means the
    /// declared remote operation must run through request_remote instead.
    pub fn local_outcome(&self, scope: &LeaseScope) -> Result<Option<CancellationReceipt>> {
        self.authorize(scope)?;
        match self.declaration.as_str() {
            "local_only" => self.stop_local(scope).map(Some),
            "unsupported" => {
                CancellationReceipt::from_public(json!({"outcome":"unsupported"})).map(Some)
            }
            _ => Ok(None),
        }
    }
    /// A fresh control budgets the cancellation RPC independently from a local
    /// wait that may already have stopped. Remote errors remain errors. Only one
    /// call to effect is made, and remote_requested explicitly requires query.
    #[allow(clippy::too_many_arguments)]
    pub async fn request_remote<F, Fut>(
        &self,
        scope: &LeaseScope,
        original: &ValidatedOperationInput,
        task: &OperationTaskRefV2,
        accepted_vendor_task_id: &str,
        invocation: &Invocation,
        remote_control: &Control,
        effect: F,
    ) -> Result<CancellationReceipt>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = std::result::Result<Response, CheckpointError>>,
    {
        self.authorize(scope)?;
        if self.declaration != "remote" {
            return Err(fail(
                "UNSUPPORTED_FEATURE",
                "Operation does not declare remote cancellation",
            ));
        }
        let r = original.request();
        if self.contract_digest != original.contract_digest()
            || self.anchors
                != json!([
                    r["modelId"],
                    r["capabilityKey"],
                    r["operationId"],
                    r["operationVersion"],
                    r["configuration"]
                ])
        {
            return Err(fail("INVALID_INPUT", "Cancellation context changed"));
        }
        let context = validate_task_input(original, task, "cancel", accepted_vendor_task_id)
            .map_err(|_| {
                fail(
                    "INVALID_INPUT",
                    "Cancellation requires the original accepted task",
                )
            })?;
        let i = serde_json::to_value(invocation)
            .map_err(|_| fail("INVALID_INPUT", "Invalid cancellation invocation"))?;
        SchemaRegistry::validate_public("invocation", &i)
            .map_err(|_| fail("INVALID_INPUT", "Invalid cancellation invocation"))?;
        let payload: Value = serde_json::from_str(i["payloadJson"].as_str().unwrap())
            .map_err(|_| fail("INVALID_INPUT", "Invalid cancellation payload"))?;
        if i["operation"] != "cancel"
            || i["bindingId"] != scope.binding_id
            || i["profileId"] != scope.binding_id
            || i["requestId"] != scope.request_id
            || i["capabilitySlot"] != original.contract()["transportSlot"]
            || payload != serde_json::to_value(task).unwrap()
        {
            return Err(fail(
                "INVALID_INPUT",
                "Cancellation invocation differs from frozen task and scope",
            ));
        }
        let response = guarded(remote_control, async { effect().await }).await?;
        validate_response(invocation, &response, Some(&context))
            .map_err(|_| fail("INVALID_INPUT", "Remote cancellation response is invalid"))?;
        let response = serde_json::to_value(response).unwrap();
        if let Some(error) = response.get("error") {
            // Preserve code, never raw service diagnostics or an invented receipt.
            return Err(CheckpointError {
                code: error["code"].as_str().unwrap().into(),
                message: "Remote cancellation did not return a successful receipt",
            });
        }
        let value: Value = serde_json::from_str(response["dataJson"].as_str().unwrap())
            .map_err(|_| fail("INVALID_INPUT", "Remote cancellation data is invalid"))?;
        if !matches!(
            value["outcome"].as_str(),
            Some("remote_requested" | "remote_confirmed")
        ) {
            return Err(fail(
                "INVALID_INPUT",
                "Remote cancellation receipt contradicts its declaration",
            ));
        }
        CancellationReceipt::from_public(value)
    }
}
