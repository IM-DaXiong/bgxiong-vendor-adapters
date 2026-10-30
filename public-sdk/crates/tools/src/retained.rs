//! Offline consumption of SDK-confirmed media. No guest, HTTP broker or secret lease.
use crate::{error, package, workspace, Args, Result};
use bgx_adapter_v2_ports::{
    decimal_u64, resource_policy,
    wire::{Invocation, MediaInputV2, OperationSubmitV2, Response},
};
use bgx_adapter_v2_reference_host::{
    checkpoint::{CheckpointRows, CheckpointStore, SnapshotPolicy, SqliteFixtureRows},
    media::{DevelopmentWorkspace, LeaseScope, LeasedMedia, MediaBroker},
    session::Session,
};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{
    capabilities::Binding, hash_bytes, usage::UsageSnapshot, validate_response, SchemaRegistry,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const HINT: &str = "Keep the original package, checkpoint and confirmed SDK workspace; never resubmit to recover retained media.";
fn invalid(message: impl Into<String>) -> crate::Error {
    error("INVALID_RETAINED_RESULT", "/retained", message, HINT)
}
fn native(e: impl std::fmt::Display) -> crate::Error {
    invalid(e.to_string())
}
fn budget(key: &str) -> usize {
    resource_policy()["limits"][key]
        .as_u64()
        .unwrap()
        .try_into()
        .unwrap()
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| invalid(format!("Missing string {key}")))
}

/// A tool can read the host's existing opaque rows, but cannot acquire write authority.
struct ReadRows(Box<dyn CheckpointRows>);
impl CheckpointRows for ReadRows {
    fn get(
        &self,
        id: &str,
        max: usize,
    ) -> bgx_adapter_v2_reference_host::checkpoint::Result<Option<(String, Vec<u8>, Vec<u8>)>> {
        self.0.get(id, max)
    }
    fn insert(
        &mut self,
        _: &str,
        _: &[u8],
        _: &[u8],
    ) -> bgx_adapter_v2_reference_host::checkpoint::Result<bool> {
        Err(read_only())
    }
    fn swap(
        &mut self,
        _: &str,
        _: &str,
        _: &[u8],
        _: &str,
        _: &[u8],
    ) -> bgx_adapter_v2_reference_host::checkpoint::Result<bool> {
        Err(read_only())
    }
}
fn read_only() -> bgx_adapter_v2_reference_host::checkpoint::CheckpointError {
    bgx_adapter_v2_reference_host::checkpoint::CheckpointError {
        code: "PERMISSION_DENIED".into(),
        message: "Retained media tools cannot write checkpoints",
    }
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&workspace::read(path, "maxFrameBytes")?)
        .map_err(|_| invalid("Retained JSON is malformed"))
}
fn existing(root: &Path, path: &Path) -> Result<PathBuf> {
    workspace::plain_path(path)?;
    let canonical = path.canonicalize().map_err(native)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(invalid("Retained file escapes its authorized workspace"));
    }
    Ok(canonical)
}

#[derive(Clone)]
struct Output {
    role: String,
    media: MediaInputV2,
}
#[derive(Default)]
struct Outputs {
    items: Vec<Output>,
    facts: BTreeMap<String, Value>,
    seen: BTreeSet<(String, String)>,
}
impl Outputs {
    fn observe(&mut self, role: &str, media: &Value) -> Result<()> {
        SchemaRegistry::validate_public("media-input", media).map_err(native)?;
        let descriptor: MediaInputV2 = serde_json::from_value(media.clone()).map_err(native)?;
        let mut facts = media.clone();
        facts.as_object_mut().unwrap().remove("handle");
        if self
            .facts
            .get(&descriptor.r#id)
            .is_some_and(|previous| previous != &facts)
        {
            return Err(invalid("One media id has conflicting durable facts"));
        }
        self.facts.insert(descriptor.r#id.clone(), facts);
        if self.seen.insert((descriptor.r#id.clone(), role.into())) {
            self.items.push(Output {
                role: role.into(),
                media: descriptor,
            });
        }
        Ok(())
    }
}

struct Snapshot {
    invocation: Invocation,
    response: Option<Value>,
    context: bgx_adapter_v2_validator::ValidatedOperationInput,
    sequence: u64,
    journal_digest: String,
    pending_tail: bool,
    usage: UsageSnapshot,
    outputs: Outputs,
}
fn snapshot(
    work: &DevelopmentWorkspace,
    id: &str,
    binding: &Binding,
    session: &Session,
    namespace: &str,
) -> Result<Snapshot> {
    let directory = work.root().join("cli").join(hash_bytes(id.as_bytes()));
    let invocation_value = read_json(&existing(
        work.root(),
        &directory.join("execution.invocation.json"),
    )?)?;
    SchemaRegistry::validate_public("invocation", &invocation_value).map_err(native)?;
    if invocation_value["requestId"] != id
        || invocation_value["bindingId"] != binding.id()
        || invocation_value["operation"] != "submit"
    {
        return Err(invalid(
            "Retained invocation is not the authorized root submission",
        ));
    }
    let identity = read_json(&existing(
        work.root(),
        &directory.join("execution.identity.json"),
    )?)?;
    let payload = text(&invocation_value, "payloadJson")?;
    if identity.as_object().is_none_or(|o| o.len() != 1)
        || identity["payloadSha256"] != hash_bytes(payload.as_bytes())
    {
        return Err(invalid("Retained input digest differs"));
    }
    let submission: OperationSubmitV2 = serde_json::from_str(payload).map_err(native)?;
    let context = session
        .retained_submission_context(&submission)
        .map_err(native)?;
    // Only the original submit invocation is addressed; no query child is guessed.
    let invocation: Invocation = serde_json::from_value(invocation_value).map_err(native)?;
    let response_path = directory.join("response.json");
    let response = if response_path.try_exists().map_err(native)? {
        Some(read_json(&existing(work.root(), &response_path)?)?)
    } else {
        None
    };
    if let Some(raw) = &response {
        let response: Response = serde_json::from_value(raw.clone()).map_err(native)?;
        validate_response(&invocation, &response, Some(&context)).map_err(native)?;
    }
    let journal = existing(work.root(), &directory.join("events.jsonl"))?;
    let mut reader = BufReader::new(File::open(journal).map_err(native)?);
    let mut result = Snapshot {
        invocation,
        response,
        context,
        sequence: 0,
        journal_digest: String::new(),
        pending_tail: false,
        usage: UsageSnapshot::default(),
        outputs: Outputs::default(),
    };
    let mut hasher = Sha256::new();
    loop {
        let mut line = Vec::new();
        let read = reader
            .by_ref()
            .take((budget("maxFrameBytes") + 2) as u64)
            .read_until(b'\n', &mut line)
            .map_err(native)?;
        if read == 0 {
            break;
        }
        if read > budget("maxFrameBytes") + usize::from(line.last() == Some(&b'\n')) {
            return Err(invalid("Retained event exceeds the published frame budget"));
        }
        if line.last() != Some(&b'\n') {
            if result.response.is_some() {
                return Err(invalid("Terminal journal has an incomplete event"));
            }
            result.pending_tail = true;
            break;
        }
        let event: Value =
            serde_json::from_slice(&line).map_err(|_| invalid("Retained event is malformed"))?;
        SchemaRegistry::validate_public("stream-event", &event).map_err(native)?;
        let sequence = event["sequence"]
            .as_u64()
            .ok_or_else(|| invalid("Event sequence exceeds u64"))?;
        if event["requestId"] != id || sequence <= result.sequence {
            return Err(invalid("Event request or monotonic sequence differs"));
        }
        match text(&event, "type")? {
            "media_chunk" => result
                .outputs
                .observe(text(&event["payload"], "role")?, &event["payload"]["media"])?,
            "usage" => result
                .usage
                .replace(&event["payload"], namespace)
                .map_err(native)?,
            "progress" | "text_delta" => {}
            _ => return Err(invalid("Unknown retained event")),
        }
        result.sequence = sequence;
        hasher.update(&line);
    }
    result.journal_digest = hex::encode(hasher.finalize());
    if let Some(response) = &result.response {
        if let Some(raw) = response["dataJson"].as_str() {
            let data: Value = serde_json::from_str(raw).map_err(native)?;
            if let Some(outputs) = data["outputs"].as_array() {
                for output in outputs {
                    if output["kind"] == "media" {
                        result
                            .outputs
                            .observe(text(output, "role")?, &output["media"])?;
                    }
                }
            }
            if let Some(usage) = data.get("usage") {
                result.usage.replace(usage, namespace).map_err(native)?;
            }
        }
    }
    Ok(result)
}

fn new_destination(work: &DevelopmentWorkspace, package: &Path, path: &Path) -> Result<PathBuf> {
    workspace::plain_path(path)?;
    if path.try_exists().map_err(native)? {
        return Err(invalid(
            "Export destination must be a new exclusive directory",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Export destination needs a parent"))?
        .canonicalize()
        .map_err(native)?;
    let package = package.canonicalize().map_err(native)?;
    if parent.starts_with(&package) {
        return Err(invalid("Export cannot modify its source package"));
    }
    for protected in ["objects", "grants", "cli", "staging"] {
        if parent.starts_with(work.root().join(protected)) {
            return Err(invalid("Export cannot modify retained source directories"));
        }
    }
    let name = path
        .file_name()
        .ok_or_else(|| invalid("Export destination needs a basename"))?;
    let destination = parent.join(name);
    fs::create_dir(&destination).map_err(native)?;
    let canonical = destination.canonicalize().map_err(native)?;
    if canonical.parent() != Some(parent.as_path()) {
        return Err(invalid("Export destination was redirected"));
    }
    Ok(canonical)
}
fn export_one(
    broker: &mut MediaBroker,
    scope: &LeaseScope,
    lease: &LeasedMedia,
    destination: &Path,
    index: usize,
    control: &Control,
) -> Result<String> {
    let extension = Path::new(&lease.file_name)
        .extension()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid("Measured media has no real extension"))?;
    let name = format!("{index:06}.{extension}");
    let mut staged = tempfile::NamedTempFile::new_in(destination).map_err(native)?;
    let length = decimal_u64(&lease.stat.r#byte_length).map_err(native)?;
    let mut offset = 0u64;
    let mut hasher = Sha256::new();
    while offset < length {
        let bytes = broker
            .read(
                scope,
                &lease.handle,
                offset,
                budget("maxMediaChunkBytes"),
                control,
            )
            .map_err(|e| invalid(e.r#message))?;
        if bytes.is_empty() || bytes.len() as u64 > length - offset {
            return Err(invalid(
                "Retained media ended at a different measured length",
            ));
        }
        staged.write_all(&bytes).map_err(native)?;
        hasher.update(&bytes);
        offset = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| invalid("Media offset overflow"))?;
    }
    if hex::encode(hasher.finalize()) != lease.identity.r#digest {
        return Err(invalid("Export digest differs from durable identity"));
    }
    staged.as_file().sync_all().map_err(native)?;
    staged
        .persist_noclobber(destination.join(&name))
        .map_err(native)?;
    Ok(name)
}
fn write_receipt(work: &DevelopmentWorkspace, path: &Path, receipt: &Value) -> Result<()> {
    workspace::contained_output(work, path)?;
    let bytes = serde_json::to_vec(receipt).map_err(native)?;
    if bytes.len() > budget("maxFrameBytes") {
        return Err(invalid("Tool receipt exceeds the published frame budget"));
    }
    let mut staged = tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(native)?;
    staged
        .write_all(&bytes)
        .and_then(|_| staged.write_all(b"\n"))
        .and_then(|_| staged.as_file().sync_all())
        .map_err(native)?;
    staged.persist_noclobber(path).map_err(native)?;
    Ok(())
}

pub fn execute(args: &[String]) -> Result<Value> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(invalid("retained requires info, inspect or export"));
    };
    if command == "info" {
        Args::parse(&args[1..], &[], &[])?;
        return Ok(
            json!({"toolVersion":env!("CARGO_PKG_VERSION"),"protocolVersion":2,"toolStatus":"complete","commands":["inspect","export"],"scope":"root-submission","network":"none","maxMediaChunkBytes":budget("maxMediaChunkBytes")}),
        );
    }
    if !matches!(command, "inspect" | "export") {
        return Err(invalid("Unknown retained command"));
    }
    let allowed = if command == "export" {
        vec![
            "--package",
            "--workspace",
            "--config",
            "--request-id",
            "--output",
            "--destination",
            "--selection",
        ]
    } else {
        vec![
            "--package",
            "--workspace",
            "--config",
            "--request-id",
            "--output",
        ]
    };
    let args = Args::parse(&args[1..], &allowed, &["--host-checkpoints"])?;
    let work = workspace::open(&args.path("--workspace")?, false)?;
    let output = args.path("--output")?;
    workspace::contained_output(&work, &output)?;
    if output.try_exists().map_err(native)? {
        return Err(invalid("Tool receipt already exists"));
    }
    let config = workspace::Config::load(&args.path("--config")?)?;
    let package_path = args.path("--package")?;
    let loaded = package::load_checked(&package_path, &config.trust())?;
    let package = loaded.verified();
    let binding = Binding::new(
        config.binding_id.clone(),
        package,
        config.binding_revision,
        config.configuration.clone(),
    )
    .map_err(native)?;
    let rows: Box<dyn CheckpointRows> = if args.flags.contains("--host-checkpoints") {
        Box::new(crate::host_rows::StdioRows)
    } else {
        Box::new(SqliteFixtureRows::open_read_only(&work).map_err(native)?)
    };
    let store = CheckpointStore::with_rows(
        Box::new(ReadRows(rows)),
        SnapshotPolicy::new(vec![]).map_err(native)?,
    );
    let id = args.value("--request-id")?;
    let session = Session::recover(&store, id, package, &binding, &config.configuration_ref)
        .map_err(native)?;
    let mut snapshot = snapshot(&work, id, &binding, &session, package.namespace())?;
    let checkpoint_result = &session.checkpoint()["result"];
    if !checkpoint_result.is_null() {
        SchemaRegistry::validate_public("result", checkpoint_result).map_err(native)?;
        bgx_adapter_v2_validator::result::validate_output_slots(
            &snapshot.context,
            checkpoint_result,
        )
        .map_err(native)?;
        if checkpoint_result.get("vendorTaskId").is_some()
            && checkpoint_result["vendorTaskId"] != session.checkpoint()["vendorTaskId"]
        {
            return Err(invalid(
                "Retained checkpoint result belongs to a different remote task",
            ));
        }
        if let Some(outputs) = checkpoint_result["outputs"].as_array() {
            for output in outputs {
                if output["kind"] == "media" {
                    let media = &output["media"];
                    let identity = session.checkpoint()["mediaIdentities"]
                        .as_array()
                        .and_then(|identities| {
                            identities
                                .iter()
                                .find(|identity| identity["id"] == media["id"])
                        })
                        .ok_or_else(|| {
                            invalid("Checkpoint output lacks an SDK-confirmed durable identity")
                        })?;
                    if identity["kind"] != media["kind"]
                        || !text(media, "fileName")?
                            .starts_with(&format!("{}.", text(identity, "digest")?))
                    {
                        return Err(invalid(
                            "Checkpoint output differs from its durable identity",
                        ));
                    }
                    snapshot.outputs.observe(text(output, "role")?, media)?;
                }
            }
        }
        if let Some(usage) = checkpoint_result.get("usage") {
            snapshot
                .usage
                .replace(usage, package.namespace())
                .map_err(native)?;
        }
    }
    let selection: Option<BTreeSet<String>> = if command == "export" {
        let ids: Vec<String> =
            serde_json::from_value(read_json(&args.path("--selection")?)?).map_err(native)?;
        let set: BTreeSet<_> = ids.iter().cloned().collect();
        if ids.len() != set.len()
            || ids.iter().any(String::is_empty)
            || set
                .iter()
                .any(|id| !snapshot.outputs.facts.contains_key(id))
        {
            return Err(invalid(
                "Selection must be a unique subset of this request's confirmed media ids",
            ));
        }
        Some(set)
    } else {
        None
    };
    let destination = if command == "export" {
        Some(new_destination(
            &work,
            &package_path,
            &args.path("--destination")?,
        )?)
    } else {
        None
    };
    let scope = LeaseScope {
        package: package.identity().clone(),
        binding_id: binding.id().into(),
        request_id: id.into(),
    };
    let control = Control::new(None);
    let mut broker = MediaBroker::new(work.clone(), Duration::from_secs(300), config.probe_tool()?)
        .map_err(|e| invalid(e.r#message))?;
    let mut media = Vec::new();
    let mut issues = Vec::new();
    let mut failed = false;
    for (index, item) in snapshot.outputs.items.iter().enumerate() {
        if selection
            .as_ref()
            .is_some_and(|ids| !ids.contains(&item.media.r#id))
        {
            continue;
        }
        let declared = snapshot.context.operation()["outputSlots"]
            .as_array()
            .is_some_and(|slots| {
                slots.iter().any(|slot| {
                    slot["role"] == item.role
                        && slot["kind"] == serde_json::to_value(&item.media.r#kind).unwrap()
                })
            });
        if !declared {
            failed = true;
            issues.push(json!({"code":"UNDECLARED_OUTPUT_ROLE","mediaId":item.media.r#id,"role":item.role,"message":"Confirmed media is retained but does not match a declared output slot"}));
        }
        let mut row = json!({"id":item.media.r#id,"role":item.role,"kind":serde_json::to_value(&item.media.r#kind).unwrap(),"mime":item.media.r#mime,"byteLength":item.media.r#byte_length,"declaredOutput":declared,"identity":null,"relativePath":null,"sha256":null,"error":null});
        match broker.rehydrate_descriptor(&scope, &item.media, &control) {
            Ok(lease) => {
                row["identity"] = serde_json::to_value(&lease.identity).unwrap();
                row["sha256"] = json!(lease.identity.r#digest);
                if let Some(destination) = &destination {
                    match export_one(&mut broker, &scope, &lease, destination, index, &control) {
                        Ok(path) => row["relativePath"] = json!(path),
                        Err(e) => {
                            failed = true;
                            row["error"] = json!({"code":e.code,"message":e.message});
                        }
                    }
                }
                if let Err(e) = broker.release(&scope, &lease.handle) {
                    failed = true;
                    issues.push(json!({"code":e.r#code,"mediaId":item.media.r#id,"role":item.role,"message":e.r#message}));
                }
            }
            Err(e) => {
                failed = true;
                row["error"] = json!({"code":e.r#code,"message":e.r#message});
            }
        }
        media.push(row);
    }
    if let Err(e) = broker.close() {
        failed = true;
        issues.push(json!({"code":e.r#code,"mediaId":null,"role":null,"message":e.r#message}));
    }
    let receipt = json!({"toolVersion":env!("CARGO_PKG_VERSION"),"protocolVersion":2,"toolStatus":if failed {"partial"} else {"complete"},
        "sourceIdentity":{"package":package.identity(),"bindingId":binding.id(),"requestId":id,"requestDigest":session.checkpoint()["requestDigest"],"inputSchemaDigest":snapshot.context.operation()["inputSchema"]["digest"],"invocationSha256":hash_bytes(&serde_json::to_vec(&snapshot.invocation).unwrap()),"confirmedJournalSha256":snapshot.journal_digest},
        "eventSequence":snapshot.sequence.to_string(),"pending":snapshot.response.is_none(),"pendingTail":snapshot.pending_tail,
        "checkpointState":session.checkpoint()["state"],"checkpointRevision":session.checkpoint()["revision"],"checkpointResult":checkpoint_result,
        "response":snapshot.response,"usage":snapshot.usage.current(),"media":media,"issues":issues,"destination":destination.map(|p|p.to_string_lossy().to_string())});
    write_receipt(&work, &output, &receipt)?;
    Ok(receipt)
}
