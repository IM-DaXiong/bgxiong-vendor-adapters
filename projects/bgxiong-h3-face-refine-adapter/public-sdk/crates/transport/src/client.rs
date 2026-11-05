use crate::{
    framing,
    process::{self, Fault, FaultKind, Options},
};
use bgx_adapter_v2_ports::{
    decimal_u64,
    wire::{
        host::HostError, HostPortCallV2, Invocation, Response, WorkerEnvelopeV2,
        WorkerEnvelopeV2Branch1Package,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::{
    sync::{mpsc, oneshot, Notify},
    time::{sleep_until, timeout, Instant as TokioInstant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Control {
    cancel: CancellationToken,
    pub deadline: Option<Instant>,
}
impl Control {
    pub fn new(deadline: Option<Instant>) -> Self {
        Self {
            cancel: CancellationToken::new(),
            deadline,
        }
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await
    }
    pub async fn expired(&self) {
        match self.deadline {
            Some(at) => sleep_until(TokioInstant::from_std(at)).await,
            None => std::future::pending().await,
        }
    }
}
pub type PortFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, HostError>> + Send + 'a>>;
/// The future owns the managed effect's cancellation cleanup. Dropping it must
/// release its stream/connection/temporary writes; no blocking work in poll.
pub trait ManagedHost: Send {
    fn call<'a>(&'a mut self, call: HostPortCallV2, control: &'a Control) -> PortFuture<'a>;
}
#[derive(Debug)]
pub struct Receipt {
    pub response: Response,
    pub worker_pid: u32,
    pub stderr_tail: Vec<u8>,
}
struct Active {
    count: AtomicUsize,
    idle: Notify,
}
struct Lease(Arc<Active>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::SeqCst);
        self.0.idle.notify_waiters();
    }
}
#[derive(Clone)]
pub struct WorkerClient {
    worker: PathBuf,
    options: Options,
    active: Arc<Active>,
}
fn packet(value: Value) -> Result<WorkerEnvelopeV2, Fault> {
    serde_json::from_value(value)
        .map_err(|_| Fault::new(FaultKind::Protocol, "Cannot encode public worker envelope"))
}
impl WorkerClient {
    pub fn new(sdk: &Path, worker: &Path, options: Options) -> Result<Self, Fault> {
        Ok(Self {
            worker: process::worker_path(sdk, worker, &options)?,
            options,
            active: Arc::new(Active {
                count: AtomicUsize::new(0),
                idle: Notify::new(),
            }),
        })
    }
    pub fn active_workers(&self) -> usize {
        self.active.count.load(Ordering::SeqCst)
    }
    pub async fn wait_idle(&self) {
        loop {
            let notified = self.active.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.active_workers() == 0 {
                return;
            }
            notified.await;
        }
    }
    /// One supervisor owns one worker. Dropping this waiting future closes its
    /// receipt channel; the supervisor then interrupts, kills if needed, waits,
    /// drains stderr, and releases its active lease. Keep the Tokio runtime alive
    /// through wait_idle() during application shutdown.
    pub async fn invoke(
        &self,
        package_root: PathBuf,
        package: WorkerEnvelopeV2Branch1Package,
        request: Invocation,
        host: Box<dyn ManagedHost>,
        control: Control,
    ) -> Result<Receipt, Fault> {
        let (reply, receive) = oneshot::channel();
        let owner = self.clone();
        self.active.count.fetch_add(1, Ordering::SeqCst);
        let lease = Lease(self.active.clone());
        tokio::spawn(async move {
            let _lease = lease;
            let mut reply = reply;
            let result = owner
                .supervise(package_root, package, request, host, control, &mut reply)
                .await;
            let _ = reply.send(result);
        });
        receive.await.map_err(|_| {
            Fault::new(
                FaultKind::Supervisor,
                "Worker supervisor ended without a receipt",
            )
        })?
    }
    async fn supervise(
        &self,
        root: PathBuf,
        package: WorkerEnvelopeV2Branch1Package,
        request: Invocation,
        mut host: Box<dyn ManagedHost>,
        control: Control,
        reply: &mut oneshot::Sender<Result<Receipt, Fault>>,
    ) -> Result<Receipt, Fault> {
        if control.is_cancelled() {
            return Err(Fault::new(
                FaultKind::Cancelled,
                "Local invocation cancelled before start",
            ));
        }
        if control.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(Fault::new(
                FaultKind::Deadline,
                "Host request deadline expired before start",
            ));
        }
        let mut child = process::spawn(&self.worker, &root)?;
        let pid = child
            .id()
            .ok_or_else(|| Fault::new(FaultKind::Spawn, "Worker identity unavailable"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Fault::new(FaultKind::Spawn, "Worker input unavailable"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| Fault::new(FaultKind::Spawn, "Worker output unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Fault::new(FaultKind::Spawn, "Worker diagnostic pipe unavailable"))?;
        let diagnostic = tokio::spawn(process::drain_tail(stderr));
        let (sender, mut frames) = mpsc::channel(1);
        let mut reader = tokio::spawn(async move {
            loop {
                let frame = framing::read(&mut stdout).await;
                let done = !matches!(&frame, Ok(Some(_)));
                if sender.send(frame).await.is_err() || done {
                    break;
                }
            }
        });
        let mut dispatched = false;
        let exchange = async {
            let hello = packet(
                json!({"schemaVersion":2,"kind":"hello","protocolVersion":2,"witPackage":"bgxiong:vendor-adapter@2.0.0","policyDigest":hex::encode(Sha256::digest(include_bytes!("../../../protocol/schema-policy.json")))}),
            )?;
            let expected = serde_json::to_value(&hello)
                .map_err(|_| Fault::new(FaultKind::Protocol, "Cannot encode handshake"))?;
            timeout(self.options.handshake_timeout, async {
                framing::write(&mut stdin, &hello)
                    .await
                    .map_err(|_| Fault::new(FaultKind::Io, "Cannot write worker handshake"))?;
                let received = next_frame(&mut frames).await?;
                if serde_json::to_value(received).ok().as_ref() != Some(&expected) {
                    return Err(Fault::new(
                        FaultKind::Handshake,
                        "Worker protocol/WIT/policy handshake differs",
                    ));
                }
                Ok(())
            })
            .await
            .map_err(|_| {
                Fault::new(FaultKind::Handshake, "Worker handshake deadline exhausted")
            })??;
            let request_id = request.r#request_id.clone();
            let operation = serde_json::to_value(&request.r#operation)
                .map_err(|_| Fault::new(FaultKind::Protocol, "Cannot encode operation"))?;
            let invoke = packet(
                json!({"schemaVersion":2,"kind":"invoke","requestId":request_id,"package":package,"invocation":request}),
            )?;
            // A partially written invoke can already have side effects at the
            // receiver: uncertainty starts BEFORE the write, not after its ack.
            dispatched = true;
            timeout(
                self.options.write_timeout,
                framing::write(&mut stdin, &invoke),
            )
            .await
            .map_err(|_| Fault::new(FaultKind::Io, "Worker invocation write deadline exhausted"))?
            .map_err(|_| Fault::new(FaultKind::Io, "Cannot write worker invocation"))?;
            let mut next_call = 1u64;
            let response = loop {
                let frame = next_frame(&mut frames).await?;
                let value = serde_json::to_value(frame)
                    .map_err(|_| Fault::new(FaultKind::Protocol, "Invalid worker frame"))?;
                if value["requestId"] != request_id {
                    return Err(Fault::new(
                        FaultKind::Protocol,
                        "Worker frame belongs to another request",
                    ));
                }
                match value["kind"].as_str() {
                    Some("response") => {
                        let response: Response = serde_json::from_value(value["response"].clone())
                            .map_err(|_| {
                                Fault::new(FaultKind::Protocol, "Invalid final worker response")
                            })?;
                        if value["response"]["operation"] != operation {
                            return Err(Fault::new(
                                FaultKind::Protocol,
                                "Worker response operation differs",
                            ));
                        }
                        if let Some(data) = value["response"].get("dataJson") {
                            serde_json::from_str::<Value>(data.as_str().ok_or_else(|| {
                                Fault::new(FaultKind::Protocol, "Invalid response dataJson")
                            })?)
                            .map_err(|_| {
                                Fault::new(FaultKind::Protocol, "Malformed response JSON")
                            })?;
                        }
                        break response;
                    }
                    Some("port_call") => {
                        if value["callId"].as_str().and_then(|s| decimal_u64(s).ok())
                            != Some(next_call)
                        {
                            return Err(Fault::new(
                                FaultKind::Protocol,
                                "Duplicate, reordered or overflowing port call identity",
                            ));
                        }
                        next_call = next_call.checked_add(1).ok_or_else(|| {
                            Fault::new(FaultKind::Protocol, "Port call identity exhausted")
                        })?;
                        let call: HostPortCallV2 = serde_json::from_value(value["call"].clone())
                            .map_err(|_| {
                                Fault::new(FaultKind::Protocol, "Invalid managed port call")
                            })?;
                        let port_result = tokio::select! {
                            biased;
                            _=frames.recv()=>return Err(Fault::new(FaultKind::Protocol,"Worker sent another frame or exited before its managed port returned")),
                            result=host.call(call,&control)=>result,
                        };
                        let result = match port_result {
                            Ok(value) => json!({"ok":value}),
                            Err(error) => json!({"error":error}),
                        };
                        let frame = packet(
                            json!({"schemaVersion":2,"kind":"port_result","requestId":request_id,"callId":value["callId"],"result":result}),
                        )?;
                        timeout(
                            self.options.write_timeout,
                            framing::write(&mut stdin, &frame),
                        )
                        .await
                        .map_err(|_| {
                            Fault::new(FaultKind::Io, "Worker port write deadline exhausted")
                        })?
                        .map_err(|_| {
                            Fault::new(FaultKind::Io, "Cannot return managed port result")
                        })?;
                    }
                    _ => {
                        return Err(Fault::new(
                            FaultKind::Protocol,
                            "Unexpected worker frame during invocation",
                        ))
                    }
                }
            };
            // Keep stdin open until the final response. Now require EOF, exactly
            // one terminal frame, and a successful process exit before success.
            stdin
                .shutdown()
                .await
                .map_err(|_| Fault::new(FaultKind::Io, "Cannot close completed worker input"))?;
            match timeout(self.options.shutdown_timeout, frames.recv()).await {
                Ok(Some(Ok(None))) => {}
                Ok(Some(Ok(Some(_)))) => {
                    return Err(Fault::new(
                        FaultKind::Protocol,
                        "Worker emitted more than one final response",
                    ))
                }
                _ => {
                    return Err(Fault::new(
                        FaultKind::Protocol,
                        "Worker did not close cleanly after final response",
                    ))
                }
            }
            Ok(response)
        };
        let result = tokio::select! {
            biased;
            _=reply.closed()=>Err(Fault::new(FaultKind::Cancelled,"Calling future dropped; local worker is being stopped")),
            _=control.cancelled()=>Err(Fault::new(FaultKind::Cancelled,"Local execution cancelled; remote cancellation is not confirmed")),
            _=control.expired()=>Err(Fault::new(FaultKind::Deadline,"Host request deadline exhausted; remote outcome may be unknown")),
            result=exchange=>result,
        };
        let mut result = result;
        if result.is_err() {
            // The exchange future is dropped, releasing any pending managed IO.
            control.cancel();
            let cancel =
                packet(json!({"schemaVersion":2,"kind":"cancel","requestId":request.r#request_id}));
            if let Ok(cancel) = cancel {
                let _ = timeout(
                    self.options.write_timeout,
                    framing::write(&mut stdin, &cancel),
                )
                .await;
            }
            let _ = stdin.shutdown().await;
        }
        drop(stdin);
        drop(frames);
        let exit = timeout(
            if result.is_ok() {
                self.options.shutdown_timeout
            } else {
                self.options.cancel_grace
            },
            child.wait(),
        )
        .await;
        match exit {
            Ok(Ok(status)) => {
                if result.is_ok() && !status.success() {
                    result = Err(Fault::new(
                        FaultKind::Exited,
                        "Worker returned a response but exited unsuccessfully",
                    ));
                }
            }
            _ => {
                let _ = child.start_kill();
                let waited = child.wait().await;
                if result.is_ok() {
                    result = Err(Fault::new(
                        FaultKind::Exited,
                        if waited.is_ok() {
                            "Worker did not exit after its response"
                        } else {
                            "Worker could not be reaped"
                        },
                    ));
                }
            }
        }
        // Pipes of the reaped child must close. Abort only the pipe readers if a
        // foreign descendant retained a handle; no detached background readers.
        match timeout(self.options.shutdown_timeout, &mut reader).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                result = Err(Fault::new(
                    FaultKind::Supervisor,
                    "Worker output reader failed",
                ));
            }
            Err(_) => {
                reader.abort();
                let _ = reader.await;
                result = Err(Fault::new(
                    FaultKind::Supervisor,
                    "Worker output pipe remained open after process cleanup",
                ));
            }
        }
        let mut diagnostic = diagnostic;
        let tail = match timeout(self.options.shutdown_timeout, &mut diagnostic).await {
            Ok(Ok(Ok(tail))) => tail,
            Ok(_) => {
                result = Err(Fault::new(
                    FaultKind::Io,
                    "Worker diagnostic pipe could not be read",
                ));
                vec![]
            }
            Err(_) => {
                diagnostic.abort();
                let _ = diagnostic.await;
                result = Err(Fault::new(
                    FaultKind::Supervisor,
                    "Worker diagnostic pipe remained open after process cleanup",
                ));
                vec![]
            }
        };
        match result {
            Ok(response) => Ok(Receipt {
                response,
                worker_pid: pid,
                stderr_tail: tail,
            }),
            Err(mut error) => {
                error.invocation_dispatched = dispatched;
                error.worker_pid = Some(pid);
                error.stderr_tail = tail;
                Err(error)
            }
        }
    }
}
use tokio::io::AsyncWriteExt;
async fn next_frame(
    frames: &mut mpsc::Receiver<std::io::Result<Option<WorkerEnvelopeV2>>>,
) -> Result<WorkerEnvelopeV2, Fault> {
    match frames.recv().await {
        Some(Ok(Some(frame))) => Ok(frame),
        Some(Ok(None)) | None => Err(Fault::new(
            FaultKind::Exited,
            "Worker exited without a final response",
        )),
        Some(Err(_)) => Err(Fault::new(
            FaultKind::Protocol,
            "Malformed, truncated or oversized worker frame",
        )),
    }
}
