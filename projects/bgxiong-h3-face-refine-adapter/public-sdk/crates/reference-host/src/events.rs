//! Ordered, byte-bounded, acknowledged events. A closed event queue, final text
//! delta, progress=1, and a successful final Response are distinct observations.
use crate::{
    media::LeaseScope,
    policy::{error, Result},
};
use bgx_adapter_v2_ports::{
    resource_policy,
    wire::{host::HostError, Invocation, Response, StreamEventV1},
};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{
    media::{validate_leased_media, MediaValidationContext},
    package::VerifiedPackage,
    usage::UsageSnapshot,
    validate_response, SchemaRegistry, ValidatedOperationInput,
};
use serde_json::Value;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::{mpsc, oneshot, watch, Mutex, Notify, OwnedSemaphorePermit, Semaphore};

struct State {
    sender: Option<mpsc::Sender<EventDelivery>>,
    last_sequence: u64,
    usage: UsageSnapshot,
    finishing: bool,
}
struct Inner {
    scope: LeaseScope,
    namespace: String,
    schema: SchemaRegistry,
    enabled: bool,
    state: Mutex<State>,
    failure: watch::Sender<Option<HostError>>,
    bytes: Arc<Semaphore>,
    budget: usize,
    pending: AtomicUsize,
    changed: Notify,
    publishers: AtomicUsize,
    closing: AtomicBool,
    final_validated: AtomicBool,
    response_budget: u64,
}
impl Inner {
    fn fail(&self, failure: HostError) {
        self.final_validated.store(false, Ordering::SeqCst);
        self.failure.send_if_modified(|current| {
            if current.is_none() {
                *current = Some(failure);
                true
            } else {
                false
            }
        });
        self.changed.notify_waiters();
    }
    async fn guard<T>(
        &self,
        control: &Control,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let mut failure = self.failure.subscribe();
        if let Some(error) = failure.borrow().clone() {
            return Err(error);
        }
        tokio::select! {biased;
            _=control.cancelled()=>Err(error("CANCELLED","Event processing cancelled; remote outcome is not confirmed")),
            _=control.expired()=>Err(error("TIMEOUT","Event processing deadline exhausted")),
            _=failure.changed()=>Err(failure.borrow().clone().unwrap_or_else(||error("INTERNAL_ERROR","Event failure signal closed"))),
            value=future=>value,
        }
    }
}
pub struct EventPublisher {
    inner: Arc<Inner>,
}
impl Clone for EventPublisher {
    fn clone(&self) -> Self {
        self.inner.publishers.fetch_add(1, Ordering::SeqCst);
        Self {
            inner: self.inner.clone(),
        }
    }
}
impl Drop for EventPublisher {
    fn drop(&mut self) {
        if self.inner.publishers.fetch_sub(1, Ordering::SeqCst) == 1
            && !self.inner.final_validated.load(Ordering::SeqCst)
        {
            self.inner.fail(error(
                "RESOURCE_UNAVAILABLE",
                "Event source ended without a validated final Response",
            ));
        }
    }
}
pub struct EventReceiver {
    inner: Arc<Inner>,
    receiver: mpsc::Receiver<EventDelivery>,
}
impl Drop for EventReceiver {
    fn drop(&mut self) {
        if !self.inner.closing.load(Ordering::SeqCst) {
            self.inner.fail(error(
                "RESOURCE_UNAVAILABLE",
                "Event consumer stopped before the final Response",
            ));
        }
    }
}
pub struct EventDelivery {
    event: StreamEventV1,
    ack: Option<oneshot::Sender<Result<()>>>,
    inner: Arc<Inner>,
    _permit: OwnedSemaphorePermit,
}
impl EventDelivery {
    pub fn event(&self) -> &StreamEventV1 {
        &self.event
    }
    pub fn acknowledge(mut self) -> Result<()> {
        let ack = self.ack.take().expect("one acknowledgement");
        ack.send(Ok(())).map_err(|_| {
            let failure = error(
                "RESOURCE_UNAVAILABLE",
                "Event producer ended before consumption acknowledgement",
            );
            self.inner.fail(failure.clone());
            failure
        })
    }
    pub fn reject(mut self, failure: HostError) {
        self.inner.fail(failure.clone());
        if let Some(ack) = self.ack.take() {
            let _ = ack.send(Err(failure));
        }
    }
}
impl Drop for EventDelivery {
    fn drop(&mut self) {
        if let Some(ack) = self.ack.take() {
            let failure = error(
                "RESOURCE_UNAVAILABLE",
                "Delivered event was dropped without acknowledgement",
            );
            self.inner.fail(failure.clone());
            let _ = ack.send(Err(failure));
        }
    }
}
struct Outstanding {
    inner: Arc<Inner>,
    finished: bool,
}
impl Drop for Outstanding {
    fn drop(&mut self) {
        if !self.finished {
            self.inner.fail(error(
                "RESOURCE_UNAVAILABLE",
                "Event emission ended before successful consumption",
            ));
        }
        self.inner.pending.fetch_sub(1, Ordering::SeqCst);
        self.inner.changed.notify_waiters();
    }
}
struct Attempt {
    inner: Arc<Inner>,
    finished: bool,
}
impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            self.inner.fail(error(
                "RESOURCE_UNAVAILABLE",
                "Event or final-response processing future was dropped",
            ));
        }
    }
}
#[derive(Debug)]
pub struct EventCompletion {
    pub response: Response,
    pub usage: Option<Value>,
    pub last_sequence: u64,
    pub stream_resumable: bool,
}

pub fn channel(
    package: &VerifiedPackage,
    scope: LeaseScope,
    negotiated_features: &std::collections::BTreeSet<String>,
) -> Result<(EventPublisher, EventReceiver)> {
    if package.identity() != &scope.package
        || scope.binding_id.is_empty()
        || scope.request_id.is_empty()
        || !negotiated_features.is_subset(package.enabled_features())
    {
        return Err(error(
            "PERMISSION_DENIED",
            "Event scope or negotiated features differ from the verified package",
        ));
    }
    let catalog: Value =
        serde_json::from_str(include_str!("../../../protocol/host-feature-catalog.json"))
            .expect("public features");
    let feature = catalog["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| {
            f["requiredImports"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "host-stream")
        })
        .ok_or_else(|| error("INTERNAL_ERROR", "Event feature is not published"))?;
    let enabled = negotiated_features.contains(feature["id"].as_str().unwrap());
    let budget = resource_policy()["limits"]["maxEventQueueBytes"]
        .as_u64()
        .unwrap() as usize;
    let (sender, receiver) = mpsc::channel(budget);
    let (failure, _) = watch::channel(None);
    let inner = Arc::new(Inner {
        scope,
        namespace: package.namespace().into(),
        schema: SchemaRegistry::public()
            .map_err(|_| error("INTERNAL_ERROR", "Public event schema unavailable"))?,
        enabled,
        state: Mutex::new(State {
            sender: Some(sender),
            last_sequence: 0,
            usage: UsageSnapshot::default(),
            finishing: false,
        }),
        failure,
        bytes: Arc::new(Semaphore::new(budget)),
        budget,
        pending: AtomicUsize::new(0),
        changed: Notify::new(),
        publishers: AtomicUsize::new(1),
        closing: AtomicBool::new(false),
        final_validated: AtomicBool::new(false),
        response_budget: package.manifest()["limits"]["outputBytes"]
            .as_u64()
            .unwrap()
            .min(
                resource_policy()["limits"]["maxFrameBytes"]
                    .as_u64()
                    .unwrap(),
            ),
    });
    Ok((
        EventPublisher {
            inner: inner.clone(),
        },
        EventReceiver { inner, receiver },
    ))
}
impl EventPublisher {
    pub fn retained_event_bytes(&self) -> usize {
        self.inner.budget - self.inner.bytes.available_permits()
    }
    pub fn final_response_validated(&self) -> bool {
        self.inner.final_validated.load(Ordering::SeqCst)
    }
    pub fn abort(&self, failure: HostError) {
        self.inner.fail(failure);
    }
    pub fn missing_response(&self) {
        self.abort(error(
            "RESOURCE_UNAVAILABLE",
            "Worker EOF is not a final Response",
        ));
    }
    pub async fn emit(
        &self,
        event: StreamEventV1,
        media: Option<&MediaValidationContext>,
        control: &Control,
    ) -> Result<()> {
        let mut attempt = Attempt {
            inner: self.inner.clone(),
            finished: false,
        };
        let result = self.emit_inner(event, media, control).await;
        if let Err(failure) = &result {
            self.inner.fail(failure.clone());
        }
        attempt.finished = true;
        result
    }
    async fn emit_inner(
        &self,
        event: StreamEventV1,
        media: Option<&MediaValidationContext>,
        control: &Control,
    ) -> Result<()> {
        if !self.inner.enabled {
            return Err(error(
                "PERMISSION_DENIED",
                "Stream events feature was not negotiated",
            ));
        }
        let value = serde_json::to_value(&event)
            .map_err(|_| error("INVALID_INPUT", "Event cannot be encoded"))?;
        self.inner
            .schema
            .validate(
                "https://bgxiong.local/vendor-adapter/v2/stream-event.schema.json",
                &value,
            )
            .map_err(|_| error("INVALID_INPUT", "Event does not satisfy the public schema"))?;
        if value["requestId"] != self.inner.scope.request_id {
            return Err(error(
                "PERMISSION_DENIED",
                "Event belongs to another request",
            ));
        }
        if value["type"] == "media_chunk" {
            let media = media.ok_or_else(|| {
                error(
                    "PERMISSION_DENIED",
                    "Media event requires live broker-owned facts",
                )
            })?;
            if media.package != self.inner.scope.package
                || media.binding_id != self.inner.scope.binding_id
                || media.request_id != self.inner.scope.request_id
            {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Event media facts belong to another request",
                ));
            }
            validate_leased_media(&value["payload"]["media"], media).map_err(|_| {
                error(
                    "PERMISSION_DENIED",
                    "Event media does not match a live owned lease",
                )
            })?;
        }
        let size = serde_json::to_vec(&value)
            .map_err(|_| error("INVALID_INPUT", "Event cannot be encoded"))?
            .len();
        if size > self.inner.budget {
            return Err(error(
                "BUDGET_EXCEEDED",
                "Event exceeds the public queue byte budget",
            ));
        }
        let mut state = self
            .inner
            .guard(control, async { Ok(self.inner.state.lock().await) })
            .await?;
        if state.finishing {
            return Err(error(
                "INVALID_INPUT",
                "Cannot emit after the final Response",
            ));
        }
        let sequence = value["sequence"]
            .as_u64()
            .ok_or_else(|| error("INVALID_INPUT", "Event sequence exceeds u64"))?;
        if sequence <= state.last_sequence {
            return Err(error(
                "INVALID_INPUT",
                "Event sequence is duplicate or out of order",
            ));
        }
        let first_sequence = self
            .inner
            .schema
            .schema("https://bgxiong.local/vendor-adapter/v2/stream-event.schema.json")
            .expect("registered event schema")["oneOf"][0]["properties"]["sequence"]["minimum"]
            .as_u64()
            .expect("public initial sequence");
        if state.last_sequence == 0 && sequence != first_sequence {
            return Err(error(
                "INVALID_INPUT",
                "The first event must start at the public initial sequence",
            ));
        }
        let permit = self
            .inner
            .guard(control, async {
                self.inner
                    .bytes
                    .clone()
                    .acquire_many_owned(size.try_into().map_err(|_| {
                        error(
                            "BUDGET_EXCEEDED",
                            "Event byte budget exceeds its supported range",
                        )
                    })?)
                    .await
                    .map_err(|_| error("RESOURCE_UNAVAILABLE", "Event byte queue was closed"))
            })
            .await?;
        if value["type"] == "usage" {
            state
                .usage
                .replace(&value["payload"], &self.inner.namespace)
                .map_err(|_| {
                    error(
                        "INVALID_INPUT",
                        "Usage event is not a valid reported cumulative snapshot",
                    )
                })?;
        }
        self.inner.pending.fetch_add(1, Ordering::SeqCst);
        let mut outstanding = Outstanding {
            inner: self.inner.clone(),
            finished: false,
        };
        let (ack, consume) = oneshot::channel();
        let delivery = EventDelivery {
            event,
            ack: Some(ack),
            inner: self.inner.clone(),
            _permit: permit,
        };
        self.inner
            .guard(control, async {
                state
                    .sender
                    .as_ref()
                    .ok_or_else(|| {
                        error(
                            "INVALID_INPUT",
                            "Final Response already closed the event queue",
                        )
                    })?
                    .send(delivery)
                    .await
                    .map_err(|_| error("RESOURCE_UNAVAILABLE", "Event consumer is unavailable"))
            })
            .await?;
        state.last_sequence = sequence;
        drop(state);
        self.inner
            .guard(control, async {
                consume
                    .await
                    .map_err(|_| error("RESOURCE_UNAVAILABLE", "Event acknowledgement was lost"))?
            })
            .await?;
        outstanding.finished = true;
        Ok(())
    }
    pub async fn finish(
        &self,
        invocation: &Invocation,
        response: &Response,
        context: Option<&ValidatedOperationInput>,
        control: &Control,
    ) -> Result<EventCompletion> {
        let mut attempt = Attempt {
            inner: self.inner.clone(),
            finished: false,
        };
        let result = self
            .finish_inner(invocation, response, context, control)
            .await;
        if let Err(failure) = &result {
            self.inner.fail(failure.clone());
        }
        attempt.finished = true;
        result
    }
    async fn finish_inner(
        &self,
        invocation: &Invocation,
        response: &Response,
        context: Option<&ValidatedOperationInput>,
        control: &Control,
    ) -> Result<EventCompletion> {
        {
            let mut state = self
                .inner
                .guard(control, async { Ok(self.inner.state.lock().await) })
                .await?;
            if state.finishing {
                return Err(error(
                    "INVALID_INPUT",
                    "Invocation already received a final Response",
                ));
            }
            state.finishing = true;
            self.inner.closing.store(true, Ordering::SeqCst);
            state.sender.take();
        }
        self.inner
            .guard(control, async {
                loop {
                    let notified = self.inner.changed.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    if self.inner.pending.load(Ordering::SeqCst) == 0 {
                        break;
                    }
                    notified.await;
                }
                Ok(())
            })
            .await?;
        if invocation.r#request_id != self.inner.scope.request_id
            || invocation.r#binding_id != self.inner.scope.binding_id
        {
            return Err(error(
                "PERMISSION_DENIED",
                "Final invocation does not match the event owner",
            ));
        }
        if serde_json::to_vec(response)
            .map_err(|_| error("INVALID_INPUT", "Final Response cannot be encoded"))?
            .len() as u64
            > self.inner.response_budget
        {
            return Err(error(
                "BUDGET_EXCEEDED",
                "Final Response exceeds its frozen byte budget",
            ));
        }
        validate_response(invocation, response, context).map_err(|_| {
            error(
                "INVALID_INPUT",
                "Final Response failed canonical operation validation",
            )
        })?;
        let value = serde_json::to_value(response)
            .map_err(|_| error("INVALID_INPUT", "Final Response cannot be encoded"))?;
        let mut state = self.inner.state.lock().await;
        if let Some(data) = value.get("dataJson") {
            let result: Value = serde_json::from_str(data.as_str().unwrap())
                .map_err(|_| error("INVALID_INPUT", "Final Response data is not JSON"))?;
            if let Some(usage) = result.get("usage") {
                state
                    .usage
                    .replace(usage, &self.inner.namespace)
                    .map_err(|_| error("INVALID_INPUT", "Final reported usage is invalid"))?;
            }
        }
        self.inner.final_validated.store(true, Ordering::SeqCst);
        Ok(EventCompletion {
            response: response.clone(),
            usage: state.usage.current().cloned(),
            last_sequence: state.last_sequence,
            stream_resumable: false,
        })
    }
}
impl EventReceiver {
    /// None means the EVENT queue closed, never success; finish() independently
    /// validates and returns the single final Response.
    pub async fn receive(&mut self, control: &Control) -> Result<Option<EventDelivery>> {
        let result = self
            .inner
            .guard(control, async { Ok(self.receiver.recv().await) })
            .await;
        if let Err(failure) = &result {
            self.inner.fail(failure.clone());
            self.receiver.close();
            while self.receiver.try_recv().is_ok() {}
        }
        result
    }
}
