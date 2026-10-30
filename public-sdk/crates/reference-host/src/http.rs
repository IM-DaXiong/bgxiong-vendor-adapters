//! Actual asynchronous HTTP. Both streaming and buffered operations start in
//! open(): identical policy, DNS pinning, headers and credentials. No automatic
//! redirects, retries, proxy discovery or vendor-specific payload modifications.
use crate::{
    credential::CredentialVault,
    media::{LeaseScope, MediaBroker},
    policy::{error, HttpAudit, NetworkPolicy, Result},
};
use bgx_adapter_v2_ports::{
    decimal_u64, resource_policy,
    wire::{HostPortCallV2HttpRequest as HttpRequest, HostPortCallV2HttpResponse as HttpResponse},
};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{media::valid_basename, SchemaRegistry};
use futures_util::{stream, StreamExt};
use serde_json::json;
use std::{
    collections::VecDeque,
    future::Future,
    sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct MediaAccess {
    pub broker: Arc<Mutex<MediaBroker>>,
    pub scope: LeaseScope,
}
impl MediaAccess {
    pub async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut MediaBroker, &LeaseScope) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let owner = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut broker = owner
                .broker
                .lock()
                .map_err(|_| error("INTERNAL_ERROR", "Development media broker lock failed"))?;
            f(&mut broker, &owner.scope)
        })
        .await
        .map_err(|_| {
            error(
                "INTERNAL_ERROR",
                "Development media operation could not finish",
            )
        })?
    }
    async fn body(
        &self,
        handle: &str,
        control: &Control,
    ) -> Result<(reqwest::Body, u64, String, String)> {
        let h = handle.to_string();
        let (stat, file_name) = self
            .run(move |b, s| Ok((b.info(s, &h)?, b.file_name(s, &h)?)))
            .await?;
        let length = decimal_u64(&stat.r#byte_length)
            .map_err(|_| error("INVALID_INPUT", "Media length is invalid"))?;
        let access = self.clone();
        let handle = handle.to_string();
        let c = control.clone();
        let chunks = stream::try_unfold(
            (access, handle, 0u64, c),
            move |(access, handle, offset, control)| async move {
                if offset == length {
                    return Ok(None);
                }
                let read_handle = handle.clone();
                let c = control.clone();
                let bytes = access
                    .run(move |b, s| {
                        b.read(
                            s,
                            &read_handle,
                            offset,
                            resource_policy()["limits"]["maxHttpChunkBytes"]
                                .as_u64()
                                .unwrap() as usize,
                            &c,
                        )
                    })
                    .await
                    .map_err(|_| std::io::Error::other("Managed upload media is unavailable"))?;
                if bytes.is_empty() {
                    return Err(std::io::Error::other(
                        "Managed upload ended before measured byte length",
                    ));
                }
                let next = offset + bytes.len() as u64;
                Ok(Some((bytes, (access, handle, next, control))))
            },
        );
        Ok((
            reqwest::Body::wrap_stream(chunks),
            length,
            stat.r#mime,
            file_name,
        ))
    }
}
pub struct OpenedHttp {
    pub response: reqwest::Response,
    pub control: Control,
}
struct OutputGuard {
    access: MediaAccess,
    handle: String,
    control: Control,
    armed: bool,
}
impl Drop for OutputGuard {
    fn drop(&mut self) {
        if self.armed {
            self.control.cancel();
            let access = self.access.clone();
            let handle = self.handle.clone();
            tokio::task::spawn_blocking(move || {
                let cleanup = access
                    .broker
                    .lock()
                    .map_err(|_| error("INTERNAL_ERROR", "Media cleanup lock failed"))
                    .and_then(|mut b| b.discard(&access.scope, &handle));
                if let Err(error) = cleanup {
                    eprintln!("SDK HTTP media cleanup failed: {}", error.r#message);
                }
            });
        }
    }
}
pub struct HttpBroker {
    pub policy: NetworkPolicy,
    pub credentials: Arc<Mutex<CredentialVault>>,
    pub media: MediaAccess,
    schema: SchemaRegistry,
    audit: Mutex<VecDeque<HttpAudit>>,
    before_send: Option<(Arc<dyn crate::before_send::BeforeSendSink>, String)>,
    call_index: AtomicU64,
}
impl HttpBroker {
    pub fn new(
        policy: NetworkPolicy,
        credentials: Arc<Mutex<CredentialVault>>,
        media: MediaAccess,
    ) -> Result<Self> {
        if policy.scope() != &media.scope {
            return Err(error(
                "PERMISSION_DENIED",
                "HTTP and media request scopes differ",
            ));
        }
        Ok(Self {
            policy,
            credentials,
            media,
            schema: SchemaRegistry::public()
                .map_err(|_| error("INTERNAL_ERROR", "Public HTTP schema unavailable"))?,
            audit: Mutex::new(VecDeque::new()),
            before_send: None,
            call_index: AtomicU64::new(0),
        })
    }
    pub fn with_before_send(mut self, sink: Arc<dyn crate::before_send::BeforeSendSink>, operation_id: String) -> Self {
        self.before_send = Some((sink, operation_id));
        self
    }
    pub fn audit(&self) -> Result<Vec<HttpAudit>> {
        Ok(self
            .audit
            .lock()
            .map_err(|_| error("INTERNAL_ERROR", "HTTP audit lock failed"))?
            .iter()
            .cloned()
            .collect())
    }
    fn record(&self, entry: HttpAudit) -> Result<()> {
        let mut events = self
            .audit
            .lock()
            .map_err(|_| error("INTERNAL_ERROR", "HTTP audit lock failed"))?;
        events.push_back(entry);
        let max = resource_policy()["limits"]["maxStateBytes"]
            .as_u64()
            .unwrap() as usize;
        while serde_json::to_vec(&*events)
            .map_err(|_| error("INTERNAL_ERROR", "HTTP audit encoding failed"))?
            .len()
            > max
        {
            events.pop_front();
        }
        Ok(())
    }
    pub async fn guarded<T>(
        &self,
        control: &Control,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        self.policy.check(self.policy.scope())?;
        tokio::select! {biased;
            _=control.cancelled()=>Err(error("CANCELLED","Local HTTP operation cancelled; remote outcome is not confirmed")),
            _=control.expired()=>Err(error("TIMEOUT","HTTP request deadline exhausted")),
            _=self.policy.revoked()=>Err(error("PERMISSION_DENIED","Network permission was revoked")),
            value=future=>value,
        }
    }
    pub async fn open(
        &self,
        plan: &HttpRequest,
        credential: Option<&str>,
        control: &Control,
    ) -> Result<OpenedHttp> {
        let plan = serde_json::to_value(plan)
            .map_err(|_| error("INVALID_INPUT", "HTTP plan could not be encoded"))?;
        self.schema
            .validate_at(
                "https://bgxiong.local/vendor-adapter/v2/host-ports.schema.json",
                "/$defs/httpRequest",
                &plan,
            )
            .map_err(|_| {
                error(
                    "INVALID_INPUT",
                    "HTTP plan does not satisfy the public schema",
                )
            })?;
        let mut control = control.clone();
        let timeout = plan["timeoutMs"].as_u64().unwrap();
        if timeout > 0 {
            let deadline = Instant::now()
                .checked_add(Duration::from_millis(timeout))
                .ok_or_else(|| {
                    error(
                        "INVALID_INPUT",
                        "HTTP deadline is outside its supported range",
                    )
                })?;
            control.deadline = Some(
                control
                    .deadline
                    .map(|d| d.min(deadline))
                    .unwrap_or(deadline),
            );
        }
        let raw = plan["url"].as_str().unwrap();
        let method = plan["method"].as_str().unwrap();
        let parsed = crate::policy::parse_url(raw)?;
        let mut audit = HttpAudit::new(method, &parsed);
        let result = self
            .guarded(&control, async {
                let destination = self.policy.resolve(self.policy.scope(), raw).await?;
                let headers = self
                    .credentials
                    .lock()
                    .map_err(|_| error("INTERNAL_ERROR", "Development credential lock failed"))?
                    .headers(&self.policy, &destination.origin, &plan, credential)?;
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .resolve_to_addrs(destination.url.host_str().unwrap(), &destination.addresses)
                    .build()
                    .map_err(|_| {
                        error(
                            "RESOURCE_UNAVAILABLE",
                            "Cannot initialize authorized HTTP transport",
                        )
                    })?;
                let method = reqwest::Method::from_bytes(method.as_bytes())
                    .map_err(|_| error("INVALID_INPUT", "HTTP method is invalid"))?;
                if plan["body"]["kind"]=="multipart"&&headers.contains_key(reqwest::header::CONTENT_TYPE){return Err(error("INVALID_INPUT","Multipart boundary and content type are generated by the HTTP transport"));}
                let mut request = client.request(method, destination.url).headers(headers);
                match plan["body"]["kind"].as_str().unwrap() {
                    "none" => {}
                    "bytes" => {
                        let bytes: Vec<u8> = serde_json::from_value(plan["body"]["bytes"].clone())
                            .map_err(|_| error("INVALID_INPUT", "HTTP bytes are invalid"))?;
                        if bytes.len() as u64
                            > resource_policy()["limits"]["maxBufferedHttpBytes"]
                                .as_u64()
                                .unwrap()
                        {
                            return Err(error(
                                "BUDGET_EXCEEDED",
                                "Buffered HTTP request exceeds the public byte budget",
                            ));
                        }
                        request = request.body(bytes);
                    }
                    "media" => {
                        let (body, length, _, _) = self
                            .media
                            .body(plan["body"]["handle"].as_str().unwrap(), &control)
                            .await?;
                        request = request
                            .header(reqwest::header::CONTENT_LENGTH, length)
                            .body(body);
                    }
                    "multipart" => {
                        let mut form = reqwest::multipart::Form::new();
                        for part in plan["body"]["parts"].as_array().unwrap() {
                            let name = part["name"].as_str().unwrap();
                            if part["kind"] == "field" {
                                let value = part["value"].as_str().unwrap();
                                if value.len() as u64
                                    > resource_policy()["limits"]["maxBufferedHttpBytes"]
                                        .as_u64()
                                        .unwrap()
                                {
                                    return Err(error(
                                        "BUDGET_EXCEEDED",
                                        "Multipart field exceeds the public byte budget",
                                    ));
                                }
                                form = form.text(name.to_string(), value.to_string());
                            } else {
                                let filename = part["fileName"].as_str().unwrap();
                                if !valid_basename(filename) {
                                    return Err(error(
                                        "INVALID_INPUT",
                                        "Multipart file name must be a basename",
                                    ));
                                }
                                let (body, length, mime, measured_name) = self
                                    .media
                                    .body(part["handle"].as_str().unwrap(), &control)
                                    .await?;
                                if std::path::Path::new(filename).extension()!=std::path::Path::new(&measured_name).extension(){return Err(error("INVALID_INPUT","Multipart file extension differs from measured media format"));}
                                if part["mime"] != mime {
                                    return Err(error(
                                        "INVALID_INPUT",
                                        "Multipart MIME differs from measured media",
                                    ));
                                }
                                let file =
                                    reqwest::multipart::Part::stream_with_length(body, length)
                                        .file_name(filename.to_string())
                                        .mime_str(&mime)
                                        .map_err(|_| {
                                            error(
                                                "INVALID_INPUT",
                                                "Measured multipart MIME is invalid",
                                            )
                                        })?;
                                form = form.part(name.to_string(), file);
                            }
                        }
                        request = request.multipart(form);
                    }
                    _ => return Err(error("INVALID_INPUT", "Unsupported HTTP body kind")),
                }
                self.policy.check(self.policy.scope())?;
                if let Some((sink, operation_id)) = &self.before_send {
                    let (body, text) = crate::before_send::body(&plan, &self.media, &control).await?;
                    let index = self.call_index.fetch_add(1, Ordering::SeqCst) + 1;
                    let mut record = json!({"schemaVersion":1,"type":"before_send",
                        "exchangeId":uuid::Uuid::new_v4().to_string(),"requestId":self.media.scope.request_id,
                        "callIndex":index,"operationId":operation_id,"method":plan["method"],
                        "url":plan["url"],"body":body,"text":text});
                    {
                        let vault = self.credentials.lock().map_err(|_| error("INTERNAL_ERROR", "Credential audit lock failed"))?;
                        record["url"] = json!(crate::before_send::redact_url(raw, &vault));
                        crate::before_send::redact(&mut record, &vault);
                    }
                    // This await is inside guarded(): cancellation/deadline/revocation still apply.
                    sink.persist(&record).await?;
                    self.policy.check(self.policy.scope())?;
                }
                request.send().await.map_err(|_| {
                    error(
                        "RESOURCE_UNAVAILABLE",
                        "Authorized HTTP connection failed; upstream outcome may be unknown",
                    )
                })
            })
            .await;
        audit.outcome = if result.is_ok() {
            "response_headers"
        } else {
            "failed"
        }
        .into();
        audit.status = result.as_ref().ok().map(|r| r.status().as_u16());
        self.record(audit)?;
        Ok(OpenedHttp {
            response: result?,
            control,
        })
    }
    pub async fn execute(
        &self,
        plan: &HttpRequest,
        credential: Option<&str>,
        control: &Control,
    ) -> Result<HttpResponse> {
        let value = serde_json::to_value(plan)
            .map_err(|_| error("INVALID_INPUT", "HTTP plan could not be encoded"))?;
        let buffered = value["sink"]["kind"] == "buffer";
        let max = if buffered {
            let max = value["sink"]["maxBytes"].as_u64().unwrap();
            if max
                > resource_policy()["limits"]["maxBufferedHttpBytes"]
                    .as_u64()
                    .unwrap()
            {
                return Err(error(
                    "BUDGET_EXCEEDED",
                    "HTTP response buffer exceeds public policy",
                ));
            }
            max as usize
        } else {
            0
        };
        let opened = self.open(plan, credential, control).await?;
        let status = opened.response.status().as_u16();
        let mut headers = vec![];
        let mut header_bytes = 0usize;
        for (name, value) in opened.response.headers() {
            let value = value
                .to_str()
                .map_err(|_| error("INVALID_INPUT", "Upstream returned a non-text HTTP header"))?;
            header_bytes = header_bytes
                .checked_add(name.as_str().len() + value.len())
                .ok_or_else(|| error("BUDGET_EXCEEDED", "HTTP header length overflow"))?;
            if header_bytes as u64
                > resource_policy()["limits"]["maxFrameBytes"]
                    .as_u64()
                    .unwrap()
            {
                return Err(error(
                    "BUDGET_EXCEEDED",
                    "HTTP headers exceed public policy",
                ));
            }
            headers.push(json!({"name":name.as_str(),"value":value}));
        }
        let handle = if buffered {
            None
        } else {
            Some(self.media.run(|b, s| b.create_detected(s)).await?)
        };
        let mut output_guard = handle.as_ref().map(|h| OutputGuard {
            access: self.media.clone(),
            handle: h.clone(),
            control: opened.control.clone(),
            armed: true,
        });
        let result = self
            .guarded(&opened.control, async {
                let mut stream = opened.response.bytes_stream();
                let mut bytes = Vec::new();
                while let Some(chunk) = stream.next().await {
                    self.policy.check(self.policy.scope())?;
                    let chunk = chunk.map_err(|_| {
                        error("RESOURCE_UNAVAILABLE", "HTTP response body was interrupted")
                    })?;
                    if buffered {
                        if chunk.len() > max.saturating_sub(bytes.len()) {
                            return Err(error(
                                "BUDGET_EXCEEDED",
                                "HTTP response exceeded its explicit buffer budget",
                            ));
                        }
                        bytes.extend_from_slice(&chunk);
                    } else {
                        for piece in chunk.chunks(
                            resource_policy()["limits"]["maxHttpChunkBytes"]
                                .as_u64()
                                .unwrap() as usize,
                        ) {
                            let h = handle.as_ref().unwrap().clone();
                            let part = piece.to_vec();
                            let c = opened.control.clone();
                            self.media
                                .run(move |b, s| b.write(s, &h, &part, &c))
                                .await?;
                        }
                    }
                }
                let payload = if let Some(handle) = &handle {
                    let h = handle.clone();
                    let c = opened.control.clone();
                    self.media.run(move |b, s| b.finish(s, &h, &c)).await?;
                    json!({"kind":"media","handle":handle})
                } else {
                    json!({"kind":"buffered","bytes":bytes})
                };
                serde_json::from_value(json!({"status":status,"headers":headers,"payload":payload}))
                    .map_err(|_| error("INTERNAL_ERROR", "HTTP result could not be encoded"))
            })
            .await;
        if result.is_err() {
            if let Some(handle) = handle {
                self.media.run(move |b, s| b.discard(s, &handle)).await?;
            }
        }
        if let Some(guard) = &mut output_guard {
            guard.armed = false;
        }
        result
    }
}
