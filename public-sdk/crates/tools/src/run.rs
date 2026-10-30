//! Thin six-operation CLI orchestration over the reference host and public wire.
use crate::{
    transcript::{self, MockInputs, Recorder},
    workspace::{self, Config},
};
use bgx_adapter_v2_ports::wire::{
    host::HostError, HostPortCallV2, Invocation, Response, StreamEventV1,
};
use bgx_adapter_v2_reference_host::{
    checkpoint::{CheckpointError, SnapshotPolicy},
    dispatch::{EventFuture, EventSink, PortObservationFuture, PortObserver},
    media::DevelopmentWorkspace,
    ReferenceHost,
};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{
    capabilities::{Binding, CapabilityRef},
    hash_bytes, SchemaRegistry,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Arguments {
    operation: String,
    paths: BTreeMap<String, PathBuf>,
    root: Option<String>,
    create: bool,
    host_checkpoints: bool,
    host_channel: bool,
}
fn observation_failure(error: crate::Error) -> HostError {
    // The exact development error remains sticky in Recorder. The managed port
    // uses only its public error vocabulary.
    let code = match error.code {
        "BUDGET_EXCEEDED" => "BUDGET_EXCEEDED",
        "INVALID_INPUT" => "INVALID_INPUT",
        "INTERNAL_ERROR" => "INTERNAL_ERROR",
        _ => "PERMISSION_DENIED",
    };
    serde_json::from_value(json!({"code":code,"message":"Development recording refused this interaction; inspect the recording diagnostic"})).expect("public host error")
}
struct RecordingObserver(Arc<Recorder>);
impl PortObserver for RecordingObserver {
    fn started<'a>(&'a self, invocation: &'a Invocation) -> PortObservationFuture<'a> {
        Box::pin(async move { self.0.started(invocation).map_err(observation_failure) })
    }
    fn completed_port<'a>(
        &'a self,
        invocation: &'a Invocation,
        call: &'a HostPortCallV2,
        result: &'a std::result::Result<Value, HostError>,
    ) -> PortObservationFuture<'a> {
        Box::pin(async move {
            self.0
                .completed_port(invocation, call, result)
                .map_err(observation_failure)
        })
    }
    fn finished<'a>(
        &'a self,
        invocation: &'a Invocation,
        response: &'a Response,
    ) -> PortObservationFuture<'a> {
        Box::pin(async move {
            self.0
                .finished(invocation, response)
                .map_err(observation_failure)
        })
    }
}
fn argument(path: impl Into<String>, message: impl Into<String>) -> crate::Error {
    let mut failure = workspace::invalid(path, message);
    failure.code = "INVALID_ARGUMENT";
    failure.exit = 2;
    failure
}
impl Arguments {
    fn parse(args: &[String]) -> crate::Result<Self> {
        let operation = match args.first().map(String::as_str) {
            Some("caps" | "capabilities") => "capabilities",
            Some(op @ ("probe" | "upload" | "submit" | "query" | "cancel")) => op,
            _ => {
                return Err(argument(
                    "command",
                    "Select capabilities, probe, upload, submit, query or cancel",
                ))
            }
        }
        .to_owned();
        let mut parsed = Self {
            operation,
            paths: BTreeMap::new(),
            root: None,
            create: false,
            host_checkpoints: false,
            host_channel: false,
        };
        let mut index = 1;
        while index < args.len() {
            let flag = &args[index];
            if flag == "--host-channel-v1" {
                if parsed.host_channel { return Err(argument(flag, "Duplicate option")); }
                parsed.host_channel = true;
                index += 1;
                continue;
            }
            if flag == "--host-checkpoints" {
                if parsed.host_checkpoints {
                    return Err(argument(flag, "Duplicate option"));
                }
                parsed.host_checkpoints = true;
                index += 1;
                continue;
            }
            if flag == "--create-workspace" {
                if parsed.create {
                    return Err(argument(flag, "Duplicate option"));
                }
                parsed.create = true;
                index += 1;
                continue;
            }
            if ![
                "--sdk",
                "--worker",
                "--package",
                "--workspace",
                "--config",
                "--input",
                "--output",
                "--request-id",
            ]
            .contains(&flag.as_str())
            {
                return Err(argument("arguments", "Unknown run option"));
            }
            let value = args
                .get(index + 1)
                .filter(|v| !v.is_empty() && !v.starts_with("--"))
                .ok_or_else(|| argument(flag, "Option requires a value"))?;
            if flag == "--request-id" {
                if parsed.root.replace(value.clone()).is_some() {
                    return Err(argument(flag, "Duplicate option"));
                }
            } else {
                let path = PathBuf::from(value);
                workspace::absolute(&path, flag).map_err(|mut e| {
                    e.code = "INVALID_ARGUMENT";
                    e.exit = 2;
                    e
                })?;
                if parsed.paths.insert(flag.clone(), path).is_some() {
                    return Err(argument(flag, "Duplicate option"));
                }
            }
            index += 2;
        }
        for flag in [
            "--sdk",
            "--worker",
            "--package",
            "--workspace",
            "--config",
            "--input",
            "--output",
        ] {
            if !parsed.paths.contains_key(flag) {
                return Err(argument(flag, "Required explicit run option is missing"));
            }
        }
        if matches!(parsed.operation.as_str(), "query" | "cancel") != parsed.root.is_some() {
            return Err(argument(
                "--request-id",
                "Only query and cancel require the original checkpoint request identifier",
            ));
        }
        if parsed.create && parsed.root.is_some() {
            return Err(argument(
                "--create-workspace",
                "Query and cancel must reopen an existing development workspace",
            ));
        }
        Ok(parsed)
    }
    fn path(&self, name: &str) -> &Path {
        &self.paths[name]
    }
}

fn host_error(e: CheckpointError) -> crate::Error {
    static RESPONSE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let schema = RESPONSE.get_or_init(|| {
        serde_json::from_str(include_str!("../../../schemas/response.schema.json"))
            .expect("public response schema")
    });
    let public = schema["oneOf"][1]["properties"]["error"]["properties"]["code"]["enum"]
        .as_array()
        .expect("public error codes")
        .iter()
        .filter_map(Value::as_str)
        .find(|code| *code == e.code);
    let code = public.unwrap_or(match e.code.as_str() {
        "BUDGET_EXCEEDED" => "BUDGET_EXCEEDED",
        "CANCELLED" => "CANCELLED",
        "CONFLICT" => "CONFLICT",
        _ => "HOST_ERROR",
    });
    let message = if code == "HOST_ERROR" {
        format!("{}: {}", e.code, e.message)
    } else {
        e.message.to_owned()
    };
    crate::error(code,"run",message,"Inspect the explicit development workspace diagnostics; no request was automatically retried.")
}
fn wire_failure(operation: &str, e: &crate::Error) -> Option<Value> {
    let response = json!({"protocolVersion":2,"operation":operation,"error":{"code":e.code,"message":e.message,"retryable":false,"vendorCode":null,"vendorRequestId":null}});
    SchemaRegistry::validate_public("response", &response)
        .ok()
        .map(|_| response)
}
fn check_response(policy: &SnapshotPolicy, response: &Value) -> crate::Result<()> {
    policy.check(response).map_err(host_error)?;
    if let Some(data) = response["dataJson"].as_str() {
        let decoded: Value = serde_json::from_str(data)
            .map_err(|_| workspace::invalid("dataJson", "Validated response data is not JSON"))?;
        policy.check(&decoded).map_err(host_error)?;
    }
    Ok(())
}
fn diagnostic_directory(
    workspace: &DevelopmentWorkspace,
    request_id: &str,
) -> crate::Result<PathBuf> {
    let parent = workspace.root().join("cli");
    match fs::create_dir(&parent) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => {
            return Err(workspace::invalid(
                "workspace",
                "Cannot create CLI diagnostic directory",
            ))
        }
    }
    if parent.canonicalize().ok().as_deref().and_then(Path::parent) != Some(workspace.root()) {
        return Err(workspace::invalid(
            "workspace",
            "CLI diagnostic directory escapes the selected workspace",
        ));
    }
    let path = parent.join(hash_bytes(request_id.as_bytes()));
    fs::create_dir(&path).map_err(|_| workspace::invalid("requestId","A CLI request identifier must be fresh; use query with the original checkpoint to resume"))?;
    Ok(path)
}
struct Sink {
    file: Arc<Mutex<File>>,
    policy: Arc<SnapshotPolicy>,
}
impl EventSink for Sink {
    fn consume<'a>(&'a self, event: &'a StreamEventV1) -> EventFuture<'a> {
        Box::pin(async move {
            let event = serde_json::to_value(event).expect("standard event");
            let policy = self.policy.clone();
            let file = self.file.clone();
            tokio::task::spawn_blocking(move || {
                policy.check(&event).map_err(|_| sink_error())?;
                let mut file = file.lock().map_err(|_| sink_error())?;
                serde_json::to_writer(&mut *file, &event).map_err(|_| sink_error())?;
                file.write_all(b"\n")
                    .and_then(|_| file.sync_all())
                    .map_err(|_| sink_error())
            })
            .await
            .map_err(|_| sink_error())?
        })
    }
}
fn sink_error() -> bgx_adapter_v2_ports::wire::host::HostError {
    serde_json::from_value(json!({"code":"RESOURCE_UNAVAILABLE","message":"Cannot persist the standard development event"})).expect("host error")
}
fn control(invocation: &Value) -> crate::Result<Control> {
    let budget = invocation["budgetMs"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| {
            workspace::invalid(
                "budgetMs",
                "Invocation must provide a positive public execution budget",
            )
        })?;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(budget))
        .ok_or_else(|| {
            workspace::invalid(
                "budgetMs",
                "Invocation deadline exceeds the native clock range",
            )
        })?;
    Ok(Control::new(Some(deadline)))
}

async fn provision_media(
    host: &ReferenceHost,
    config: &Config,
    invocation: &mut Value,
    payload: &mut Value,
) -> crate::Result<()> {
    let mut used = BTreeSet::new();
    let mut media: Vec<&mut Value> = match invocation["operation"].as_str() {
        Some("upload") => vec![&mut payload["media"]],
        Some("submit") => payload["media"]
            .as_array_mut()
            .ok_or_else(|| {
                workspace::invalid("media", "Submit must declare the standard media array")
            })?
            .iter_mut()
            .collect(),
        _ => Vec::new(),
    };
    for m in &mut media {
        let id = m["id"].as_str().ok_or_else(|| {
            workspace::invalid(
                "media.id",
                "Each media input must declare its public identifier",
            )
        })?;
        let source = config
            .media
            .iter()
            .find(|source| source.id == id)
            .ok_or_else(|| {
                workspace::invalid(
                    "config.media",
                    "Every media input requires an explicit development source",
                )
            })?;
        if m["kind"] != source.kind
            || m["handle"].as_str().is_none_or(str::is_empty)
            || !used.insert(id.to_owned())
        {
            return Err(workspace::invalid(
                "media",
                "Media identifier, kind and nonempty handle must match the explicit source",
            ));
        }
        let lease = host
            .import_media(
                invocation["requestId"].as_str().unwrap(),
                id,
                &source.path,
                &source.kind,
                control(invocation)?,
            )
            .await
            .map_err(host_error)?;
        // Only the explicitly provisioned lease changes. All other public fields
        // are checked against broker-measured facts by the shared validator.
        m["handle"] = json!(lease.handle);
    }
    if used.len() != config.media.len() {
        return Err(workspace::invalid(
            "config.media",
            "Unused media provisioning is forbidden for this operation",
        ));
    }
    if !used.is_empty() {
        invocation["payloadJson"] = json!(payload.to_string());
    }
    Ok(())
}
async fn dispatch(
    host: &ReferenceHost,
    config: &Config,
    args: &Arguments,
    mut invocation: Value,
    workspace: &DevelopmentWorkspace,
    directory: &Path,
    policy: Arc<SnapshotPolicy>,
    observer: Option<Arc<dyn PortObserver>>,
) -> crate::Result<Value> {
    let mut payload: Value = serde_json::from_str(invocation["payloadJson"].as_str().unwrap())
        .map_err(|_| workspace::invalid("payloadJson", "Invocation payload must contain JSON"))?;
    SchemaRegistry::public()
        .and_then(|r| r.validate_dispatch(&args.operation, "input", &payload))
        .map_err(|_| {
            workspace::invalid(
                "payloadJson",
                "Invocation payload violates the published operation schema",
            )
        })?;
    policy.check(&payload).map_err(host_error)?;
    let discover = matches!(args.operation.as_str(), "submit" | "upload")
        || (args.operation == "probe" && payload["kind"] == "connectivity");
    let mut reference: Option<CapabilityRef> = None;
    if discover {
        let mut discovery = invocation.clone();
        discovery["operation"] = json!("capabilities");
        discovery["requestId"] = json!(format!(
            "sdk-discovery-{}",
            hash_bytes(invocation["requestId"].as_str().unwrap().as_bytes())
        ));
        discovery["capabilitySlot"] = json!("generic");
        discovery["payloadJson"] = json!("{}");
        // Discovery is prerequisite catalog negotiation, never a retry of the operation.
        let receipt = host
            .invoke_observed(
                serde_json::from_value(discovery.clone()).unwrap(),
                None,
                None,
                control(&discovery)?,
                None,
                observer.clone(),
            )
            .await
            .map_err(host_error)?;
        let response = serde_json::to_value(receipt.response).unwrap();
        check_response(&policy, &response)?;
        workspace::atomic_json(
            workspace,
            &directory.join("discovery.response.json"),
            &response,
        )?;
        if response.get("error").is_some() {
            return Err(workspace::invalid(
                "capabilities",
                "Capability discovery failed; inspect discovery.response.json",
            ));
        }
        if args.operation != "probe" {
            let mut matches = host
                .catalog()
                .map_err(host_error)?
                .into_iter()
                .filter(|view| {
                    let r = &view.reference;
                    payload["modelId"] == r.model_id
                        && payload["capabilityKey"] == r.capability_key
                        && payload["operationId"] == r.operation_id
                        && payload["operationVersion"] == r.operation_version
                });
            reference = Some(
                matches
                    .next()
                    .ok_or_else(|| {
                        workspace::invalid(
                            "capability",
                            "Input does not select a discovered operation",
                        )
                    })?
                    .reference,
            );
            if matches.next().is_some() {
                return Err(workspace::invalid(
                    "capability",
                    "Input operation selection is ambiguous",
                ));
            }
        }
    }
    provision_media(host, config, &mut invocation, &mut payload).await?;
    policy.check(&invocation).map_err(host_error)?;
    workspace::atomic_json(
        workspace,
        &directory.join("execution.invocation.json"),
        &invocation,
    )?;
    workspace::atomic_json(
        workspace,
        &directory.join("execution.identity.json"),
        &json!({"payloadSha256":hash_bytes(invocation["payloadJson"].as_str().unwrap().as_bytes())}),
    )?;
    let events = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join("events.jsonl"))
        .map_err(|_| {
            workspace::invalid("events", "Cannot create the explicit development event log")
        })?;
    let sink = Arc::new(Sink {
        file: Arc::new(Mutex::new(events)),
        policy,
    });
    let ctl = control(&invocation)?;
    let invocation: Invocation = serde_json::from_value(invocation).map_err(|_| {
        workspace::invalid(
            "input",
            "Invocation cannot be decoded by the public wire types",
        )
    })?;
    let receipt = host
        .invoke_observed(
            invocation,
            reference,
            args.root.clone(),
            ctl,
            Some(sink),
            observer,
        )
        .await
        .map_err(host_error)?;
    Ok(serde_json::to_value(receipt.response).expect("standard response"))
}
pub async fn execute(args: &[String]) -> crate::Result<Value> {
    execute_inner(args, None).await
}
pub async fn execute_recorded(
    args: &[String],
    transcript_path: &Path,
    mock: MockInputs,
) -> crate::Result<Value> {
    execute_inner(args, Some((transcript_path, mock))).await
}
fn output_target(path: &Path) -> crate::Result<PathBuf> {
    let parent = path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| {
            workspace::invalid(
                "transcript",
                "Recording file parent directory is unavailable",
            )
        })?;
    Ok(parent.join(
        path.file_name()
            .ok_or_else(|| workspace::invalid("transcript", "Recording path must name a file"))?,
    ))
}
async fn execute_inner(
    args: &[String],
    recording: Option<(&Path, MockInputs)>,
) -> crate::Result<Value> {
    let args = Arguments::parse(args)?;
    let config = Config::load(args.path("--config"))?;
    let workspace = workspace::open(args.path("--workspace"), args.create)?;
    workspace::contained_output(&workspace, args.path("--output"))?;
    for name in ["--input", "--config"] {
        if args.path("--output") == args.path(name)
            || args
                .path("--output")
                .canonicalize()
                .ok()
                .is_some_and(|output| args.path(name).canonicalize().ok().as_ref() == Some(&output))
        {
            return Err(workspace::invalid(
                "--output",
                "Receipt cannot overwrite an input or configuration file",
            ));
        }
    }
    if let Some((path, _)) = &recording {
        workspace::contained_output(&workspace, path)?;
        for flag in ["--input", "--config", "--output"] {
            if output_target(path)? == output_target(args.path(flag))? {
                return Err(workspace::invalid(
                    "transcript",
                    "Recording cannot overwrite a request, configuration or response file",
                ));
            }
        }
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(crate::error(
                    "NO_OVERWRITE",
                    "transcript",
                    "Recording output already exists",
                    "Select a new transcript file; the existing recording will not be replaced.",
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(workspace::invalid(
                    "transcript",
                    "Cannot inspect the explicit recording output path",
                ))
            }
        }
    }
    let raw_input = workspace::read(args.path("--input"), "maxFrameBytes")?;
    let input: Value = serde_json::from_slice(&raw_input).map_err(|_| {
        workspace::invalid(
            "input",
            "Input file must contain a standard Invocation JSON object",
        )
    })?;
    SchemaRegistry::validate_public("invocation", &input).map_err(|_| {
        workspace::invalid("input", "Input violates the published Invocation schema")
    })?;
    if input["operation"] != args.operation {
        return Err(workspace::invalid(
            "operation",
            "CLI operation differs from the standard Invocation operation",
        ));
    }
    let provision = config.provision(
        workspace.clone(),
        args.path("--sdk").to_owned(),
        args.path("--worker").to_owned(),
    )?;
    let policy = Arc::new(provision.snapshot_policy().map_err(host_error)?);
    policy.check(&input).map_err(host_error)?;
    let decoded_payload: Value = serde_json::from_str(input["payloadJson"].as_str().unwrap())
        .map_err(|_| workspace::invalid("payloadJson", "Invocation payload must contain JSON"))?;
    policy.check(&decoded_payload).map_err(host_error)?;
    if recording.is_some() {
        transcript::check_nonsecret(&policy, &input)?;
    }
    workspace::plain_path(args.path("--package"))?;
    let package = crate::package::load_checked(args.path("--package"), &config.trust())?;
    let recorder = recording
        .as_ref()
        .map(|(_, mock)| {
            Recorder::new(
                package.verified().identity().clone(),
                mock.clone(),
                provision.snapshot_policy().map_err(host_error)?,
            )
            .map(Arc::new)
        })
        .transpose()?;
    let observer: Option<Arc<dyn PortObserver>> = recorder
        .as_ref()
        .map(|r| Arc::new(RecordingObserver(r.clone())) as Arc<dyn PortObserver>);
    let binding = Binding::new(
        config.binding_id.clone(),
        package.verified(),
        config.binding_revision,
        config.configuration.clone(),
    )
    .map_err(|_| {
        workspace::invalid(
            "binding",
            "Explicit binding is invalid for the selected package",
        )
    })?;
    let host = if args.host_checkpoints {
        ReferenceHost::with_checkpoints(
            provision,
            package,
            binding,
            Box::new(crate::host_rows::StdioRows),
        )
    } else {
        ReferenceHost::new(provision, package, binding)
    }
    .map_err(host_error)?;
    let directory = diagnostic_directory(&workspace, input["requestId"].as_str().unwrap())?;
    let auditor = crate::host_channel::AuditChannel::open(&directory, args.host_channel).await?;
    let host = host.with_before_send(Arc::new(auditor));
    workspace::atomic_json(&workspace, &directory.join("input.invocation.json"), &input)?;
    workspace::atomic_json(
        &workspace,
        &directory.join("input.identity.json"),
        &json!({"requestSha256":hash_bytes(&raw_input),"payloadSha256":hash_bytes(input["payloadJson"].as_str().unwrap().as_bytes())}),
    )?;
    let result = dispatch(
        &host,
        &config,
        &args,
        input,
        &workspace,
        &directory,
        policy.clone(),
        observer,
    )
    .await;
    let cleanup = host.wait_idle().await.map_err(host_error);
    let result = match (result, cleanup) {
        (Ok(response), Ok(())) => Ok(response),
        (Err(e), Ok(())) | (_, Err(e)) => Err(e),
    };
    let response = match result {
        Ok(response) => response,
        Err(e) => {
            let diagnostic = json!({"code":e.code,"message":e.message});
            policy.check(&diagnostic).map_err(host_error)?;
            workspace::atomic_json(&workspace, &directory.join("host.error.json"), &diagnostic)?;
            match wire_failure(&args.operation, &e) {
                Some(response) => response,
                None => return Err(e),
            }
        }
    };
    check_response(&policy, &response)?;
    if let (Some(recorder), Some((path, _))) = (&recorder, &recording) {
        if let Err(mut error) = recorder.save(path) {
            error.hint="Recording failed. Any accepted remote task remains in the development checkpoint; inspect diagnostics and query it without resubmitting.";
            let diagnostic = json!({"code":error.code,"message":error.message});
            transcript::check_nonsecret(&policy, &diagnostic)?;
            workspace::atomic_json(
                &workspace,
                &directory.join("recording.error.json"),
                &diagnostic,
            )?;
            return Err(error);
        }
    }
    workspace::atomic_json(&workspace, args.path("--output"), &response)?;
    workspace::atomic_json(&workspace, &directory.join("response.json"), &response)?;
    Ok(response)
}
