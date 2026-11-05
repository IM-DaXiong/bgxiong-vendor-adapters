//! Six-operation development host using the SAME validator/ports/transport.
//! Runtime execution remains in the explicit SDK worker process.
use crate::{
    cancel::CancellationReceipt,
    checkpoint::{
        fail, CheckpointError, CheckpointRows, CheckpointStore, Result, SnapshotPolicy,
        SqliteFixtureRows,
    },
    config::{self, LoadedPackage, ReferenceConfig},
    credential::{CredentialVault, DevelopmentSecret},
    events::{self, EventCompletion, EventPublisher},
    http::{HttpBroker, MediaAccess},
    http_stream::HttpStreams,
    media::{LeaseScope, LeasedMedia, MediaBroker},
    policy::{self, NetworkPolicy},
    session::Session,
};
use bgx_adapter_v2_ports::{
    decimal_u64, resource_policy,
    wire::{
        host::HostError, CapabilityContractV2, HostPortCallV2, Invocation, OperationResultV2,
        OperationSubmitV2, Response, StreamEventV1,
    },
};
use bgx_adapter_v2_transport::{Control, FaultKind, ManagedHost, PortFuture, WorkerClient};
use bgx_adapter_v2_validator::{
    capabilities::{Binding, CapabilityRef, CapabilityRegistry, CapabilityView},
    hash_bytes,
    media::{validate_host_media, validate_leased_media},
    validate_response, validate_task_input, SchemaRegistry, ValidatedOperationInput,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    future::Future,
    io::Write,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Notify;
use zeroize::Zeroizing;

fn host_error(e: CheckpointError) -> HostError {
    // CAS conflict is an invalid expectedRevision on the public host port. The
    // native API keeps its precise code; no new wire error enum is introduced.
    policy::error(
        match e.code.as_str() {
            "PERMISSION_DENIED" => "PERMISSION_DENIED",
            "RESOURCE_UNAVAILABLE" => "RESOURCE_UNAVAILABLE",
            "BUDGET_EXCEEDED" => "BUDGET_EXCEEDED",
            "TIMEOUT" => "TIMEOUT",
            "CANCELLED" => "CANCELLED",
            "INTERNAL_ERROR" => "INTERNAL_ERROR",
            _ => "INVALID_INPUT",
        },
        e.message,
    )
}
fn native(e: HostError) -> CheckpointError {
    CheckpointError {
        code: serde_json::to_value(e.r#code)
            .unwrap()
            .as_str()
            .unwrap()
            .into(),
        message: "Managed host operation failed; inspect the development trace",
    }
}
fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| fail("INTERNAL_ERROR", "Development host lock failed"))
}
fn cv(e: bgx_adapter_v2_validator::ContractError) -> CheckpointError {
    CheckpointError {
        code: e.code.into(),
        message: "Public contract validation failed",
    }
}
fn check_control(control: &Control) -> Result<()> {
    if control.is_cancelled() {
        return Err(fail("CANCELLED", "Local request was interrupted"));
    }
    if control
        .deadline
        .is_some_and(|at| std::time::Instant::now() >= at)
    {
        return Err(fail("TIMEOUT", "Local request deadline exhausted"));
    }
    Ok(())
}
fn encode<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("wire serialization")
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|_| {
        fail(
            "INTERNAL_ERROR",
            "Development host blocking operation failed",
        )
    })?
}
pub type EventFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<(), HostError>> + Send + 'a>>;
pub trait EventSink: Send + Sync {
    fn consume<'a>(&'a self, event: &'a StreamEventV1) -> EventFuture<'a>;
}
/// Optional development auditing. Observers cannot replace ports or change wire
/// results; an observation failure aborts through the existing cleanup path.
pub type PortObservationFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<(), HostError>> + Send + 'a>>;
pub trait PortObserver: Send + Sync {
    fn started<'a>(&'a self, invocation: &'a Invocation) -> PortObservationFuture<'a>;
    fn completed_port<'a>(
        &'a self,
        invocation: &'a Invocation,
        call: &'a HostPortCallV2,
        result: &'a std::result::Result<Value, HostError>,
    ) -> PortObservationFuture<'a>;
    fn finished<'a>(
        &'a self,
        invocation: &'a Invocation,
        response: &'a Response,
    ) -> PortObservationFuture<'a>;
}
async fn observe(
    future: PortObservationFuture<'_>,
    control: &Control,
) -> std::result::Result<(), HostError> {
    tokio::select! {
        biased;
        _=control.cancelled()=>Err(policy::error("CANCELLED","Development observation was interrupted")),
        _=control.expired()=>Err(policy::error("TIMEOUT","Development observation deadline exhausted")),
        result=future=>result,
    }
}
pub struct DispatchReceipt {
    pub response: Response,
    pub worker_pid: Option<u32>,
    pub events: EventCompletion,
    pub cancellation: Option<CancellationReceipt>,
}
struct Inner {
    config: ReferenceConfig,
    package: LoadedPackage,
    binding: Binding,
    catalog: Mutex<CapabilityRegistry>,
    profile: Option<crate::profile::Profile>,
    media: Arc<Mutex<MediaBroker>>,
    checkpoints: Arc<Mutex<CheckpointStore>>,
    snapshot: Arc<SnapshotPolicy>,
    state: Arc<Mutex<BTreeMap<String, String>>>,
}
struct Active {
    requests: Mutex<BTreeSet<String>>,
    cleanup_failures: Mutex<Option<HostError>>,
    changed: Notify,
}
#[derive(Clone)]
pub struct ReferenceHost {
    inner: Arc<Inner>,
    client: WorkerClient,
    active: Arc<Active>,
    before_send: Option<Arc<dyn crate::before_send::BeforeSendSink>>,
}
struct Prepared {
    context: Option<Arc<ValidatedOperationInput>>,
    session: Option<Arc<Mutex<Session>>>,
    root_id: Option<String>,
}
impl ReferenceHost {
    /// Development host: checkpoints live in the marked workspace fixture.
    pub fn new(config: ReferenceConfig, package: LoadedPackage, binding: Binding) -> Result<Self> {
        let rows = Box::new(SqliteFixtureRows::open(&config.workspace)?);
        Self::with_checkpoints(config, package, binding, rows)
    }
    /// Embedding host: the same session rules persist through the host's own durable rows.
    pub fn with_checkpoints(
        config: ReferenceConfig,
        package: LoadedPackage,
        binding: Binding,
        rows: Box<dyn CheckpointRows>,
    ) -> Result<Self> {
        if binding.package() != package.verified().identity() || config.configuration_ref.is_empty()
        {
            return Err(fail(
                "INVALID_INPUT",
                "Explicit binding/configuration reference differs from package",
            ));
        }
        let profile = if package.verified().manifest()["language"] == "profile" {
            Some(
                crate::profile::Profile::load(package.verified(), &package.profile_bytes()?)
                    .map_err(cv)?,
            )
        } else {
            None
        };
        let snapshot = config.snapshot_policy()?;
        snapshot.check(binding.configuration())?;
        let client = WorkerClient::new(&config.sdk_root, &config.worker, config.transport.clone())
            .map_err(|_| {
                fail(
                    "INVALID_INPUT",
                    "Explicit SDK worker configuration is invalid",
                )
            })?;
        let media = MediaBroker::new(
            config.workspace.clone(),
            config.media_ttl,
            config.probe.clone(),
        )
        .map_err(native)?;
        let checkpoints = CheckpointStore::with_rows(rows, config.snapshot_policy()?);
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                package,
                profile,
                binding,
                catalog: Mutex::new(CapabilityRegistry::default()),
                media: Arc::new(Mutex::new(media)),
                checkpoints: Arc::new(Mutex::new(checkpoints)),
                snapshot: Arc::new(snapshot),
                state: Arc::new(Mutex::new(BTreeMap::new())),
            }),
            client,
            before_send: None,
            active: Arc::new(Active {
                requests: Mutex::new(BTreeSet::new()),
                cleanup_failures: Mutex::new(None),
                changed: Notify::new(),
            }),
        })
    }
    pub fn with_before_send(mut self, sink: Arc<dyn crate::before_send::BeforeSendSink>) -> Self {
        self.before_send = Some(sink);
        self
    }
    pub fn catalog(&self) -> Result<Vec<CapabilityView>> {
        Ok(lock(&self.inner.catalog)?.list(None))
    }
    pub fn scope(&self, request_id: &str) -> LeaseScope {
        LeaseScope {
            package: self.inner.package.verified().identity().clone(),
            binding_id: self.inner.binding.id().into(),
            request_id: request_id.into(),
        }
    }
    pub async fn import_media(
        &self,
        request_id: &str,
        id: &str,
        path: &std::path::Path,
        kind: &str,
        control: Control,
    ) -> Result<LeasedMedia> {
        let broker = self.inner.media.clone();
        let scope = self.scope(request_id);
        {
            let mut active = lock(&self.active.requests)?;
            if !active.insert(scope.request_id.clone()) {
                return Err(fail(
                    "INVALID_INPUT",
                    "Cannot import into an already active request",
                ));
            }
        }
        let cleanup = Arc::new(Cleanup::new(
            self.inner.clone(),
            self.active.clone(),
            scope.clone(),
            control.clone(),
            None,
        ));
        let mut lease = CleanupLease(Some(cleanup.clone()));
        let id = id.to_owned();
        let path = path.to_owned();
        let kind = kind.to_owned();
        let imported = blocking(move || {
            lock(&broker)?
                .import_file(&scope, &id, &path, &kind, &control)
                .map_err(native)
        })
        .await;
        match imported {
            Ok(media) => {
                lock(&self.active.requests)?.remove(request_id);
                self.active.changed.notify_waiters();
                lease.0 = None;
                Ok(media)
            }
            Err(error) => {
                let cleaned = cleanup.finish().await;
                lease.0 = None;
                cleaned.map_err(native)?;
                Err(error)
            }
        }
    }
    pub async fn wait_idle(&self) -> Result<()> {
        self.client.wait_idle().await;
        loop {
            let notified = self.active.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let empty = lock(&self.active.requests)?.is_empty();
            if empty {
                self.client.wait_idle().await;
                let mut failures = lock(&self.active.cleanup_failures)?;
                return match failures.take() {
                    Some(e) => Err(native(e)),
                    None => Ok(()),
                };
            }
            notified.await;
        }
    }
    /// Caller supplies a fresh request ID and the exact selected catalog reference.
    /// Query/cancel additionally select the ORIGINAL root checkpoint explicitly.
    pub async fn invoke(
        &self,
        invocation: Invocation,
        reference: Option<CapabilityRef>,
        checkpoint_request: Option<String>,
        control: Control,
        sink: Option<Arc<dyn EventSink>>,
    ) -> Result<DispatchReceipt> {
        self.invoke_observed(
            invocation,
            reference,
            checkpoint_request,
            control,
            sink,
            None,
        )
        .await
    }
    pub async fn invoke_observed(
        &self,
        invocation: Invocation,
        reference: Option<CapabilityRef>,
        checkpoint_request: Option<String>,
        control: Control,
        sink: Option<Arc<dyn EventSink>>,
        observer: Option<Arc<dyn PortObserver>>,
    ) -> Result<DispatchReceipt> {
        if self.inner.profile.is_some() && observer.is_some() {
            return Err(fail("UNSUPPORTED_FEATURE", "Profile recording requires an interpreter replay implementation; no WASM transcript was created"));
        }
        let i = encode(&invocation);
        SchemaRegistry::validate_public("invocation", &i).map_err(cv)?;
        let scope = self.scope(i["requestId"].as_str().unwrap());
        let (publisher, mut receiver) = events::channel(
            self.inner.package.verified(),
            scope.clone(),
            self.inner.package.verified().enabled_features(),
        )
        .map_err(native)?;
        {
            let mut active = lock(&self.active.requests)?;
            if !active.insert(scope.request_id.clone()) {
                return Err(fail("INVALID_INPUT", "Request ID is already active"));
            }
        }
        let cleanup = Arc::new(Cleanup::new(
            self.inner.clone(),
            self.active.clone(),
            scope.clone(),
            control.clone(),
            Some(publisher.clone()),
        ));
        let mut lease = CleanupLease(Some(cleanup.clone()));
        let result=async{
            let inner=self.inner.clone();let selected=reference.clone();let request=invocation.clone();let scope_copy=scope.clone();let c=control.clone();
            let prepared=blocking(move||inner.prepare(&request,selected.as_ref(),checkpoint_request.as_deref(),&scope_copy,&c)).await?;
            let enabled=!(i["operation"]=="capabilities"||(i["operation"]=="probe"&&serde_json::from_str::<Value>(i["payloadJson"].as_str().unwrap()).map_err(|_|fail("INVALID_INPUT","Invalid probe payload"))?["kind"]=="package_validation"));
            let network=NetworkPolicy::new(self.inner.package.verified(),scope.clone(),self.inner.config.network.clone(),enabled).map_err(native)?;
            let mut vault=CredentialVault::new().map_err(native)?;
            if enabled {for credential in &self.inner.config.credentials{
                vault.insert_reference(&network,credential.reference.clone(),DevelopmentSecret{bytes:Zeroizing::new(credential.secret.bytes.to_vec()),public_id:credential.secret.public_id.clone(),custom_header:credential.secret.custom_header.clone()},credential.modes.clone(),credential.origins.clone(),credential.ttl).map_err(native)?;
            }}
            let vault=Arc::new(Mutex::new(vault));cleanup.resources(&network,&vault)?;
            let http = HttpBroker::new(network, vault.clone(), MediaAccess{broker:self.inner.media.clone(),scope:scope.clone()}).map_err(native)?;
            let http = if let Some(sink) = &self.before_send {
                let operation = prepared.context.as_ref().and_then(|c| c.operation()["id"].as_str()).unwrap_or(i["operation"].as_str().unwrap()).to_owned();
                http.with_before_send(sink.clone(), operation)
            } else { http };
            let http = Arc::new(http);
            let inner=self.inner.clone();let owner=scope.clone();let trace=blocking(move||Trace::new(&inner,&owner)).await?;
            trace.emit(json!({"stage":"dispatch","operation":i["operation"],"requestId":scope.request_id}),self.inner.snapshot.clone()).await?;
            let kernel=Kernel{inner:self.inner.clone(),scope:scope.clone(),http:http.clone(),streams:HttpStreams::new(http,self.inner.package.verified().enabled_features().clone()),publisher:publisher.clone(),context:prepared.context.clone(),session:prepared.session.clone(),root_id:prepared.root_id.clone(),trace:trace.clone()};
            let identity=serde_json::from_value(encode(self.inner.package.verified().identity())).map_err(|_|fail("INTERNAL_ERROR","Cannot encode verified package identity"))?;
            let worker=async{
                let local_cancel=if i["operation"]=="cancel"{prepared.context.as_ref().map(|c|c.operation()["execution"]["cancellation"].clone()).filter(|v|v!="remote")}else{None};
                if local_cancel.is_none(){if let Some(observer)=&observer{if let Err(error)=observe(observer.started(&invocation),&control).await{publisher.abort(error.clone());return Err(native(error));}}}
                if i["operation"]=="submit"{let s=prepared.session.as_ref().unwrap().clone();let store=self.inner.checkpoints.clone();let c=control.clone();blocking(move||{check_control(&c)?;lock(&s)?.begin_submit(&mut *lock(&store)?)}).await?;}
                let (worker_pid,response)=if let Some(outcome)=local_cancel{
                    (None,serde_json::from_value(json!({"protocolVersion":2,"operation":"cancel","dataJson":json!({"outcome":outcome}).to_string()})).map_err(|_|fail("INTERNAL_ERROR","Cannot encode cancellation receipt"))?)
                }else if let Some(profile)=&self.inner.profile {
                    let mut managed=kernel;
                    let response=match profile.invoke(&invocation,&mut managed,&control).await {
                        Ok(response)=>response,
                        Err(error)=>{publisher.abort(error.clone());return Err(if i["operation"]=="submit"{fail("ACCEPTANCE_UNKNOWN","Profile execution failed after submission preparation; inspect the durable checkpoint before any further action")}else{native(error)});}
                    };
                    (None,response)
                }else{let observed=ObservedKernel{kernel,invocation:invocation.clone(),observer:observer.clone()};match self.client.invoke(self.inner.package.root.clone(),identity,invocation.clone(),Box::new(observed),control.clone()).await{
                    Ok(receipt)=>(Some(receipt.worker_pid),receipt.response),
                    Err(fault)=>{publisher.abort(policy::error("RESOURCE_UNAVAILABLE","SDK worker did not return a valid response"));trace.emit(json!({"stage":"worker_error","kind":format!("{:?}",fault.kind),"dispatched":fault.invocation_dispatched}),self.inner.snapshot.clone()).await?;
                        return Err(fail(if i["operation"]=="submit" {"ACCEPTANCE_UNKNOWN"} else {match fault.kind{FaultKind::Deadline=>"TIMEOUT",FaultKind::Cancelled=>"CANCELLED",_=>"RESOURCE_UNAVAILABLE"}},"Worker execution failed; no remote resubmission was attempted"));}
                }};
                let events=publisher.finish(&invocation,&response,prepared.context.as_deref(),&control).await.map_err(native)?;
                Ok((worker_pid,response,events))
            };
            let consumer=async{
                loop{
                    let Some(delivery)=receiver.receive(&control).await.map_err(native)? else{return Ok::<(),CheckpointError>(());};
                    let consumed=match &sink{
                        Some(sink)=>tokio::select!{biased;_=control.cancelled()=>Err(policy::error("CANCELLED","Event consumption was interrupted")),_=control.expired()=>Err(policy::error("TIMEOUT","Event consumption deadline exhausted")),r=sink.consume(delivery.event())=>r},
                        None=>Err(policy::error("RESOURCE_UNAVAILABLE","Streaming event requires an explicit consumer")),
                    };
                    match consumed{Ok(())=>delivery.acknowledge().map_err(native)?,Err(e)=>{delivery.reject(e.clone());control.cancel();return Err(native(e));}}
                }
            };
            let (worker,consumer)=tokio::join!(worker,consumer);
            let (worker_pid,response,events)=worker?;consumer?;
            let inner=self.inner.clone();let final_response=response.clone();let invoke=invocation.clone();let root=prepared.root_id.clone();let ctx=prepared.context.clone();let session=prepared.session.clone();let scope_copy=scope.clone();
            let cancellation=blocking(move||inner.finish(&invoke,&final_response,reference.as_ref(),root.as_deref(),ctx.as_deref(),session.as_ref(),&scope_copy)).await?;
            if worker_pid.is_some(){if let Some(observer)=&observer{observe(observer.finished(&invocation,&response),&control).await.map_err(native)?;}}
            trace.emit(json!({"stage":"validated_receipt","operation":i["operation"],"workerPid":worker_pid}),self.inner.snapshot.clone()).await?;
            Ok(DispatchReceipt{response,worker_pid,events,cancellation})
        }.await;
        let cleaned = cleanup.finish().await;
        lease.0 = None;
        match (result, cleaned) {
            (Ok(receipt), Ok(())) => Ok(receipt),
            (Err(e), Ok(())) => Err(e),
            (_, Err(e)) => Err(native(e)),
        }
    }
}
impl Inner {
    fn prepare(
        &self,
        invocation: &Invocation,
        reference: Option<&CapabilityRef>,
        root_id: Option<&str>,
        scope: &LeaseScope,
        control: &Control,
    ) -> Result<Prepared> {
        check_control(control)?;
        let i = encode(invocation);
        let op = i["operation"].as_str().unwrap();
        if root_id.is_some() && !matches!(op, "query" | "cancel") {
            return Err(fail(
                "INVALID_INPUT",
                "Only query/cancel may select an existing checkpoint",
            ));
        }
        if reference.is_some() && matches!(op, "query" | "cancel") {
            return Err(fail("INVALID_INPUT","Query/cancel use the frozen checkpoint rather than a refreshed capability selection"));
        }
        if i["profileId"] != self.binding.id()
            || i["bindingId"] != self.binding.id()
            || i["budgetMs"] == 0
        {
            return Err(fail(
                "INVALID_INPUT",
                "Invocation binding or guest budget is invalid",
            ));
        }
        let features: BTreeSet<_> = i["enabledFeatures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        if features != *self.package.verified().enabled_features() {
            return Err(fail(
                "UNSUPPORTED_FEATURE",
                "Invocation must use the exact negotiated feature set",
            ));
        }
        let payload: Value = serde_json::from_str(i["payloadJson"].as_str().unwrap())
            .map_err(|_| fail("INVALID_INPUT", "Payload is not JSON"))?;
        SchemaRegistry::public()
            .and_then(|r| r.validate_dispatch(op, "input", &payload))
            .map_err(cv)?;
        self.snapshot.check(&payload)?;
        let mut prepared = Prepared {
            context: None,
            session: None,
            root_id: None,
        };
        match op {
            "capabilities" => {
                if reference.is_some() || root_id.is_some() {
                    return Err(fail(
                        "INVALID_INPUT",
                        "Discovery cannot select an existing task",
                    ));
                }
            }
            "probe" => {
                if payload["kind"] == "connectivity"
                    && payload["configuration"] != *self.binding.configuration()
                {
                    return Err(fail(
                        "INVALID_INPUT",
                        "Probe configuration differs from selected binding",
                    ));
                }
                if payload["kind"] == "connectivity" {
                    let catalog = lock(&self.catalog)?;
                    let valid = if let Some(reference) = reference {
                        bgx_adapter_v2_validator::validate_configuration(
                            catalog.contract_for(reference, &self.binding).map_err(cv)?,
                            self.binding.configuration(),
                        )
                        .is_ok()
                    } else {
                        catalog.list(None).iter().any(|view| {
                            catalog
                                .contract_for(&view.reference, &self.binding)
                                .is_ok_and(|contract| {
                                    bgx_adapter_v2_validator::validate_configuration(
                                        contract,
                                        self.binding.configuration(),
                                    )
                                    .is_ok()
                                })
                        })
                    };
                    if !valid {
                        return Err(fail("INVALID_INPUT","Connectivity probe configuration must match a discovered configuration schema"));
                    }
                }
            }
            "submit" => {
                if root_id.is_some() {
                    return Err(fail(
                        "INVALID_INPUT",
                        "Submit cannot select an existing checkpoint",
                    ));
                }
                let selected = reference.ok_or_else(|| {
                    fail(
                        "INVALID_INPUT",
                        "Submit requires an explicit capability reference",
                    )
                })?;
                let request: OperationSubmitV2 = serde_json::from_value(payload.clone())
                    .map_err(|_| fail("INVALID_INPUT", "Submit wire input is invalid"))?;
                let context = lock(&self.catalog)?
                    .validate_submission(selected, &self.binding, invocation, &request)
                    .map_err(cv)?;
                let media = lock(&self.media)?;
                let mut facts = media.validation_context(scope);
                let config_digest =
                    hash_bytes(&serde_json::to_vec(self.binding.configuration()).unwrap());
                for u in payload["uploads"].as_array().unwrap() {
                    let r = u["uploadRef"].as_str().unwrap();
                    facts.uploads_by_ref.insert(
                        r.into(),
                        config::read_upload(
                            &self.config.workspace,
                            scope,
                            &config_digest,
                            r,
                            &self.snapshot,
                        )?,
                    );
                }
                validate_host_media(&context, &facts).map_err(cv)?;
                let mut store = lock(&self.checkpoints)?;
                let session = Session::prepare(
                    &mut store,
                    self.package.verified(),
                    &self.binding,
                    scope,
                    &context,
                    &facts,
                    &media,
                    &self.config.configuration_ref,
                )?;
                prepared.context = Some(Arc::new(context));
                prepared.session = Some(Arc::new(Mutex::new(session)));
                prepared.root_id = Some(scope.request_id.clone());
            }
            "query" | "cancel" => {
                let root = root_id.ok_or_else(|| {
                    fail(
                        "INVALID_INPUT",
                        "Query/cancel requires the original checkpoint identity",
                    )
                })?;
                let session = Session::recover(
                    &*lock(&self.checkpoints)?,
                    root,
                    self.package.verified(),
                    &self.binding,
                    &self.config.configuration_ref,
                )?;
                let query = session.query_context(&mut *lock(&self.media)?, scope, control)?;
                if payload != encode(&query.input) {
                    return Err(fail(
                        "INVALID_INPUT",
                        "Task payload differs from the frozen original identity/token",
                    ));
                }
                if i["capabilitySlot"] != query.context.contract()["transportSlot"] {
                    return Err(fail("INVALID_INPUT", "Task transport slot changed"));
                }
                let context = if op == "cancel" {
                    validate_task_input(
                        &query.context,
                        &query.input,
                        "cancel",
                        payload["vendorTaskId"].as_str().unwrap(),
                    )
                    .map_err(cv)?
                } else {
                    query.context
                };
                prepared.context = Some(Arc::new(context));
                prepared.session = Some(Arc::new(Mutex::new(session)));
                prepared.root_id = Some(root.into());
            }
            "upload" => {
                let selected = reference.ok_or_else(|| {
                    fail(
                        "INVALID_INPUT",
                        "Upload requires an explicit capability reference",
                    )
                })?;
                let c = encode(
                    lock(&self.catalog)?
                        .contract_for(selected, &self.binding)
                        .map_err(cv)?,
                );
                let contract: CapabilityContractV2 = serde_json::from_value(c.clone())
                    .map_err(|_| fail("INVALID_PACKAGE", "Frozen upload contract is invalid"))?;
                bgx_adapter_v2_validator::validate_configuration(
                    &contract,
                    self.binding.configuration(),
                )
                .map_err(cv)?;
                let operation = c["operations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|op| {
                        op["id"] == selected.operation_id
                            && op["version"] == selected.operation_version
                    })
                    .unwrap();
                if payload["modelId"] != selected.model_id
                    || payload["capabilityKey"] != selected.capability_key
                    || payload["operationId"] != selected.operation_id
                    || payload["operationVersion"] != selected.operation_version
                    || payload["configuration"] != *self.binding.configuration()
                    || i["capabilitySlot"] != c["transportSlot"]
                {
                    return Err(fail(
                        "INVALID_INPUT",
                        "Upload differs from selected binding/capability",
                    ));
                }
                if !operation["mediaSlots"].as_array().unwrap().iter().any(|s| {
                    s["role"] == payload["role"] && s["mediaKind"] == payload["media"]["kind"]
                }) {
                    return Err(fail("INVALID_INPUT", "Upload role/kind was not declared"));
                }
                validate_leased_media(
                    &payload["media"],
                    &lock(&self.media)?.validation_context(scope),
                )
                .map_err(cv)?;
            }
            _ => return Err(fail("UNSUPPORTED_FEATURE", "Unknown transport operation")),
        }
        Ok(prepared)
    }
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        invocation: &Invocation,
        response: &Response,
        reference: Option<&CapabilityRef>,
        _root: Option<&str>,
        context: Option<&ValidatedOperationInput>,
        session: Option<&Arc<Mutex<Session>>>,
        scope: &LeaseScope,
    ) -> Result<Option<CancellationReceipt>> {
        validate_response(invocation, response, context).map_err(cv)?;
        let r = encode(response);
        self.snapshot.check(&r)?;
        if r.get("error").is_some() {
            return Ok(None);
        }
        let data: Value = serde_json::from_str(r["dataJson"].as_str().unwrap())
            .map_err(|_| fail("INVALID_INPUT", "Result JSON is invalid"))?;
        self.snapshot.check(&data)?;
        let i = encode(invocation);
        let op = i["operation"].as_str().unwrap();
        match op {
            "capabilities" => {
                if data["pluginId"] != self.package.verified().identity().plugin_id
                    || data["pluginVersion"] != self.package.verified().identity().version
                {
                    return Err(fail(
                        "INVALID_PACKAGE",
                        "Runtime capability identity differs from measured package",
                    ));
                }
                let contracts: Vec<CapabilityContractV2> =
                    serde_json::from_value(data["capabilityContracts"].clone())
                        .map_err(|_| fail("INVALID_PACKAGE", "Capability contracts are invalid"))?;
                lock(&self.catalog)?
                    .refresh(self.package.verified(), &self.binding, &contracts)
                    .map_err(cv)?;
            }
            "probe" => {
                if let Some(reference) = reference {
                    lock(&self.catalog)?
                        .set_readiness(reference, data)
                        .map_err(cv)?;
                }
            }
            "upload" => {
                let payload: Value =
                    serde_json::from_str(i["payloadJson"].as_str().unwrap()).unwrap();
                if data["mediaId"] != payload["media"]["id"] {
                    return Err(fail(
                        "INVALID_INPUT",
                        "Upload receipt refers to a different media item",
                    ));
                }
                config::save_upload(
                    &self.config.workspace,
                    scope,
                    &hash_bytes(&serde_json::to_vec(self.binding.configuration()).unwrap()),
                    data["mediaId"].as_str().unwrap(),
                    &lock(&self.media)?
                        .identity(scope, payload["media"]["handle"].as_str().unwrap())
                        .map_err(native)?
                        .r#digest,
                    data["uploadRef"].as_str().unwrap(),
                    &self.snapshot,
                )?;
            }
            "submit" | "query" => {
                let context = context
                    .ok_or_else(|| fail("INTERNAL_ERROR", "Frozen operation context missing"))?;
                let media = lock(&self.media)?;
                let result: OperationResultV2 = serde_json::from_value(data.clone())
                    .map_err(|_| fail("INVALID_INPUT", "Operation result is invalid"))?;
                let mut session = lock(
                    session.ok_or_else(|| fail("INTERNAL_ERROR", "Request checkpoint missing"))?,
                )?;
                // A guest may have already persisted exactly this accepted receipt.
                if session.checkpoint()["result"] != data {
                    session.record_result_with_media(
                        &mut *lock(&self.checkpoints)?,
                        context,
                        &result,
                        &media,
                        scope,
                    )?;
                }
            }
            "cancel" => {
                let declaration = &context
                    .ok_or_else(|| fail("INTERNAL_ERROR", "Frozen cancellation context missing"))?
                    .operation()["execution"]["cancellation"];
                let outcome = &data["outcome"];
                if !((declaration == "remote"
                    && (outcome == "remote_requested" || outcome == "remote_confirmed"))
                    || (declaration == "local_only" && outcome == "local_only")
                    || (declaration == "unsupported" && outcome == "unsupported"))
                {
                    return Err(fail(
                        "INVALID_INPUT",
                        "Cancel outcome contradicts the frozen declaration",
                    ));
                }
                return CancellationReceipt::from_public(data).map(Some);
            }
            _ => {}
        }
        Ok(None)
    }
}

// One cleanup claim, shared by explicit completion and dropped-call cleanup.
struct Cleanup {
    inner: Arc<Inner>,
    active: Arc<Active>,
    scope: LeaseScope,
    control: Control,
    publisher: Option<EventPublisher>,
    resources: Mutex<Option<(NetworkPolicy, Arc<Mutex<CredentialVault>>)>>,
    started: AtomicBool,
    result: Mutex<Option<std::result::Result<(), HostError>>>,
    done: Notify,
}
impl Cleanup {
    fn new(
        inner: Arc<Inner>,
        active: Arc<Active>,
        scope: LeaseScope,
        control: Control,
        publisher: Option<EventPublisher>,
    ) -> Self {
        Self {
            inner,
            active,
            scope,
            control,
            publisher,
            resources: Mutex::new(None),
            started: AtomicBool::new(false),
            result: Mutex::new(None),
            done: Notify::new(),
        }
    }
    fn resources(&self, p: &NetworkPolicy, v: &Arc<Mutex<CredentialVault>>) -> Result<()> {
        *lock(&self.resources)? = Some((p.clone(), v.clone()));
        Ok(())
    }
    fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        self.control.cancel();
        if let Some(publisher) = self
            .publisher
            .as_ref()
            .filter(|p| !p.final_response_validated())
        {
            publisher.abort(policy::error(
                "CANCELLED",
                "Invocation ended without a validated final receipt",
            ));
        }
        let owner = self.clone();
        tokio::spawn(async move {
            let work = owner.clone();
            let result = tokio::task::spawn_blocking(move || {
                let media = work
                    .inner
                    .media
                    .lock()
                    .map_err(|_| policy::error("INTERNAL_ERROR", "Media cleanup lock failed"))?
                    .release_request(&work.scope);
                let credentials = match work.resources.lock() {
                    Ok(resources) => match &*resources {
                        Some((policy, vault)) => {
                            policy.revoke();
                            vault
                                .lock()
                                .map_err(|_| {
                                    policy::error(
                                        "INTERNAL_ERROR",
                                        "Credential cleanup lock failed",
                                    )
                                })?
                                .release_request(&work.scope);
                            Ok(())
                        }
                        None => Ok(()),
                    },
                    Err(_) => Err(policy::error(
                        "INTERNAL_ERROR",
                        "Cleanup resource lock failed",
                    )),
                };
                media.and(credentials)
            })
            .await
            .unwrap_or_else(|_| Err(policy::error("INTERNAL_ERROR", "Request cleanup failed")));
            if let Err(error) = &result {
                match owner.active.cleanup_failures.lock() {
                    Ok(mut failure) => {
                        failure.get_or_insert_with(|| error.clone());
                    }
                    Err(_) => eprintln!("SDK cleanup failure delivery lock failed"),
                };
            }
            match owner.result.lock() {
                Ok(mut saved) => *saved = Some(result),
                Err(_) => eprintln!("SDK cleanup result lock failed"),
            };
            match owner.active.requests.lock() {
                Ok(mut active) => {
                    active.remove(&owner.scope.request_id);
                }
                Err(_) => eprintln!("SDK cleanup active request lock failed"),
            };
            owner.done.notify_waiters();
            owner.active.changed.notify_waiters();
        });
    }
    async fn finish(self: &Arc<Self>) -> std::result::Result<(), HostError> {
        self.start();
        loop {
            let notified = self.done.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(r) = self
                .result
                .lock()
                .map_err(|_| policy::error("INTERNAL_ERROR", "Cleanup result lock failed"))?
                .clone()
            {
                return r;
            }
            notified.await;
        }
    }
}
struct CleanupLease(Option<Arc<Cleanup>>);
impl Drop for CleanupLease {
    fn drop(&mut self) {
        if let Some(cleanup) = self.0.take() {
            cleanup.start();
        }
    }
}
struct Trace {
    file: Mutex<(File, usize)>,
}
impl Trace {
    async fn emit(self: &Arc<Self>, value: Value, policy: Arc<SnapshotPolicy>) -> Result<()> {
        let trace = self.clone();
        blocking(move || trace.write(&value, &policy)).await
    }
    fn new(inner: &Inner, scope: &LeaseScope) -> Result<Arc<Self>> {
        let directory = inner.config.workspace.root().join("traces");
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                return Err(fail(
                    "RESOURCE_UNAVAILABLE",
                    "Trace directory is unavailable",
                ))
            }
        }
        if directory
            .canonicalize()
            .ok()
            .as_deref()
            .and_then(std::path::Path::parent)
            != Some(inner.config.workspace.root())
        {
            return Err(fail(
                "PERMISSION_DENIED",
                "Trace directory escapes development workspace",
            ));
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(format!("{}.jsonl", hash_bytes(scope.request_id.as_bytes()))))
            .map_err(|_| {
                fail(
                    "RESOURCE_UNAVAILABLE",
                    "Trace already exists or cannot be created; use a fresh invocation request ID",
                )
            })?;
        Ok(Arc::new(Self {
            file: Mutex::new((file, 0)),
        }))
    }
    fn write(&self, value: &Value, policy: &SnapshotPolicy) -> Result<()> {
        policy.check(value)?;
        let mut bytes = serde_json::to_vec(value).unwrap();
        bytes.push(b'\n');
        let mut file = lock(&self.file)?;
        if file.1.saturating_add(bytes.len())
            > resource_policy()["limits"]["maxStateBytes"]
                .as_u64()
                .unwrap() as usize
        {
            return Err(fail(
                "BUDGET_EXCEEDED",
                "Request trace exceeds public state byte budget",
            ));
        }
        file.0
            .write_all(&bytes)
            .and_then(|_| file.0.sync_data())
            .map_err(|_| fail("RESOURCE_UNAVAILABLE", "Request trace cannot be written"))?;
        file.1 += bytes.len();
        Ok(())
    }
}
struct ObservedKernel {
    kernel: Kernel,
    invocation: Invocation,
    observer: Option<Arc<dyn PortObserver>>,
}
impl ManagedHost for ObservedKernel {
    fn call<'a>(&'a mut self, call: HostPortCallV2, control: &'a Control) -> PortFuture<'a> {
        Box::pin(async move {
            match &self.observer {
                None => self.kernel.call(call, control).await,
                Some(observer) => {
                    let result = self.kernel.call(call.clone(), control).await;
                    observe(
                        observer.completed_port(&self.invocation, &call, &result),
                        control,
                    )
                    .await?;
                    result
                }
            }
        })
    }
}
struct Kernel {
    inner: Arc<Inner>,
    scope: LeaseScope,
    http: Arc<HttpBroker>,
    streams: HttpStreams,
    publisher: EventPublisher,
    context: Option<Arc<ValidatedOperationInput>>,
    session: Option<Arc<Mutex<Session>>>,
    root_id: Option<String>,
    trace: Arc<Trace>,
}
impl ManagedHost for Kernel {
    fn call<'a>(&'a mut self, call: HostPortCallV2, control: &'a Control) -> PortFuture<'a> {
        Box::pin(async move {
            let call = encode(&call);
            SchemaRegistry::validate_public("host-ports", &call)
                .map_err(cv)
                .map_err(host_error)?;
            let args = &call["args"];
            let method = call["method"].as_str().unwrap();
            let access = self.http.media.clone();
            let a = args.clone();
            let c = control.clone();
            let result = match method {
                "host-http.execute" => encode(
                    &self
                        .http
                        .execute(
                            &serde_json::from_value(args["plan"].clone())
                                .map_err(|_| policy::error("INVALID_INPUT", "Invalid HTTP plan"))?,
                            args["credential"].as_str(),
                            control,
                        )
                        .await?,
                ),
                "host-http-stream.open" => encode(
                    &self
                        .streams
                        .open(
                            &serde_json::from_value(args["plan"].clone()).map_err(|_| {
                                policy::error("INVALID_INPUT", "Invalid stream plan")
                            })?,
                            args["credential"].as_str(),
                            control,
                        )
                        .await?,
                ),
                "host-http-stream.read" => encode(
                    &self
                        .streams
                        .read(
                            &self.scope,
                            args["streamId"].as_str().unwrap(),
                            args["maxBytes"].as_u64().unwrap() as usize,
                            control,
                        )
                        .await?,
                ),
                "host-http-stream.close" => {
                    self.streams
                        .close(&self.scope, args["streamId"].as_str().unwrap())?;
                    Value::Null
                }
                "host-media.info" => {
                    access
                        .run(move |b, s| Ok(encode(&b.info(s, a["handle"].as_str().unwrap())?)))
                        .await?
                }
                "host-media.read" => {
                    access
                        .run(move |b, s| {
                            Ok(encode(&b.read(
                                s,
                                a["handle"].as_str().unwrap(),
                                decimal_u64(a["offset"].as_str().unwrap()).map_err(|_| {
                                    policy::error("INVALID_INPUT", "Media offset exceeds u64")
                                })?,
                                a["maxBytes"].as_u64().unwrap() as usize,
                                &c,
                            )?))
                        })
                        .await?
                }
                "host-media.create" => {
                    access
                        .run(move |b, s| {
                            check_control(&c).map_err(host_error)?;
                            Ok(encode(&b.create(
                                s,
                                a["kind"].as_str().unwrap(),
                                a["mimeHint"].as_str().unwrap(),
                            )?))
                        })
                        .await?
                }
                "host-media.write" => {
                    access
                        .run(move |b, s| {
                            let bytes: Vec<u8> = serde_json::from_value(a["bytes"].clone())
                                .map_err(|_| {
                                    policy::error("INVALID_INPUT", "Invalid media bytes")
                                })?;
                            b.write(s, a["handle"].as_str().unwrap(), &bytes, &c)?;
                            Ok(Value::Null)
                        })
                        .await?
                }
                "host-media.finish" => {
                    access
                        .run(move |b, s| {
                            b.finish(s, a["handle"].as_str().unwrap(), &c)?;
                            Ok(Value::Null)
                        })
                        .await?
                }
                "host-media.release" => {
                    access
                        .run(move |b, s| {
                            b.release(s, a["handle"].as_str().unwrap())?;
                            Ok(Value::Null)
                        })
                        .await?
                }
                "host-credential.capabilities" => encode(
                    &self
                        .http
                        .credentials
                        .lock()
                        .map_err(|_| policy::error("INTERNAL_ERROR", "Credential lock failed"))?
                        .capabilities(&self.http.policy, args["handle"].as_str().unwrap())?,
                ),
                "host-credential.sign" => encode(
                    &self
                        .http
                        .credentials
                        .lock()
                        .map_err(|_| policy::error("INTERNAL_ERROR", "Credential lock failed"))?
                        .sign(
                            &self.http.policy,
                            args["handle"].as_str().unwrap(),
                            args["specJson"].as_str().unwrap(),
                        )?,
                ),
                "host-state.get" => encode(
                    &lock(&self.inner.state)
                        .map_err(host_error)?
                        .get(args["key"].as_str().unwrap())
                        .cloned(),
                ),
                "host-state.put" => {
                    self.inner.snapshot.check(args).map_err(host_error)?;
                    let mut state = lock(&self.inner.state).map_err(host_error)?;
                    let mut next = state.clone();
                    next.insert(
                        args["key"].as_str().unwrap().into(),
                        args["value"].as_str().unwrap().into(),
                    );
                    let bytes = serde_json::to_vec(&next).unwrap().len() as u64;
                    if bytes
                        > self.inner.package.verified().manifest()["requestedCapabilities"]
                            ["stateBytes"]
                            .as_u64()
                            .unwrap()
                        || bytes
                            > resource_policy()["limits"]["maxStateBytes"]
                                .as_u64()
                                .unwrap()
                    {
                        return Err(policy::error(
                            "BUDGET_EXCEEDED",
                            "Scoped state exceeds declared/public byte budget",
                        ));
                    }
                    *state = next;
                    Value::Null
                }
                "host-log.log" => {
                    let mut value = args.clone();
                    {
                        let vault = self.http.credentials.lock().map_err(|_| {
                            policy::error("INTERNAL_ERROR", "Credential log lock failed")
                        })?;
                        for k in ["code", "message"] {
                            value[k] = json!(vault.redact(value[k].as_str().unwrap()));
                        }
                    }
                    if serde_json::to_vec(&value).unwrap().len() as u64
                        > resource_policy()["limits"]["maxDiagnosticBytes"]
                            .as_u64()
                            .unwrap()
                    {
                        return Err(policy::error(
                            "BUDGET_EXCEEDED",
                            "Guest diagnostic exceeds public byte budget",
                        ));
                    }
                    self.trace
                        .emit(
                            json!({"stage":"guest_log","entry":value}),
                            self.inner.snapshot.clone(),
                        )
                        .await
                        .map_err(host_error)?;
                    Value::Null
                }
                "host-stream.emit-event" => {
                    let facts = access.run(move |b, s| Ok(b.validation_context(s))).await?;
                    self.publisher
                        .emit(
                            serde_json::from_value(args["event"].clone())
                                .map_err(|_| policy::error("INVALID_INPUT", "Invalid event"))?,
                            Some(&facts),
                            control,
                        )
                        .await?;
                    Value::Null
                }
                "host-checkpoint.read" => {
                    let root = self.root_id.clone().ok_or_else(|| {
                        policy::error(
                            "PERMISSION_DENIED",
                            "This invocation has no request checkpoint",
                        )
                    })?;
                    let store = self.inner.checkpoints.clone();
                    let request = args["requestId"].as_str().unwrap().to_owned();
                    blocking(move || {
                        Ok(lock(&store)?
                            .read(&root, &request)?
                            .map(|v| json!({"revision":v["revision"],"valueJson":v.to_string()}))
                            .unwrap_or(Value::Null))
                    })
                    .await
                    .map_err(host_error)?
                }
                "host-checkpoint.write" => {
                    let session = self.session.clone().ok_or_else(|| {
                        policy::error(
                            "PERMISSION_DENIED",
                            "This invocation has no request checkpoint",
                        )
                    })?;
                    let context = self.context.clone().ok_or_else(|| {
                        policy::error("PERMISSION_DENIED", "This invocation has no frozen context")
                    })?;
                    let store = self.inner.checkpoints.clone();
                    let media = self.inner.media.clone();
                    let scope = self.scope.clone();
                    blocking(move || {
                        let value: Value =
                            serde_json::from_str(a["valueJson"].as_str().unwrap())
                                .map_err(|_| fail("INVALID_INPUT", "Checkpoint JSON is invalid"))?;
                        let media = lock(&media)?;
                        let revision = lock(&session)?.write_with_media(
                            &mut *lock(&store)?,
                            a["requestId"].as_str().unwrap(),
                            a["expectedRevision"].as_str().unwrap(),
                            value,
                            &context,
                            &media,
                            &scope,
                        )?;
                        Ok(json!({"revision":revision}))
                    })
                    .await
                    .map_err(host_error)?
                }
                _ => return Err(policy::error("INVALID_INPUT", "Unknown managed host port")),
            };
            let schema: Value =
                serde_json::from_str(include_str!("../../../schemas/host-ports.schema.json"))
                    .unwrap();
            let signature = schema["x-wit-functions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| {
                    format!(
                        "{}.{}",
                        m["interface"].as_str().unwrap(),
                        m["name"].as_str().unwrap()
                    ) == method
                })
                .ok_or_else(|| policy::error("INTERNAL_ERROR", "Public port signature missing"))?;
            let reference = signature["returns"]["$ref"].as_str().unwrap();
            let pointer = reference.split_once('#').unwrap().1;
            SchemaRegistry::public()
                .and_then(|r| {
                    r.validate_at(
                        "https://bgxiong.local/vendor-adapter/v2/host-ports.schema.json",
                        pointer,
                        &result,
                    )
                })
                .map_err(cv)
                .map_err(host_error)?;
            Ok(result)
        })
    }
}
