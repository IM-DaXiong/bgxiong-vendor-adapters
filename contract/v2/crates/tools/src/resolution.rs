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
    let checkpoint = bgx_adapter_v2_reference_host::resolution::prepare_operator_reviewed(
        &input.checkpoint, &input.frozen, &input.expected_revision, &input.evidence,
    ).map_err(|e| {
        let code = match e.code.as_str() {
            "CHECKPOINT_CONFLICT" => "CHECKPOINT_CONFLICT",
            "BUDGET_EXCEEDED" => "BUDGET_EXCEEDED",
            _ => "INVALID_INPUT",
        };
        error(code, "/checkpoint", e.message, "Reload the original task; never resubmit it")
    })?;
    Ok(json!({"expectedRevision":input.expected_revision,"checkpoint":checkpoint}))
}
