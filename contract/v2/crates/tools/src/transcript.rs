//! Bounded development audit records and offline execution against an exact port tape.
//! A replay host contains no real broker, credential vault, filesystem port or HTTP client.
use bgx_adapter_v2_ports::{
    resource_policy,
    wire::{host::HostError, HostPortCallV2, Invocation, Response, WorkerEnvelopeV2Branch3Result},
};
use bgx_adapter_v2_reference_host::{checkpoint::SnapshotPolicy, config::LoadedPackage};
use bgx_adapter_v2_transport::{Control, ManagedHost, Options, PortFuture, WorkerClient};
use bgx_adapter_v2_validator::{hash_bytes, identity::PackageIdentity, SchemaRegistry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn failure(code: &'static str, message: impl Into<String>) -> crate::Error {
    crate::error(code,"transcript",message,"Use an explicit nonsecret MOCK fixture and the identical verified adapter package; no replay network access is available.")
}
fn limit() -> usize {
    resource_policy()["limits"]["maxFrameBytes"]
        .as_u64()
        .expect("public frame budget") as usize
}
fn encoded<T: Serialize>(value: &T) -> crate::Result<Value> {
    serde_json::to_value(value)
        .map_err(|_| failure("INVALID_INPUT", "Cannot encode the typed transcript field"))
}
fn budget<T: Serialize>(value: &T) -> crate::Result<usize> {
    let size = serde_json::to_vec(value)
        .map_err(|_| failure("INVALID_INPUT", "Cannot encode the transcript"))?
        .len();
    if size > limit() {
        return Err(failure(
            "BUDGET_EXCEEDED",
            "Transcript exceeds the public frame byte budget",
        ));
    }
    Ok(size)
}
fn transcript_digest(value: &Transcript) -> crate::Result<String> {
    // Hash a logical JSON object with serde_json's deterministic map ordering,
    // rather than Rust struct declaration order or caller whitespace.
    Ok(hash_bytes(&serde_json::to_vec(&encoded(value)?).map_err(
        |_| failure("INVALID_INPUT", "Cannot hash transcript JSON"),
    )?))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MockInputs {
    /// Fixed inputs for the author's mock service. These do not change a live service clock.
    pub seed: u64,
    pub now_unix_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[serde(rename = "MOCK")]
    Mock,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretHandling {
    #[serde(rename = "explicit-host-secrets-nested-json-deny-signatures")]
    ExplicitHostSecrets,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortExchange {
    pub call: HostPortCallV2,
    /// The exact generated worker result union; no transcript-only result protocol.
    pub result: WorkerEnvelopeV2Branch3Result,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordedInvocation {
    pub invocation: Invocation,
    pub ports: Vec<PortExchange>,
    pub response: Response,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transcript {
    pub schema_version: u64,
    pub scope: Scope,
    pub package: PackageIdentity,
    pub mock_inputs: MockInputs,
    pub secret_handling: SecretHandling,
    pub invocations: Vec<RecordedInvocation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptFile {
    pub sha256: String,
    pub transcript: Transcript,
}

/// Check decoded JSON strings as well as their enclosing wire fields. Refusing a
/// secret preserves exact replay semantics; a redacted tape is never called valid.
pub fn check_nonsecret(policy: &SnapshotPolicy, value: &Value) -> crate::Result<()> {
    fn visit(
        policy: &SnapshotPolicy,
        value: &Value,
        depth: u64,
        scanned: &mut usize,
    ) -> crate::Result<()> {
        if depth
            > resource_policy()["limits"]["maxSchemaDepth"]
                .as_u64()
                .unwrap()
        {
            return Err(failure(
                "BUDGET_EXCEEDED",
                "Nested transcript JSON exceeds the public depth budget",
            ));
        }
        policy.check(value).map_err(|_| {
            failure(
                "RECORD_DENIED",
                "Explicit credential material cannot be recorded",
            )
        })?;
        match value {
            Value::String(text) => {
                *scanned = scanned.saturating_add(text.len());
                if *scanned > limit() {
                    return Err(failure(
                        "BUDGET_EXCEEDED",
                        "Decoded transcript strings exceed the public frame budget",
                    ));
                }
                if let Ok(decoded) = serde_json::from_str::<Value>(text) {
                    visit(policy, &decoded, depth + 1, scanned)?;
                }
            }
            Value::Array(values) => {
                for v in values {
                    visit(policy, v, depth + 1, scanned)?;
                }
            }
            Value::Object(values) => {
                // A header with an authorization/cookie value cannot be copied
                // even when its derived secret is absent from the key inventory.
                if let (Some(name), Some(value)) = (
                    values.get("name").and_then(Value::as_str),
                    values.get("value").and_then(Value::as_str),
                ) {
                    if !value.is_empty()
                        && [
                            "authorization",
                            "proxy-authorization",
                            "cookie",
                            "set-cookie",
                        ]
                        .iter()
                        .any(|h| name.eq_ignore_ascii_case(h))
                    {
                        return Err(failure(
                            "RECORD_DENIED",
                            "Raw authentication or session headers cannot be recorded",
                        ));
                    }
                }
                for v in values.values() {
                    visit(policy, v, depth + 1, scanned)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    budget(value)?;
    visit(policy, value, 0, &mut 0)
}
fn invocation(value: &Invocation) -> crate::Result<()> {
    let value = encoded(value)?;
    SchemaRegistry::validate_public("invocation", &value).map_err(|_| {
        failure(
            "INVALID_INPUT",
            "Transcript Invocation violates the public schema",
        )
    })?;
    let payload: Value = serde_json::from_str(value["payloadJson"].as_str().unwrap())
        .map_err(|_| failure("INVALID_INPUT", "Transcript payloadJson is not JSON"))?;
    SchemaRegistry::public()
        .and_then(|r| r.validate_dispatch(value["operation"].as_str().unwrap(), "input", &payload))
        .map_err(|_| {
            failure(
                "INVALID_INPUT",
                "Transcript payload violates its public operation schema",
            )
        })
}
fn response(request: &Invocation, value: &Response) -> crate::Result<()> {
    let request = encoded(request)?;
    let value = encoded(value)?;
    SchemaRegistry::validate_public("response", &value).map_err(|_| {
        failure(
            "INVALID_INPUT",
            "Transcript Response violates the public schema",
        )
    })?;
    if value["operation"] != request["operation"] {
        return Err(failure(
            "INVALID_INPUT",
            "Transcript Response belongs to a different transport operation",
        ));
    }
    if let Some(data) = value["dataJson"].as_str() {
        let data: Value = serde_json::from_str(data)
            .map_err(|_| failure("INVALID_INPUT", "Transcript dataJson is not JSON"))?;
        SchemaRegistry::public()
            .and_then(|r| {
                r.validate_dispatch(request["operation"].as_str().unwrap(), "result", &data)
            })
            .map_err(|_| {
                failure(
                    "INVALID_INPUT",
                    "Transcript result violates its public operation schema",
                )
            })?;
    }
    Ok(())
}
fn exchange(value: &PortExchange) -> crate::Result<()> {
    let call = encoded(&value.call)?;
    SchemaRegistry::validate_public("host-ports", &call).map_err(|_| {
        failure(
            "INVALID_INPUT",
            "Transcript port call violates the public schema",
        )
    })?;
    if call["method"] == "host-credential.sign" {
        return Err(failure("RECORD_DENIED","Credential signing interactions are not recordable; use explicit fake handles in deterministic mock fixtures"));
    }
    let result = encoded(&value.result)?;
    let registry = SchemaRegistry::public()
        .map_err(|_| failure("INTERNAL_ERROR", "Public schema registry is unavailable"))?;
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/host-ports.schema.json"))
            .expect("public ports");
    let pointer = if result.get("error").is_some() {
        "/$defs/hostError"
    } else {
        let method = call["method"].as_str().unwrap();
        let f = schema["x-wit-functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| {
                format!(
                    "{}.{}",
                    f["interface"].as_str().unwrap(),
                    f["name"].as_str().unwrap()
                ) == method
            })
            .ok_or_else(|| failure("INVALID_INPUT", "Transcript port is not published"))?;
        f["returns"]["$ref"]
            .as_str()
            .unwrap()
            .split_once('#')
            .unwrap()
            .1
    };
    let output = if result.get("error").is_some() {
        &result["error"]
    } else {
        &result["ok"]
    };
    registry
        .validate_at(
            "https://bgxiong.local/vendor-adapter/v2/host-ports.schema.json",
            pointer,
            output,
        )
        .map_err(|_| {
            failure(
                "INVALID_INPUT",
                "Transcript port result violates its public return schema",
            )
        })
}
impl TranscriptFile {
    pub fn from_transcript(transcript: Transcript, policy: &SnapshotPolicy) -> crate::Result<Self> {
        let file = Self {
            sha256: transcript_digest(&transcript)?,
            transcript,
        };
        file.verify(policy)?;
        Ok(file)
    }
    pub fn verify(&self, policy: &SnapshotPolicy) -> crate::Result<()> {
        budget(self)?;
        if self.transcript.schema_version != 1 || self.transcript.invocations.is_empty() {
            return Err(failure(
                "INVALID_INPUT",
                "Transcript version or invocation list is invalid",
            ));
        }
        if self.sha256 != transcript_digest(&self.transcript)? {
            return Err(failure(
                "TRANSCRIPT_HASH_MISMATCH",
                "Transcript content differs from its recorded hash",
            ));
        }
        check_nonsecret(policy, &encoded(self)?)?;
        let mut ids = BTreeSet::new();
        for item in &self.transcript.invocations {
            invocation(&item.invocation)?;
            response(&item.invocation, &item.response)?;
            if !ids.insert(item.invocation.r#request_id.as_str()) {
                return Err(failure(
                    "INVALID_INPUT",
                    "Transcript invocation identifiers must be unique",
                ));
            }
            for port in &item.ports {
                exchange(port)?;
            }
        }
        Ok(())
    }
    pub fn load(path: &Path, policy: &SnapshotPolicy) -> crate::Result<Self> {
        Self::load_snapshot(path, policy, None)
    }
    pub fn load_matching(
        path: &Path,
        policy: &SnapshotPolicy,
        expected_file_sha256: &str,
    ) -> crate::Result<Self> {
        Self::load_snapshot(path, policy, Some(expected_file_sha256))
    }
    fn load_snapshot(
        path: &Path,
        policy: &SnapshotPolicy,
        expected_file_sha256: Option<&str>,
    ) -> crate::Result<Self> {
        crate::workspace::absolute(path, "transcript")?;
        crate::workspace::plain_path(path)?;
        let file = File::open(path).map_err(|_| {
            failure(
                "RESOURCE_UNAVAILABLE",
                "Cannot open the explicit transcript file",
            )
        })?;
        let mut bytes = Vec::new();
        file.take(limit() as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                failure(
                    "RESOURCE_UNAVAILABLE",
                    "Cannot read the explicit transcript file",
                )
            })?;
        if bytes.len() > limit() {
            return Err(failure(
                "BUDGET_EXCEEDED",
                "Transcript file exceeds the public frame budget",
            ));
        }
        if expected_file_sha256.is_some_and(|expected| expected != hash_bytes(&bytes)) {
            return Err(failure(
                "TRANSCRIPT_HASH_MISMATCH",
                "The exact replay input bytes differ from the declared evidence artifact",
            ));
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| {
            failure(
                "INVALID_INPUT",
                "Transcript is not a valid closed record object",
            )
        })?;
        value.verify(policy)?;
        Ok(value)
    }
    pub fn save(&self, path: &Path, policy: &SnapshotPolicy) -> crate::Result<()> {
        self.verify(policy)?;
        crate::workspace::absolute(path, "transcript")?;
        crate::workspace::plain_path(path)?;
        let parent = path.parent().ok_or_else(|| {
            failure(
                "INVALID_INPUT",
                "Transcript requires an explicit parent directory",
            )
        })?;
        let mut file = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| failure("RESOURCE_UNAVAILABLE", "Cannot stage transcript output"))?;
        serde_json::to_writer(file.as_file_mut(), self)
            .map_err(|_| failure("RESOURCE_UNAVAILABLE", "Cannot write transcript output"))?;
        file.write_all(b"\n")
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| {
                failure(
                    "RESOURCE_UNAVAILABLE",
                    "Cannot synchronize transcript output",
                )
            })?;
        file.persist_noclobber(path).map_err(|_| {
            failure(
                "RESOURCE_UNAVAILABLE",
                "Transcript output already exists or cannot be atomically committed",
            )
        })?;
        Ok(())
    }
}

struct Pending {
    invocation: Invocation,
    ports: Vec<PortExchange>,
    response: Option<Response>,
}
struct Recording {
    items: Vec<Pending>,
    by_id: BTreeMap<String, usize>,
    bytes: usize,
    denied: bool,
}
pub struct Recorder {
    package: PackageIdentity,
    inputs: MockInputs,
    policy: SnapshotPolicy,
    state: Mutex<Recording>,
}
impl Recorder {
    pub fn new(
        package: PackageIdentity,
        inputs: MockInputs,
        policy: SnapshotPolicy,
    ) -> crate::Result<Self> {
        check_nonsecret(&policy, &encoded(&package)?)?;
        Ok(Self {
            package,
            inputs,
            policy,
            state: Mutex::new(Recording {
                items: Vec::new(),
                by_id: BTreeMap::new(),
                bytes: 0,
                denied: false,
            }),
        })
    }
    fn update(
        &self,
        action: impl FnOnce(&mut Recording) -> crate::Result<()>,
    ) -> crate::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| failure("INTERNAL_ERROR", "Recorder lock failed"))?;
        if state.denied {
            return Err(failure(
                "RECORD_DENIED",
                "This recording previously failed and cannot be saved",
            ));
        }
        let result = action(&mut state);
        if result.is_err() {
            state.denied = true;
        }
        result
    }
    fn reserve<T: Serialize>(state: &mut Recording, value: &T) -> crate::Result<()> {
        state.bytes = state
            .bytes
            .saturating_add(budget(value)?)
            .saturating_add(32);
        if state.bytes > limit() {
            return Err(failure(
                "BUDGET_EXCEEDED",
                "Retained recording exceeds the public frame budget",
            ));
        }
        Ok(())
    }
    pub fn started(&self, value: &Invocation) -> crate::Result<()> {
        self.update(|state| {
            invocation(value)?;
            check_nonsecret(&self.policy, &encoded(value)?)?;
            if state.by_id.contains_key(&value.r#request_id) {
                return Err(failure(
                    "INVALID_INPUT",
                    "Recorder invocation identifier was already used",
                ));
            }
            Self::reserve(state, value)?;
            state
                .by_id
                .insert(value.r#request_id.clone(), state.items.len());
            state.items.push(Pending {
                invocation: value.clone(),
                ports: Vec::new(),
                response: None,
            });
            Ok(())
        })
    }
    pub fn completed_port(
        &self,
        request: &Invocation,
        call: &HostPortCallV2,
        result: &std::result::Result<Value, HostError>,
    ) -> crate::Result<()> {
        self.update(|state| {
            let wire = serde_json::from_value(match result {
                Ok(value) => json!({"ok":value}),
                Err(error) => json!({"error":error}),
            })
            .map_err(|_| failure("INVALID_INPUT", "Cannot encode the public port result"))?;
            let port = PortExchange {
                call: call.clone(),
                result: wire,
            };
            exchange(&port)?;
            check_nonsecret(&self.policy, &encoded(&port)?)?;
            let index = *state.by_id.get(&request.r#request_id).ok_or_else(|| {
                failure(
                    "INVALID_INPUT",
                    "Port was observed before its invocation started",
                )
            })?;
            if state.items[index].response.is_some() || state.items[index].invocation != *request {
                return Err(failure(
                    "INVALID_INPUT",
                    "Port was observed after the response or for a different invocation",
                ));
            }
            Self::reserve(state, &port)?;
            state.items[index].ports.push(port);
            Ok(())
        })
    }
    pub fn finished(&self, request: &Invocation, value: &Response) -> crate::Result<()> {
        self.update(|state| {
            response(request, value)?;
            check_nonsecret(&self.policy, &encoded(value)?)?;
            let index = *state.by_id.get(&request.r#request_id).ok_or_else(|| {
                failure(
                    "INVALID_INPUT",
                    "Response was observed before its invocation started",
                )
            })?;
            if state.items[index].response.is_some() || state.items[index].invocation != *request {
                return Err(failure(
                    "INVALID_INPUT",
                    "Duplicate response or changed invocation",
                ));
            }
            Self::reserve(state, value)?;
            state.items[index].response = Some(value.clone());
            Ok(())
        })
    }
    pub fn snapshot(&self) -> crate::Result<TranscriptFile> {
        let state = self
            .state
            .lock()
            .map_err(|_| failure("INTERNAL_ERROR", "Recorder lock failed"))?;
        if state.denied {
            return Err(failure(
                "RECORD_DENIED",
                "This recording failed and cannot be saved",
            ));
        }
        let invocations = state
            .items
            .iter()
            .map(|p| {
                Ok(RecordedInvocation {
                    invocation: p.invocation.clone(),
                    ports: p.ports.clone(),
                    response: p.response.clone().ok_or_else(|| {
                        failure(
                            "INVALID_INPUT",
                            "Incomplete invocation cannot be recorded as successful",
                        )
                    })?,
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        TranscriptFile::from_transcript(
            Transcript {
                schema_version: 1,
                scope: Scope::Mock,
                package: self.package.clone(),
                mock_inputs: self.inputs.clone(),
                secret_handling: SecretHandling::ExplicitHostSecrets,
                invocations,
            },
            &self.policy,
        )
    }
    pub fn save(&self, path: &Path) -> crate::Result<TranscriptFile> {
        let file = self.snapshot()?;
        file.save(path, &self.policy)?;
        Ok(file)
    }
}

struct ReplayState {
    next: usize,
    mismatch: Option<&'static str>,
}
struct TapeHost {
    ports: Vec<PortExchange>,
    state: Arc<Mutex<ReplayState>>,
}
fn port_failure() -> HostError {
    serde_json::from_value(json!({"code":"PERMISSION_DENIED","message":"Offline replay port tape mismatch; no real broker is available"})).expect("public host error")
}
impl ManagedHost for TapeHost {
    fn call<'a>(&'a mut self, call: HostPortCallV2, _control: &'a Control) -> PortFuture<'a> {
        Box::pin(async move {
            let mut state = self.state.lock().map_err(|_| port_failure())?;
            let Some(expected) = self.ports.get(state.next) else {
                state.mismatch = Some("Guest made an additional unrecorded port call");
                return Err(port_failure());
            };
            if expected.call != call {
                state.mismatch = Some("Guest port call differs from the ordered transcript");
                return Err(port_failure());
            }
            state.next += 1;
            match &expected.result {
                WorkerEnvelopeV2Branch3Result::V0(value) => Ok(value.r#ok.clone()),
                WorkerEnvelopeV2Branch3Result::V1(value) => Err(value.r#error.clone()),
            }
        })
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayInvocation {
    pub request_id: String,
    pub worker_pid: u32,
    pub port_calls: usize,
    pub response_sha256: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayReport {
    pub scope: Scope,
    pub passed: bool,
    pub transcript_sha256: String,
    pub invocations: Vec<ReplayInvocation>,
}
pub async fn replay(
    sdk: &Path,
    worker: &Path,
    package: &LoadedPackage,
    file: &TranscriptFile,
    options: Options,
    policy: &SnapshotPolicy,
) -> crate::Result<ReplayReport> {
    if package.verified().manifest()["language"] != "rust" {
        return Err(failure(
            "UNSUPPORTED_FEATURE",
            "Offline replay currently requires the actual Rust/WASM author runtime",
        ));
    }
    file.verify(policy)?;
    if file.transcript.package != *package.verified().identity() {
        return Err(failure(
            "PACKAGE_IDENTITY_MISMATCH",
            "Replay requires the exact recorded package version and inventory digest",
        ));
    }
    let client = WorkerClient::new(sdk, worker, options).map_err(|_| {
        failure(
            "INVALID_INPUT",
            "Replay requires an explicit worker inside the selected SDK",
        )
    })?;
    let identity: bgx_adapter_v2_ports::wire::WorkerEnvelopeV2Branch1Package =
        serde_json::from_value(encoded(package.verified().identity())?).map_err(|_| {
            failure(
                "INTERNAL_ERROR",
                "Cannot encode the verified worker package identity",
            )
        })?;
    let mut invocations = Vec::new();
    for item in &file.transcript.invocations {
        let state = Arc::new(Mutex::new(ReplayState {
            next: 0,
            mismatch: None,
        }));
        let input = encoded(&item.invocation)?;
        let ms = input["budgetMs"].as_u64().unwrap();
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(ms))
            .ok_or_else(|| {
                failure(
                    "INVALID_INPUT",
                    "Replay deadline is outside the native clock range",
                )
            })?;
        let result = client
            .invoke(
                package.root().to_owned(),
                identity.clone(),
                item.invocation.clone(),
                Box::new(TapeHost {
                    ports: item.ports.clone(),
                    state: state.clone(),
                }),
                Control::new(Some(deadline)),
            )
            .await;
        client.wait_idle().await;
        let state = state
            .lock()
            .map_err(|_| failure("INTERNAL_ERROR", "Replay tape lock failed"))?;
        if let Some(reason) = state.mismatch {
            return Err(failure("REPLAY_MISMATCH", reason));
        }
        let receipt = result.map_err(|e| {
            failure(
                "REPLAY_WORKER_FAILED",
                format!("Offline worker failed: {:?}", e.kind),
            )
        })?;
        if state.next != item.ports.len() {
            return Err(failure(
                "REPLAY_MISMATCH",
                "Guest omitted recorded port calls",
            ));
        }
        if receipt.response != item.response {
            return Err(failure(
                "REPLAY_MISMATCH",
                "Actual guest Response differs from the recorded Response",
            ));
        }
        invocations.push(ReplayInvocation {
            request_id: item.invocation.r#request_id.clone(),
            worker_pid: receipt.worker_pid,
            port_calls: state.next,
            response_sha256: hash_bytes(&serde_json::to_vec(&receipt.response).unwrap()),
        });
    }
    Ok(ReplayReport {
        scope: Scope::Mock,
        passed: true,
        transcript_sha256: file.sha256.clone(),
        invocations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(payload: Value) -> Invocation {
        serde_json::from_value(json!({"protocolVersion":2,"operation":"probe","profileId":"binding","bindingId":"binding","capabilitySlot":"generic","requestId":"recording-test","budgetMs":1000,"negotiatedVersion":2,"enabledFeatures":[],"payloadJson":payload.to_string()})).unwrap()
    }
    fn recorder(secret: Option<&str>) -> Recorder {
        Recorder::new(
            PackageIdentity {
                plugin_id: "local.record-test".into(),
                version: "1.0.0".into(),
                digest: "a".repeat(64),
            },
            MockInputs {
                seed: 1,
                now_unix_ms: 0,
            },
            SnapshotPolicy::new(
                secret
                    .into_iter()
                    .map(|s| s.as_bytes().to_vec().into())
                    .collect(),
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn payload() -> Value {
        json!({"schemaVersion":2,"kind":"package_validation","configuration":{}})
    }
    #[test]
    fn decoded_json_secrets_and_signing_failure_are_sticky() {
        let secret = "known-quote\"and-backslash\\secret";
        let rec = recorder(Some(secret));
        let mut input = payload();
        input["configuration"] = json!({"value":secret});
        assert_eq!(
            rec.started(&request(input)).unwrap_err().code,
            "RECORD_DENIED"
        );
        assert_eq!(rec.snapshot().unwrap_err().code, "RECORD_DENIED");
        let rec = recorder(None);
        let input = request(payload());
        rec.started(&input).unwrap();
        let sign:HostPortCallV2=serde_json::from_value(json!({"method":"host-credential.sign","args":{"handle":"explicit-fake-handle","specJson":json!({"version":1,"kind":"hmac-sha256","message":"fixture","encoding":"hex"}).to_string()}})).unwrap();
        assert_eq!(
            rec.completed_port(&input, &sign, &Ok(json!("a".repeat(64))))
                .unwrap_err()
                .code,
            "RECORD_DENIED"
        );
        assert_eq!(rec.snapshot().unwrap_err().code, "RECORD_DENIED");
        let rec = recorder(Some(secret));
        let input = request(payload());
        rec.started(&input).unwrap();
        let output:Response=serde_json::from_value(json!({"protocolVersion":2,"operation":"probe","dataJson":json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[{"code":"RESOURCE_UNAVAILABLE","message":secret}]}).to_string()})).unwrap();
        response(&input, &output)
            .expect("Secret fixture must first satisfy the public probe result schema");
        assert_eq!(
            rec.finished(&input, &output).unwrap_err().code,
            "RECORD_DENIED"
        );
        assert_eq!(rec.snapshot().unwrap_err().code, "RECORD_DENIED");
    }
    #[test]
    fn recorder_refuses_incomplete_order_and_public_byte_budget_overflow() {
        let rec = recorder(None);
        let input = request(payload());
        rec.started(&input).unwrap();
        assert_eq!(rec.snapshot().unwrap_err().code, "INVALID_INPUT");
        let call:HostPortCallV2=serde_json::from_value(json!({"method":"host-state.put","args":{"key":"fixture","value":"x".repeat(limit()+1)}})).unwrap();
        assert_eq!(
            rec.completed_port(&input, &call, &Ok(Value::Null))
                .unwrap_err()
                .code,
            "BUDGET_EXCEEDED"
        );
        assert_eq!(rec.snapshot().unwrap_err().code, "RECORD_DENIED");
    }
}
