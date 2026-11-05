use std::collections::{BTreeMap, BTreeSet};
use serde_json::Value;
use crate::{ContractError, ValidatedOperationInput, hash_bytes};
use crate::identity::PackageIdentity;

#[derive(Debug, Clone)]
pub struct MediaFacts {
    pub digest: String,
    pub kind: String, pub mime: String, pub byte_length: u64, pub file_name: String,
    pub width: Option<u32>, pub height: Option<u32>, pub duration_ms: Option<u64>,
    pub timestamps_ms: Option<Vec<u64>>,
}
#[derive(Debug, Clone)]
pub struct MediaLease {
    pub package: PackageIdentity, pub binding_id: String, pub request_id: String,
    pub media_id: String, pub live: bool, pub facts: MediaFacts,
}
#[derive(Debug, Clone)]
pub struct UploadProvenance {
    pub media_digest: String,
    pub package: PackageIdentity, pub binding_id: String, pub configuration_digest: String,
    pub media_id: String,
}

/// Broker-owned facts. Callers must never deserialize this from adapter JSON.
pub struct MediaValidationContext {
    pub package: PackageIdentity, pub binding_id: String, pub request_id: String,
    pub leases_by_handle: BTreeMap<String, MediaLease>,
    pub uploads_by_ref: BTreeMap<String, UploadProvenance>,
}

pub fn valid_basename(name: &str) -> bool {
    !name.is_empty() && !matches!(name, "." | "..") && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
}

pub fn validate_slots(contract: &Value) -> Result<(), ContractError> {
    for op in contract["operations"].as_array().unwrap() {
        for list in ["mediaSlots", "outputSlots"] {
            let mut roles = BTreeSet::new();
            let mut pointers = BTreeSet::new();
            for slot in op[list].as_array().unwrap() {
                if !roles.insert(slot["role"].as_str().unwrap())
                    || slot["maxCount"].as_u64().is_some_and(|max| max < slot["minCount"].as_u64().unwrap()) {
                    return Err(ContractError::new("INVALID_PACKAGE", list, "duplicate role or contradictory count limits"));
                }
                if list == "mediaSlots" && (!slot["inputPointer"].as_str().unwrap().starts_with('/')
                    || !pointers.insert(slot["inputPointer"].as_str().unwrap())) {
                    return Err(ContractError::new("INVALID_PACKAGE", "/mediaSlots", "duplicate or invalid media field pointer"));
                }
            }
        }
    }
    Ok(())
}

pub fn validate_references(context: &ValidatedOperationInput) -> Result<(), ContractError> {
    let request = context.request();
    let mut media = BTreeMap::new();
    for m in request["media"].as_array().unwrap() {
        let id = m["id"].as_str().unwrap();
        if media.insert(id, m).is_some() || !valid_basename(m["fileName"].as_str().unwrap())
            || m["byteLength"].as_str().unwrap().parse::<u64>().is_err() {
            return Err(ContractError::new("INVALID_INPUT", "/media", "duplicate media ID, invalid basename or byte length"));
        }
    }
    for slot in context.operation()["mediaSlots"].as_array().unwrap() {
        let pointer = slot["inputPointer"].as_str().unwrap();
        let value = request["input"].pointer(pointer);
        let references: Vec<&Value> = match value { None | Some(Value::Null) => vec![], Some(Value::Array(v)) => v.iter().collect(), Some(v) => vec![v] };
        let count = references.len() as u64;
        if count < slot["minCount"].as_u64().unwrap() || slot["maxCount"].as_u64().is_some_and(|max| count > max) {
            return Err(ContractError::new("INVALID_INPUT", pointer, "media slot count outside declared limits"));
        }
        let mut seen = BTreeSet::new();
        for reference in references {
            let id = reference.get("mediaId").and_then(Value::as_str).ok_or_else(|| ContractError::new("INVALID_INPUT", pointer, "media slot requires structured mediaId references"))?;
            if !reference.is_object() || reference.as_object().unwrap().len() != 1 || !seen.insert(id)
                || media.get(id).is_none_or(|m| m["kind"] != slot["mediaKind"]) {
                return Err(ContractError::new("INVALID_INPUT", pointer, "duplicate, dangling or wrong-kind media reference"));
            }
        }
    }
    let mut uploaded = BTreeSet::new();
    for upload in request["uploads"].as_array().unwrap() {
        let id = upload["mediaId"].as_str().unwrap();
        if !media.contains_key(id) || !uploaded.insert(id) {
            return Err(ContractError::new("INVALID_INPUT", "/uploads", "upload media ID is missing or duplicated"));
        }
    }
    Ok(())
}

pub fn validate_host_media(context: &ValidatedOperationInput, host: &MediaValidationContext) -> Result<(), ContractError> {
    validate_references(context)?;
    let mut facts = BTreeMap::new();
    for m in context.request()["media"].as_array().unwrap() {
        facts.insert(m["id"].as_str().unwrap().into(),validate_leased_media(m,host)?);
    }
    let config_digest = hash_bytes(&serde_json::to_vec(&context.request()["configuration"]).unwrap());
    for upload in context.request()["uploads"].as_array().unwrap() {
        let proof = host.uploads_by_ref.get(upload["uploadRef"].as_str().unwrap())
            .ok_or_else(|| ContractError::new("PERMISSION_DENIED", "/uploads", "upload reference has no successful broker receipt"))?;
        if proof.package != host.package || proof.binding_id != host.binding_id || proof.configuration_digest != config_digest
            || proof.media_id != upload["mediaId"].as_str().unwrap()
            || facts.get(&proof.media_id).is_none_or(|facts:&MediaFacts|facts.digest != proof.media_digest) {
            return Err(ContractError::new("PERMISSION_DENIED", "/uploads", "upload receipt belongs to another package, binding or media"));
        }
    }
    crate::interaction::validate_bounds(context, &facts)
}

/// Same live-lease check for operation inputs and streamed media references.
pub fn validate_leased_media(m:&Value,host:&MediaValidationContext)->Result<MediaFacts,ContractError>{
    crate::SchemaRegistry::validate_public("media-input",m)?;
    let lease=host.leases_by_handle.get(m["handle"].as_str().unwrap()).ok_or_else(||ContractError::new("PERMISSION_DENIED","/media/handle","media handle is not a live broker lease"))?;
    if !lease.live||lease.package!=host.package||lease.binding_id!=host.binding_id||lease.request_id!=host.request_id||m["id"]!=lease.media_id||m["kind"]!=lease.facts.kind||m["mime"]!=lease.facts.mime||m["fileName"]!=lease.facts.file_name||m["byteLength"].as_str().unwrap().parse::<u64>().ok()!=Some(lease.facts.byte_length){return Err(ContractError::new("PERMISSION_DENIED","/media","lease ownership or broker-measured media facts do not match"));}
    Ok(lease.facts.clone())
}
