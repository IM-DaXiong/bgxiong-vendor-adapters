//! Pure package checks. The caller streams files and supplies their measured hashes.
//! Trust is explicit development input; this module never reads a trust store.
use std::collections::{BTreeMap, BTreeSet};
use serde_json::Value;
use crate::{ContractError, SchemaRegistry, hash_bytes};
use crate::identity::{PackageIdentity, protocol};

pub struct PackageContents<'a> {
    pub manifest_bytes: &'a [u8],
    pub hash_manifest_bytes: &'a [u8],
    pub signature: Option<&'a str>,
    pub measured_file_hashes: &'a BTreeMap<String, String>,
}

pub struct PublisherTrust {
    pub publisher_id: String,
    pub key_id: String,
    pub public_key: String,
    pub plugin_prefixes: Vec<String>,
}

#[derive(Default)]
pub struct TrustContext {
    pub publishers: Vec<PublisherTrust>,
    pub revoked_plugins: BTreeSet<String>,
    pub revoked_versions: BTreeSet<(String, String)>,
    pub revoked_keys: BTreeSet<String>,
    pub denylisted_hosts: BTreeSet<String>,
    pub is_fresh: bool,
    pub supported_features: BTreeSet<String>,
}

pub struct VerifiedPackage {
    identity: PackageIdentity,
    manifest: Value,
    namespace: String,
    trusted: bool,
    features: BTreeSet<String>,
}

impl VerifiedPackage {
    pub fn identity(&self) -> &PackageIdentity { &self.identity }
    pub fn manifest(&self) -> &Value { &self.manifest }
    pub fn namespace(&self) -> &str { &self.namespace }
    pub fn is_trusted(&self) -> bool { self.trusted }
    pub fn enabled_features(&self) -> &BTreeSet<String> { &self.features }
}

pub fn valid_relative_path(path: &str) -> bool {
    bgx_adapter_v2_sdk::guest::valid_package_path(path)
}

pub fn verify_package(contents: &PackageContents<'_>, trust: &TrustContext) -> Result<VerifiedPackage, ContractError> {
    let invalid = |path: &str, why: &str| ContractError::new("INVALID_PACKAGE", path, why);
    let manifest: Value = serde_json::from_slice(contents.manifest_bytes).map_err(|_| invalid("/manifest", "invalid manifest JSON"))?;
    SchemaRegistry::validate_public("manifest", &manifest)?;
    let entry = manifest["entry"].as_str().unwrap();
    semver::Version::parse(manifest["version"].as_str().unwrap()).map_err(|_| invalid("/version", "version must be semver"))?;
    if !valid_relative_path(entry) { return Err(invalid("/entry", "entry must stay inside the package")); }
    let inventory = std::str::from_utf8(contents.hash_manifest_bytes).map_err(|_| invalid("/manifest.sha256", "inventory must be UTF-8"))?;
    let declared = bgx_adapter_v2_sdk::guest::parse_hash_inventory(inventory).map_err(|reason|invalid("/manifest.sha256",reason))?;
    let p = protocol();
    for file in declared.keys() {
        if file == p["signature"]["hashManifest"].as_str().unwrap()
            || file == p["signature"]["signatureFile"].as_str().unwrap()
            {
            return Err(invalid("/manifest.sha256", "invalid, duplicate or self-referential inventory entry"));
        }
    }
    if declared != *contents.measured_file_hashes || declared.get("manifest.json") != Some(&hash_bytes(contents.manifest_bytes)) {
        return Err(invalid("/manifest.sha256", "file set or content digest mismatch"));
    }
    if declared.get(entry).map(String::as_str) != manifest["artifactSha256"].as_str() {
        return Err(invalid("/artifactSha256", "artifact digest does not match entry"));
    }
    let plugin = manifest["pluginId"].as_str().unwrap();
    let version = manifest["version"].as_str().unwrap();
    if trust.revoked_plugins.contains(plugin) || trust.revoked_versions.contains(&(plugin.to_owned(), version.to_owned())) {
        return Err(ContractError::new("UNAUTHORIZED", "/pluginId", "package has been revoked"));
    }
    for host in manifest["requestedCapabilities"]["networkHosts"].as_array().unwrap() {
        if trust.denylisted_hosts.contains(host.as_str().unwrap()) { return Err(ContractError::new("PERMISSION_DENIED", "/networkHosts", "host is denylisted")); }
    }
    let mut enabled = BTreeSet::new();
    let features: Value = serde_json::from_str(include_str!("../../../protocol/host-feature-catalog.json")).expect("published feature catalog");
    let known = |feature: &str| features["features"].as_array().unwrap().iter().any(|f| f["id"] == feature);
    for feature in manifest["requiredFeatures"].as_array().unwrap() {
        let feature = feature.as_str().unwrap();
        if !known(feature) || !trust.supported_features.contains(feature) {
            return Err(ContractError::new("UNSUPPORTED_FEATURE", "/requiredFeatures", "required host feature is unavailable"));
        }
        enabled.insert(feature.to_owned());
    }
    for feature in manifest["optionalFeatures"].as_array().unwrap() {
        if known(feature.as_str().unwrap()) && trust.supported_features.contains(feature.as_str().unwrap()) { enabled.insert(feature.as_str().unwrap().to_owned()); }
    }
    let signed_claim = !manifest["publisherKeyId"].is_null();
    if signed_claim != contents.signature.is_some() { return Err(invalid("/publisherKeyId", "publisher identity and signature must be supplied together")); }
    let namespace;
    if let Some(signature) = contents.signature {
        if !trust.is_fresh { return Err(ContractError::new("UNAUTHORIZED", "/publisherKeyId", "explicit trust data is stale or absent")); }
        let key_id = manifest["publisherKeyId"].as_str().unwrap();
        if trust.revoked_keys.contains(key_id) { return Err(ContractError::new("UNAUTHORIZED", "/publisherKeyId", "publisher key revoked")); }
        let publisher = trust.publishers.iter().find(|t| t.key_id == key_id && t.publisher_id == manifest["publisherId"].as_str().unwrap())
            .ok_or_else(|| ContractError::new("UNAUTHORIZED", "/publisherKeyId", "publisher is not trusted by explicit context"))?;
        if !publisher.plugin_prefixes.iter().any(|prefix| plugin == prefix.trim_end_matches('.') || plugin.starts_with(&(prefix.trim_end_matches('.').to_owned() + "."))) {
            return Err(ContractError::new("UNAUTHORIZED", "/pluginId", "package is outside publisher prefixes"));
        }
        let key = minisign_verify::PublicKey::decode(&publisher.public_key).map_err(|_| invalid("/signature", "invalid publisher public key"))?;
        let signature = minisign_verify::Signature::decode(signature).map_err(|_| invalid("/signature", "invalid minisign signature"))?;
        key.verify(contents.hash_manifest_bytes, &signature, false).map_err(|_| invalid("/signature", "minisign verification failed"))?;
        if !manifest["requestedCapabilities"]["embeddedHeaders"].as_array().unwrap().is_empty() {
            return Err(ContractError::new("PERMISSION_DENIED", "/embeddedHeaders", "signed packages cannot embed protected authorization headers"));
        }
        namespace = publisher.publisher_id.clone();
    } else {
        if !plugin.starts_with(p["signature"]["unsignedNamespacePrefix"].as_str().unwrap()) {
            return Err(ContractError::new("UNAUTHORIZED", "/pluginId", "unsigned package requires local namespace"));
        }
        namespace = plugin.to_owned();
    }
    Ok(VerifiedPackage { identity: PackageIdentity::from_verified(&manifest, contents.hash_manifest_bytes),
        manifest, namespace, trusted: signed_claim, features: enabled })
}
