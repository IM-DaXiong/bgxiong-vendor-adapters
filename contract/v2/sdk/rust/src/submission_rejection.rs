//! Explicit non-acceptance evidence over the existing checkpoint port.
//! The caller must know that no operation was accepted. This helper never infers
//! acceptance from HTTP status, error codes, text, or absent task identifiers.
use alloc::string::ToString;
use serde_json::{json, Value};

pub const FEATURE: &str = "submission-rejection-v1";

/// Build the exact next checkpoint. Read the original checkpoint and then write
/// this candidate with the original requestId/revision using host-checkpoint.
/// Only successful host CAS confirms durable rejection; preserve unknown on error.
pub fn checkpoint_candidate(
    checkpoint: &Value,
    basis: &str,
    reason: &str,
    evidence: &Value,
) -> Result<Value, &'static str> {
    if checkpoint["state"] != "acceptance_unknown"
        || !checkpoint["vendorTaskId"].is_null()
        || !checkpoint["resumeToken"].is_null()
        || !checkpoint["result"].is_null()
        || !matches!(basis, "before_submission" | "service_rejected")
        || reason.is_empty() || reason.chars().count() > 4096
        || !evidence.is_object()
    {
        return Err("Rejection requires an unknown original submission and explicit evidence");
    }
    let evidence_json = evidence.to_string();
    if evidence_json.chars().count() > 16384 {
        return Err("Rejection evidence exceeds the public budget");
    }
    let current = checkpoint["revision"].as_str().ok_or("Missing revision")?;
    let revision = current.parse::<u64>().map_err(|_| "Invalid revision")?;
    if current != revision.to_string() { return Err("Noncanonical revision"); }
    let next_revision = revision.checked_add(1).ok_or("Revision exhausted")?;
    let mut next = checkpoint.clone();
    next["revision"] = json!(next_revision.to_string());
    next["state"] = json!("rejected");
    next["submissionRejection"] = json!({"schemaVersion":1,
        "requestId":checkpoint["requestId"],"requestDigest":checkpoint["requestDigest"],
        "basis":basis,"reason":reason,"evidenceJson":evidence_json});
    Ok(next)
}
