//! Pull-only generic bytes. No SSE, NDJSON or provider payload interpretation.
use crate::{
    http::HttpBroker,
    media::LeaseScope,
    policy::{error, Result},
};
use bgx_adapter_v2_ports::{
    resource_policy,
    wire::{
        HostPortCallV2HttpRequest as HttpRequest, HostPortCallV2StreamRead as StreamRead,
        HostPortCallV2StreamResponse as StreamResponse,
    },
};
use bgx_adapter_v2_transport::Control;
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};
use uuid::Uuid;

struct Stream {
    scope: LeaseScope,
    response: Option<reqwest::Response>,
    control: Control,
    pending: Vec<u8>,
    offset: usize,
    eof: bool,
    limit: usize,
}
pub struct HttpStreams {
    http: Arc<HttpBroker>,
    streams: HashMap<String, Stream>,
    features: BTreeSet<String>,
}
impl HttpStreams {
    pub fn new(http: Arc<HttpBroker>, negotiated_features: BTreeSet<String>) -> Self {
        Self {
            http,
            streams: HashMap::new(),
            features: negotiated_features,
        }
    }
    fn enabled(&self) -> Result<()> {
        let catalog: Value =
            serde_json::from_str(include_str!("../../../protocol/host-feature-catalog.json"))
                .expect("public host features");
        let feature = catalog["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| {
                f["requiredImports"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == "host-http-stream")
            })
            .ok_or_else(|| error("INTERNAL_ERROR", "HTTP stream feature is not published"))?;
        if !self.features.contains(feature["id"].as_str().unwrap()) {
            return Err(error(
                "PERMISSION_DENIED",
                "Managed HTTP stream feature was not negotiated",
            ));
        }
        Ok(())
    }
    pub fn retained_bytes(&self) -> usize {
        self.streams.values().map(|s| s.pending.capacity()).sum()
    }
    pub fn active_streams(&self) -> usize {
        self.streams.len()
    }
    pub async fn open(
        &mut self,
        plan: &HttpRequest,
        credential: Option<&str>,
        control: &Control,
    ) -> Result<StreamResponse> {
        self.enabled()?;
        let plan_value = serde_json::to_value(plan)
            .map_err(|_| error("INVALID_INPUT", "Stream plan cannot be encoded"))?;
        let limit = plan_value["sink"]["maxBytes"].as_u64().unwrap_or(0);
        if plan_value["sink"]["kind"] != "buffer"
            || limit == 0
            || limit
                > resource_policy()["limits"]["maxBufferedHttpBytes"]
                    .as_u64()
                    .unwrap()
        {
            return Err(error(
                "INVALID_INPUT",
                "Stream sink requires a positive public buffer budget",
            ));
        }
        let opened = self.http.open(plan, credential, control).await?;
        let status = opened.response.status().as_u16();
        let mut headers = vec![];
        let mut bytes = 0usize;
        for (name, value) in opened.response.headers() {
            let value = value.to_str().map_err(|_| {
                error(
                    "INVALID_INPUT",
                    "Stream response contains a non-text header",
                )
            })?;
            bytes = bytes
                .checked_add(name.as_str().len() + value.len())
                .ok_or_else(|| error("BUDGET_EXCEEDED", "Stream headers overflow"))?;
            if bytes as u64
                > resource_policy()["limits"]["maxFrameBytes"]
                    .as_u64()
                    .unwrap()
            {
                return Err(error(
                    "BUDGET_EXCEEDED",
                    "Stream headers exceed public policy",
                ));
            }
            headers.push(json!({"name":name.as_str(),"value":value}));
        }
        let id = Uuid::new_v4().to_string();
        let result =
            serde_json::from_value(json!({"streamId":id,"status":status,"headers":headers}))
                .map_err(|_| error("INTERNAL_ERROR", "Stream receipt cannot be encoded"))?;
        self.streams.insert(
            id,
            Stream {
                scope: self.http.policy.scope().clone(),
                response: Some(opened.response),
                control: opened.control,
                pending: vec![],
                offset: 0,
                eof: false,
                limit: limit as usize,
            },
        );
        Ok(result)
    }
    pub async fn read(
        &mut self,
        scope: &LeaseScope,
        id: &str,
        max_bytes: usize,
        control: &Control,
    ) -> Result<StreamRead> {
        self.enabled()?;
        if let Err(error) = self.http.policy.check(scope) {
            if scope == self.http.policy.scope() {
                self.release_request(scope);
            }
            return Err(error);
        }
        if max_bytes == 0
            || max_bytes as u64
                > resource_policy()["limits"]["maxHttpChunkBytes"]
                    .as_u64()
                    .unwrap()
        {
            return Err(error(
                "INVALID_INPUT",
                "Stream read size is outside the public chunk budget",
            ));
        }
        let stream = self
            .streams
            .get(id)
            .ok_or_else(|| error("PERMISSION_DENIED", "Stream handle is not live"))?;
        if &stream.scope != scope {
            return Err(error(
                "PERMISSION_DENIED",
                "Stream handle belongs to another request",
            ));
        }
        // Owning the entry across await is deliberate: dropping this future drops
        // the live response, closes the connection and removes the handle.
        let mut stream = self.streams.remove(id).expect("checked stream");
        let retained = self.retained_bytes();
        let http = self.http.clone();
        let stream_control = stream.control.clone();
        let result = http
            .guarded(
                control,
                http.guarded(&stream_control, async {
                    while stream.pending.is_empty() && !stream.eof {
                        let response = stream.response.as_mut().ok_or_else(|| {
                            error("INTERNAL_ERROR", "Live stream response is missing")
                        })?;
                        match response.chunk().await {
                            Ok(Some(chunk)) => {
                                if chunk.is_empty() {
                                    tokio::task::yield_now().await;
                                    continue;
                                }
                                if chunk.len() > stream.limit
                                    || retained.checked_add(chunk.len()).is_none_or(|n| {
                                        n > resource_policy()["limits"]["maxBufferedHttpBytes"]
                                            .as_u64()
                                            .unwrap()
                                            as usize
                                    })
                                {
                                    return Err(error(
                                        "BUDGET_EXCEEDED",
                                        "Retained HTTP stream buffers exceed public policy",
                                    ));
                                }
                                stream.pending = chunk.to_vec();
                                stream.offset = 0;
                            }
                            Ok(None) => {
                                stream.eof = true;
                                stream.response = None;
                            }
                            Err(_) => {
                                return Err(error(
                                    "RESOURCE_UNAVAILABLE",
                                    "HTTP stream was interrupted; it has not been restarted",
                                ))
                            }
                        }
                    }
                    let count = max_bytes.min(stream.pending.len().saturating_sub(stream.offset));
                    let bytes = stream.pending[stream.offset..stream.offset + count].to_vec();
                    stream.offset += count;
                    if stream.offset == stream.pending.len() {
                        stream.pending = Vec::new();
                        stream.offset = 0;
                    }
                    serde_json::from_value(json!({"bytes":bytes,"eof":stream.eof}))
                        .map_err(|_| error("INTERNAL_ERROR", "Stream chunk cannot be encoded"))
                }),
            )
            .await;
        if result.is_ok() {
            self.streams.insert(id.into(), stream);
        }
        result
    }
    pub fn close(&mut self, scope: &LeaseScope, id: &str) -> Result<()> {
        let stream = self
            .streams
            .get(id)
            .ok_or_else(|| error("PERMISSION_DENIED", "Stream handle is not live"))?;
        if &stream.scope != scope {
            return Err(error(
                "PERMISSION_DENIED",
                "Stream handle belongs to another request",
            ));
        }
        self.streams.remove(id);
        Ok(())
    }
    pub fn release_request(&mut self, scope: &LeaseScope) {
        self.streams.retain(|_, stream| &stream.scope != scope);
    }
}
