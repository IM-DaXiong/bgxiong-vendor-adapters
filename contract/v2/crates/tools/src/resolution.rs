//! Offline, pure transition preparation. The embedding host owns authorization
//! and the atomic commit of its task state and checkpoint.
use crate::{error, Args, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    checkpoint: Value,
    frozen: Value,
    expected_revision: String,
    evidence: Value,
    #[serde(default)]
    completion: Option<Completion>,
}

pub fn execute(args: &[String]) -> Result<Value> {
    let p = Args::parse(args, &["--input"], &[])?;
    let path = p.path("--input")?;
    let max = bgx_adapter_v2_ports::resource_policy()["limits"]["maxStateBytes"].as_u64().unwrap() * 3;
    let file = std::fs::File::open(&path).map_err(|_| error("RESOURCE_UNAVAILABLE", "/input", "Resolution input cannot be opened", "Provide the original checkpoint and reviewed evidence"))?;
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).map_err(|_| error("RESOURCE_UNAVAILABLE", "/input", "Resolution input cannot be read", "Check the explicit input path"))?;
    if bytes.len() as u64 > max {
        return Err(error("BUDGET_EXCEEDED", "/input", "Resolution input exceeds state byte budget", "Provide bounded original evidence"));
    }
    let input: Input = serde_json::from_slice(&bytes).map_err(|_| error("INVALID_INPUT", "/input", "Resolution input differs from the public input shape", "Use checkpoint, frozen, expectedRevision and evidence"))?;
    let checkpoint = if let Some(completion) = &input.completion {
        prepare_completion(&input, completion)?
    } else { bgx_adapter_v2_reference_host::resolution::prepare_operator_reviewed(
        &input.checkpoint, &input.frozen, &input.expected_revision, &input.evidence,
    ).map_err(|e| {
        let code = match e.code.as_str() {
            "CHECKPOINT_CONFLICT" => "CHECKPOINT_CONFLICT",
            "BUDGET_EXCEEDED" => "BUDGET_EXCEEDED",
            _ => "INVALID_INPUT",
        };
        error(code, "/checkpoint", e.message, "Reload the original task; never resubmit it")
    })? };
    Ok(json!({"expectedRevision":input.expected_revision,"checkpoint":checkpoint}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Completion {
    result: Value,
    media: Vec<CompletionMedia>,
    workspace: std::path::PathBuf,
    package: std::path::PathBuf,
    config: std::path::PathBuf,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompletionMedia {
    id: String, role: String, kind: String, mime: String,
    path: std::path::PathBuf, sha256: String, byte_length: String, file_name: String,
}
fn completion_error(message: impl std::fmt::Display) -> crate::Error {
    error("INVALID_RESOLUTION", "/completion", message.to_string(), "Preserve original evidence; never resubmit")
}
fn prepare_completion(input: &Input, completion: &Completion) -> Result<Value> {
    use bgx_adapter_v2_reference_host::{media::{DevelopmentWorkspace, MediaBroker, LeaseScope}, session::Session};
    use bgx_adapter_v2_validator::capabilities::Binding;
    use bgx_adapter_v2_transport::Control;
    use sha2::{Digest, Sha256};
    use std::io::Write;
    if input.evidence["outcome"] != "completed" || completion.result["kind"] != "completed"
        || completion.result["outputs"] != json!([]) {
        return Err(completion_error("Completion requires a terminal result with outputs supplied only by the media broker"));
    }
    let config = crate::workspace::Config::load(&completion.config)?;
    let loaded = crate::package::load_checked(&completion.package, &config.trust())?;
    let package = loaded.verified();
    let binding = Binding::new(config.binding_id.clone(), package, config.binding_revision, config.configuration.clone()).map_err(completion_error)?;
    let session = Session::recover_snapshot(input.checkpoint.clone(), input.frozen.clone(), package, &binding, &config.configuration_ref)
        .map_err(|e| completion_error(e.message))?;
    let work = DevelopmentWorkspace::open(&completion.workspace).map_err(|e| completion_error(e.r#message))?;
    let scope = LeaseScope { package: package.identity().clone(), binding_id: binding.id().into(),
        request_id: input.checkpoint["requestId"].as_str().ok_or_else(|| completion_error("Missing request identity"))?.into() };
    let control = Control::new(None);
    let mut broker = MediaBroker::new(work.clone(), std::time::Duration::from_secs(300), config.probe_tool()?)
        .map_err(|e| completion_error(e.r#message))?;
    let outcome = (|| {
        let mut result = completion.result.clone();
        let mut ids = std::collections::BTreeSet::new();
        for source in &completion.media {
            if source.id.is_empty() || !ids.insert(&source.id) || !source.path.is_absolute()
                || !bgx_adapter_v2_validator::media::valid_basename(&source.file_name) {
                return Err(completion_error("Invalid or duplicate completion media identity/path"));
            }
            let expected = bgx_adapter_v2_ports::decimal_u64(&source.byte_length).map_err(completion_error)?;
            let mut original = std::fs::File::open(&source.path).map_err(completion_error)?;
            if original.metadata().map_err(completion_error)?.len() != expected {
                return Err(completion_error("Evidence media length changed"));
            }
            // Stage one open-file snapshot, hash that same stream, then let the broker measure it.
            let mut staged = tempfile::NamedTempFile::new_in(work.root().join("staging")).map_err(completion_error)?;
            let mut hasher = Sha256::new();
            let mut length = 0u64;
            let mut bytes = vec![0; 65536];
            loop {
                let n = original.read(&mut bytes).map_err(completion_error)?;
                if n == 0 { break; }
                length = length.checked_add(n as u64).ok_or_else(|| completion_error("Media length overflow"))?;
                if length > expected { return Err(completion_error("Evidence media grew")); }
                staged.write_all(&bytes[..n]).map_err(completion_error)?;
                hasher.update(&bytes[..n]);
            }
            if length != expected || hex::encode(hasher.finalize()) != source.sha256 {
                return Err(completion_error("Evidence media hash changed"));
            }
            staged.as_file().sync_all().map_err(completion_error)?;
            let lease = broker.import_file(&scope, &source.id, staged.path(), &source.kind, &control)
                .map_err(|e| completion_error(e.r#message))?;
            let stat = serde_json::to_value(&lease.stat).map_err(completion_error)?;
            if lease.identity.r#digest != source.sha256 || stat["mime"] != source.mime || stat["byteLength"] != source.byte_length {
                return Err(completion_error("Measured media differs from evidence"));
            }
            result["outputs"].as_array_mut().unwrap().push(json!({"kind":"media","role":source.role,
                "media":{"id":source.id,"kind":source.kind,"handle":lease.handle,"mime":stat["mime"],
                "fileName":lease.file_name,"byteLength":stat["byteLength"]}}));
        }
        let result = serde_json::from_value(result).map_err(completion_error)?;
        session.prepare_reviewed_completion(&input.expected_revision, &input.evidence, &result, &mut broker, &scope, &control)
            .map_err(|e| completion_error(e.message))
    })();
    let closed = broker.close().map_err(|e| completion_error(e.r#message));
    match outcome { Err(e) => Err(e), Ok(value) => { closed?; Ok(value) } }
}
