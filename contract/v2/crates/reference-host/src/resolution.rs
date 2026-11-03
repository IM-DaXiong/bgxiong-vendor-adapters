//! Offline adjudication of an unknown submission by an explicitly authorized host
//! operator. Evidence content is reviewed by that operator, never inferred by SDK.
//! Preparing a transition neither contacts a service nor changes storage.
use crate::checkpoint::{fail, validate, CheckpointStore, Result};
use bgx_adapter_v2_ports::{decimal_u64, resource_policy};
use bgx_adapter_v2_validator::SchemaRegistry;
use serde_json::{json, Value};

/// The caller must authorize the maintenance operator before invoking this API.
/// Returned checkpoint must replace the original with an exact revision/row CAS,
/// atomically with any host task state. This is not a submission permission.
pub fn prepare_operator_reviewed(
    checkpoint: &Value,
    frozen: &Value,
    expected_revision: &str,
    evidence: &Value,
) -> Result<Value> {
    validate_reviewed(checkpoint, frozen, expected_revision, evidence)?;
    if evidence["outcome"] == "completed" {
        return Err(fail("INVALID_INPUT", "Completed recovery requires the original session and live media broker"));
    }
    let revision = decimal_u64(expected_revision)
        .map_err(|_| fail("INVALID_INPUT", "Checkpoint revision is invalid"))?
        .checked_add(1)
        .ok_or_else(|| fail("BUDGET_EXCEEDED", "Checkpoint revision is exhausted"))?;
    let mut next = checkpoint.clone();
    next["revision"] = json!(revision.to_string());
    next["resolution"] = evidence.clone();
    if evidence["outcome"] == "rejected" {
        next["state"] = json!("rejected");
    }
    validate(&next)?;
    if serde_json::to_vec(&next).map_err(|_| fail("INVALID_INPUT", "Resolution cannot be serialized"))?.len() > resource_policy()["limits"]["maxStateBytes"].as_u64().unwrap() as usize {
        return Err(fail("BUDGET_EXCEEDED", "Resolved checkpoint exceeds public state byte budget"));
    }
    Ok(next)
}

/// Pluggable-store variant; the same validation followed by the store's exact CAS.
pub fn apply_operator_reviewed(
    store: &mut CheckpointStore,
    request_id: &str,
    expected_revision: &str,
    evidence: &Value,
) -> Result<Value> {
    let saved = store.load(request_id)?
        .ok_or_else(|| fail("RESOURCE_UNAVAILABLE", "Original checkpoint is missing"))?;
    let next = prepare_operator_reviewed(&saved.checkpoint, &saved.frozen, expected_revision, evidence)?;
    store.update(&saved.checkpoint, &next)?;
    Ok(next)
}

pub(crate) fn validate_reviewed(checkpoint: &Value, frozen: &Value, expected_revision: &str, evidence: &Value) -> Result<()> {
    let max = resource_policy()["limits"]["maxStateBytes"].as_u64().unwrap() as usize;
    for value in [checkpoint, frozen, evidence] {
        if serde_json::to_vec(value).map_err(|_| fail("INVALID_INPUT", "Resolution cannot be serialized"))?.len() > max {
            return Err(fail("BUDGET_EXCEEDED", "Resolution input exceeds public state byte budget"));
        }
    }
    validate(checkpoint)?;
    SchemaRegistry::validate_public("resolution", evidence)
        .map_err(|_| fail("INVALID_INPUT", "Resolution evidence differs from public schema"))?;
    crate::session::validate_frozen_checkpoint(checkpoint, frozen)?;
    if checkpoint["revision"] != expected_revision {
        return Err(fail("CHECKPOINT_CONFLICT", "Checkpoint revision changed; reload original request"));
    }
    if checkpoint["state"] != "acceptance_unknown" {
        return Err(fail("INVALID_INPUT", "Only an unknown submission may be adjudicated"));
    }
    if evidence["requestId"] != checkpoint["requestId"]
        || evidence["requestDigest"] != checkpoint["requestDigest"]
    {
        return Err(fail("INVALID_INPUT", "Resolution evidence belongs to another frozen request"));
    }
    Ok(())
}
