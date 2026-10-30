//! Explicit development credentials only. No environment, keychain or application
//! credential-store access. Secret material has no Serialize/Debug implementation.
use crate::{
    media::LeaseScope,
    policy::{self, error, protected_header, NetworkPolicy, Result},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use bgx_adapter_v2_ports::resource_policy;
use bgx_adapter_v2_validator::SchemaRegistry;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct DevelopmentSecret {
    pub bytes: Zeroizing<Vec<u8>>,
    pub public_id: Option<String>,
    pub custom_header: Option<String>,
}
struct Entry {
    scope: LeaseScope,
    secret: DevelopmentSecret,
    modes: BTreeSet<String>,
    origins: Vec<String>,
    expires: Instant,
}
struct Signature {
    credential: String,
    hash: [u8; 32],
    bytes: usize,
}
pub struct CredentialVault {
    entries: BTreeMap<String, Entry>,
    signatures: Vec<Signature>,
    schema: SchemaRegistry,
    signature_bytes: usize,
}
impl CredentialVault {
    pub fn new() -> Result<Self> {
        Ok(Self {
            entries: BTreeMap::new(),
            signatures: vec![],
            schema: SchemaRegistry::public()
                .map_err(|_| error("INTERNAL_ERROR", "Public signing schema is unavailable"))?,
            signature_bytes: 0,
        })
    }
    pub fn insert(
        &mut self,
        policy: &NetworkPolicy,
        secret: DevelopmentSecret,
        modes: BTreeSet<String>,
        origins: Vec<String>,
        ttl: Duration,
    ) -> Result<String> {
        self.insert_reference(
            policy,
            Uuid::new_v4().to_string(),
            secret,
            modes,
            origins,
            ttl,
        )
    }
    /// Trusted native provisioning of a stable NONSECRET opaque reference. A
    /// fresh per-invocation vault can reissue it after restart with explicit new
    /// secret material; frozen configuration never contains or restores a key.
    pub fn insert_reference(
        &mut self,
        policy: &NetworkPolicy,
        handle: String,
        secret: DevelopmentSecret,
        modes: BTreeSet<String>,
        origins: Vec<String>,
        ttl: Duration,
    ) -> Result<String> {
        policy.check(policy.scope())?;
        if handle.is_empty() || self.entries.contains_key(&handle) {
            return Err(error(
                "INVALID_INPUT",
                "Credential reference is empty or already issued",
            ));
        }
        let limit = resource_policy()["limits"]["maxStateBytes"]
            .as_u64()
            .unwrap() as usize;
        let used = self.entries.iter().fold(0usize, |n, (reference,e)| {
            n.saturating_add(
                reference.len()+e.secret.bytes.len()
                    + e.secret.public_id.as_ref().map_or(0, String::len)
                    + e.secret.custom_header.as_ref().map_or(0, String::len),
            )
        });
        let incoming = handle.len().saturating_add(
            secret
                .bytes
                .len()
                .saturating_add(secret.public_id.as_ref().map_or(0, String::len))
                .saturating_add(secret.custom_header.as_ref().map_or(0, String::len)),
        );
        if used.saturating_add(incoming) > limit {
            return Err(error(
                "BUDGET_EXCEEDED",
                "Retained credential material exceeds the public byte budget",
            ));
        }
        if secret.bytes.is_empty()
            || ttl.is_zero()
            || Instant::now().checked_add(ttl).is_none()
            || origins.is_empty()
            || origins.iter().any(|o| !policy.origins().contains(o))
        {
            return Err(error(
                "INVALID_INPUT",
                "Development credential material, origins or lifetime is invalid",
            ));
        }
        let unsupported: Value =
            serde_json::from_str(include_str!("../../../protocol/host-feature-catalog.json"))
                .expect("feature catalog");
        if modes.is_empty()
            || modes.iter().any(|m| {
                !policy.auth_modes().contains(m)
                    || unsupported["unsupportedAuthModes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v == m)
                    || !(matches!(m.as_str(), "bearer" | "custom-header")
                        || policy::protocol()["security"]["guestVisiblePrimitives"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|v| v == m))
            })
        {
            return Err(error(
                "PERMISSION_DENIED",
                "Credential auth mode is unsupported or was not requested by the package",
            ));
        }
        if modes.contains("custom-header") {
            let name = secret.custom_header.as_deref().ok_or_else(|| {
                error(
                    "INVALID_INPUT",
                    "Custom header credential requires an explicit header name",
                )
            })?;
            if HeaderName::from_bytes(name.as_bytes()).is_err() || protected_header(name) {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Custom credential header is protected or invalid",
                ));
            }
        }
        self.entries.insert(
            handle.clone(),
            Entry {
                scope: policy.scope().clone(),
                secret,
                modes,
                origins,
                expires: Instant::now() + ttl,
            },
        );
        Ok(handle)
    }
    fn entry(&self, policy: &NetworkPolicy, handle: &str) -> Result<&Entry> {
        policy.check(policy.scope())?;
        let entry = self
            .entries
            .get(handle)
            .ok_or_else(|| error("PERMISSION_DENIED", "Credential handle is not authorized"))?;
        if &entry.scope != policy.scope() || Instant::now() >= entry.expires {
            return Err(error(
                "PERMISSION_DENIED",
                "Credential handle is expired or belongs to another request",
            ));
        }
        Ok(entry)
    }
    pub fn capabilities(&self, policy: &NetworkPolicy, handle: &str) -> Result<Vec<String>> {
        Ok(self.entry(policy, handle)?.modes.iter().cloned().collect())
    }
    pub fn sign(
        &mut self,
        policy: &NetworkPolicy,
        handle: &str,
        spec_json: &str,
    ) -> Result<String> {
        let limit = resource_policy()["limits"]["maxStateBytes"]
            .as_u64()
            .unwrap() as usize;
        if spec_json.len() > limit {
            return Err(error(
                "BUDGET_EXCEEDED",
                "Signing input exceeds the public byte budget",
            ));
        }
        let spec: Value = serde_json::from_str(spec_json)
            .map_err(|_| error("INVALID_INPUT", "Signing specification is invalid JSON"))?;
        self.schema
            .validate_at(
                "https://bgxiong.local/vendor-adapter/v2/host-ports.schema.json",
                "/$defs/signSpec",
                &spec,
            )
            .map_err(|_| {
                error(
                    "INVALID_INPUT",
                    "Signing specification does not satisfy its versioned public schema",
                )
            })?;
        let entry = self.entry(policy, handle)?;
        let kind = spec["kind"].as_str().unwrap();
        if !entry.modes.contains(kind) {
            return Err(error(
                "PERMISSION_DENIED",
                "Credential does not grant this signing primitive",
            ));
        }
        let hmac = |bytes: &[u8]| -> Result<Vec<u8>> {
            let mut mac = Hmac::<Sha256>::new_from_slice(&entry.secret.bytes)
                .map_err(|_| error("INVALID_INPUT", "Credential signing key is invalid"))?;
            mac.update(bytes);
            Ok(mac.finalize().into_bytes().to_vec())
        };
        let value = match kind {
            "hmac-sha256" => {
                let digest = hmac(spec["message"].as_str().unwrap().as_bytes())?;
                match spec["encoding"].as_str().unwrap() {
                    "hex" => hex::encode(digest),
                    "base64url" => URL_SAFE_NO_PAD.encode(digest),
                    _ => unreachable!("closed public encoding"),
                }
            }
            "jwt-hs256" => {
                let claims = spec["claimsJson"].as_str().unwrap();
                let parsed: Value = serde_json::from_str(claims)
                    .map_err(|_| error("INVALID_INPUT", "JWT claims are not JSON"))?;
                if !parsed.is_object() {
                    return Err(error("INVALID_INPUT", "JWT claims must be a JSON object"));
                }
                let header =
                    serde_json::to_vec(&policy::protocol()["security"]["jwtHs256Header"]).unwrap();
                let signing = format!(
                    "{}.{}",
                    URL_SAFE_NO_PAD.encode(header),
                    URL_SAFE_NO_PAD.encode(claims.as_bytes())
                );
                format!(
                    "{}.{}",
                    signing,
                    URL_SAFE_NO_PAD.encode(hmac(signing.as_bytes())?)
                )
            }
            "hmac-sha256-credential" => {
                let public_id = entry.secret.public_id.as_deref().ok_or_else(|| {
                    error(
                        "INVALID_INPUT",
                        "Credential presentation requires its explicit public ID",
                    )
                })?;
                let signature = hex::encode(hmac(spec["canonical"].as_str().unwrap().as_bytes())?);
                render_presentation(
                    policy::protocol()["security"]["hmacCredentialPresentation"]
                        .as_str()
                        .unwrap(),
                    &[
                        ("publicId", public_id),
                        ("date", spec["date"].as_str().unwrap()),
                        ("hexHmacOfCanonical", &signature),
                    ],
                )?
            }
            _ => return Err(error("PERMISSION_DENIED", "Unsupported signing primitive")),
        };
        let cost = std::mem::size_of::<Signature>() + handle.len();
        let hash = Sha256::digest(value.as_bytes()).into();
        if self
            .signatures
            .iter()
            .any(|s| s.credential == handle && s.hash == hash)
        {
            return Ok(value);
        }
        if self
            .signature_bytes
            .checked_add(cost)
            .is_none_or(|n| n > limit)
        {
            return Err(error(
                "BUDGET_EXCEEDED",
                "Signing receipts exceed the public byte budget",
            ));
        }
        if !self
            .signatures
            .iter()
            .any(|s| s.credential == handle && s.hash == hash)
        {
            self.signatures.push(Signature {
                credential: handle.into(),
                hash,
                bytes: cost,
            });
            self.signature_bytes += cost;
        }
        Ok(value)
    }
    fn signed_authorization(&self, policy: &NetworkPolicy, origin: &str, value: &str) -> bool {
        let signature = value.strip_prefix("Bearer ").unwrap_or(value);
        let hash: [u8; 32] = Sha256::digest(signature.as_bytes()).into();
        self.signatures.iter().any(|s| {
            s.hash == hash
                && self
                    .entry(policy, &s.credential)
                    .is_ok_and(|e| e.origins.iter().any(|o| o == origin))
        })
    }
    pub fn headers(
        &self,
        policy: &NetworkPolicy,
        origin: &str,
        plan: &Value,
        credential: Option<&str>,
    ) -> Result<HeaderMap> {
        policy.check(policy.scope())?;
        let mut headers = HeaderMap::new();
        let mut add = |name: &str, value: &str| -> Result<()> {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| error("INVALID_INPUT", "HTTP header name is invalid"))?;
            let mut value = HeaderValue::from_str(value)
                .map_err(|_| error("INVALID_INPUT", "HTTP header value is invalid"))?;
            value.set_sensitive(true);
            if headers.contains_key(&name) {
                return Err(error("INVALID_INPUT", "Duplicate HTTP header is forbidden"));
            }
            headers.insert(name, value);
            Ok(())
        };
        for header in plan["headers"]
            .as_array()
            .ok_or_else(|| error("INVALID_INPUT", "HTTP headers must be materialized"))?
        {
            let name = header["name"].as_str().unwrap();
            let value = header["value"].as_str().unwrap();
            if protected_header(name)
                && !(name.eq_ignore_ascii_case("authorization")
                    && (policy.permits_embedded_authorization()
                        || self.signed_authorization(policy, origin, value)))
            {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Adapter supplied a protected HTTP header",
                ));
            }
            add(name, value)?;
        }
        let entry = credential.map(|h| self.entry(policy, h)).transpose()?;
        if entry.is_some_and(|e| !e.origins.iter().any(|o| o == origin)) {
            return Err(error(
                "PERMISSION_DENIED",
                "Credential is not granted to this exact origin",
            ));
        }
        for generated in plan["generatedHeaders"]
            .as_array()
            .ok_or_else(|| error("INVALID_INPUT", "Generated headers must be materialized"))?
        {
            let name = generated["name"].as_str().unwrap();
            let target = generated["targetField"].as_str().unwrap();
            let mode = generated["kind"].as_str().unwrap();
            if !name.eq_ignore_ascii_case(target) {
                return Err(error(
                    "INVALID_INPUT",
                    "Generated header target differs from its name",
                ));
            }
            let entry = entry.ok_or_else(|| {
                error(
                    "PERMISSION_DENIED",
                    "Generated HTTP header requires an authorized credential",
                )
            })?;
            if !entry.modes.contains(mode) {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Credential does not grant the requested header mode",
                ));
            }
            let secret = std::str::from_utf8(&entry.secret.bytes)
                .map_err(|_| error("INVALID_INPUT", "HTTP credential is not valid UTF-8"))?;
            match mode {
                "bearer" if name.eq_ignore_ascii_case("authorization") => {
                    add(name, &format!("Bearer {secret}"))?
                }
                "custom-header"
                    if !protected_header(name)
                        && entry
                            .secret
                            .custom_header
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(name)) =>
                {
                    add(name, secret)?
                }
                _ => {
                    return Err(error(
                        "PERMISSION_DENIED",
                        "Generated credential header is unsupported or unauthorized",
                    ))
                }
            }
        }
        Ok(headers)
    }
    pub fn release_request(&mut self, scope: &LeaseScope) {
        self.entries.retain(|_, entry| &entry.scope != scope);
        self.signatures
            .retain(|s| self.entries.contains_key(&s.credential));
        self.signature_bytes = self.signatures.iter().map(|s| s.bytes).sum();
    }
    pub fn redact(&self, text: &str) -> String {
        let mut output = text.to_string();
        for entry in self.entries.values() {
            if let Ok(secret) = std::str::from_utf8(&entry.secret.bytes) {
                if !secret.is_empty() {
                    output = output.replace(secret, "[redacted]");
                }
            }
        }
        output
    }
}
fn render_presentation(template: &str, fields: &[(&str, &str)]) -> Result<String> {
    // Scan the authored template once. Substituted public IDs/dates are data and
    // must never be rescanned as template instructions.
    let mut output = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}').ok_or_else(|| {
            error(
                "INTERNAL_ERROR",
                "Public signing presentation template is invalid",
            )
        })?;
        let value = fields
            .iter()
            .find(|(name, _)| *name == &after[..end])
            .map(|(_, value)| *value)
            .ok_or_else(|| {
                error(
                    "INTERNAL_ERROR",
                    "Public signing presentation placeholder is unknown",
                )
            })?;
        output.push_str(value);
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}
