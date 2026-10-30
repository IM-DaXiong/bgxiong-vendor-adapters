//! `run --host-checkpoints`: checkpoint rows are served by the embedding host over this
//! process's stdio, one JSON line each way, so a product keeps v2 checkpoints in its own
//! durable task table. Shared rules (schema, secrets, budget, CAS) stay in the reference
//! host; the host only stores bytes. The final stdout line is still the standard Response.
use bgx_adapter_v2_reference_host::checkpoint::{CheckpointError, CheckpointRows, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::sync::Mutex;

pub struct StdioRows;

fn fail(message: &'static str) -> CheckpointError {
    CheckpointError {
        code: "RESOURCE_UNAVAILABLE".into(),
        message,
    }
}

pub(crate) fn exchange(request: Value) -> Result<Value> {
    static LINE: Mutex<()> = Mutex::new(());
    let _turn = LINE.lock().map_err(|_| fail("Host checkpoint channel is poisoned"))?;
    {
        let mut out = std::io::stdout().lock();
        serde_json::to_writer(&mut out, &request)
            .map_err(|_| fail("Host checkpoint request cannot be written"))?;
        out.write_all(b"\n")
            .and_then(|_| out.flush())
            .map_err(|_| fail("Host checkpoint request cannot be written"))?;
    }
    let mut line = String::new();
    let limit = bgx_adapter_v2_ports::resource_policy()["limits"]["maxFrameBytes"].as_u64().unwrap();
    let read = std::io::stdin()
        .lock().take(limit + 1)
        .read_line(&mut line)
        .map_err(|_| fail("Host checkpoint reply cannot be read"))?;
    if read as u64 > limit || !line.ends_with('\n') { return Err(fail("Host channel reply exceeds its frame budget or has no newline")); }
    if read == 0 {
        return Err(fail("Host closed the checkpoint channel"));
    }
    let reply: Value =
        serde_json::from_str(&line).map_err(|_| fail("Host checkpoint reply is not JSON"))?;
    if reply.get("error").is_some() {
        return Err(fail("Host checkpoint storage failed"));
    }
    Ok(reply)
}

fn call(request: Value) -> Result<Value> { exchange(json!({"hostCheckpoint":request})) }

fn text(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| fail("Checkpoint bytes are not UTF-8 JSON"))
}

fn cut(value: &Value, max: usize) -> Result<Vec<u8>> {
    let bytes = value
        .as_str()
        .ok_or_else(|| fail("Host checkpoint row is malformed"))?
        .as_bytes();
    Ok(bytes[..bytes.len().min(max + 1)].to_vec())
}

impl CheckpointRows for StdioRows {
    fn get(&self, request_id: &str, max: usize) -> Result<Option<(String, Vec<u8>, Vec<u8>)>> {
        let reply = call(json!({ "op": "get", "requestId": request_id }))?;
        let row = &reply["row"];
        if row.is_null() {
            return Ok(None);
        }
        let revision = row["revision"]
            .as_str()
            .ok_or_else(|| fail("Host checkpoint row is malformed"))?;
        Ok(Some((
            revision.to_owned(),
            cut(&row["checkpoint"], max)?,
            cut(&row["frozen"], max)?,
        )))
    }
    fn insert(&mut self, request_id: &str, checkpoint: &[u8], frozen: &[u8]) -> Result<bool> {
        let reply = call(json!({
            "op": "insert",
            "requestId": request_id,
            "checkpoint": text(checkpoint)?,
            "frozen": text(frozen)?,
        }))?;
        reply["inserted"]
            .as_bool()
            .ok_or_else(|| fail("Host checkpoint reply is malformed"))
    }
    fn swap(
        &mut self,
        request_id: &str,
        old_revision: &str,
        old: &[u8],
        revision: &str,
        new: &[u8],
    ) -> Result<bool> {
        let reply = call(json!({
            "op": "swap",
            "requestId": request_id,
            "oldRevision": old_revision,
            "old": text(old)?,
            "revision": revision,
            "new": text(new)?,
        }))?;
        reply["swapped"]
            .as_bool()
            .ok_or_else(|| fail("Host checkpoint reply is malformed"))
    }
}
