//! Explicit development inputs. No ambient configuration or credential discovery.
use crate::{
    checkpoint::{fail, Result, SnapshotPolicy},
    credential::DevelopmentSecret,
    media::{DevelopmentWorkspace, ProbeTool},
    policy::DevelopmentNetworkGrant,
};
use bgx_adapter_v2_ports::{resource_policy, wire::guest::parse_hash_inventory};
use bgx_adapter_v2_transport::Options;
use bgx_adapter_v2_validator::{
    hash_bytes,
    package::{verify_package, PackageContents, TrustContext, VerifiedPackage},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use zeroize::Zeroizing;

pub struct CredentialProvision {
    pub reference: String,
    pub secret: DevelopmentSecret,
    pub modes: BTreeSet<String>,
    pub origins: Vec<String>,
    pub ttl: Duration,
}
pub struct ReferenceConfig {
    pub workspace: DevelopmentWorkspace,
    pub sdk_root: PathBuf,
    pub worker: PathBuf,
    pub transport: Options,
    pub network: DevelopmentNetworkGrant,
    pub credentials: Vec<CredentialProvision>,
    pub media_ttl: Duration,
    pub probe: Option<ProbeTool>,
    pub configuration_ref: String,
}
impl ReferenceConfig {
    pub fn snapshot_policy(&self) -> Result<SnapshotPolicy> {
        SnapshotPolicy::new(
            self.credentials
                .iter()
                .map(|c| Zeroizing::new(c.secret.bytes.to_vec()))
                .collect(),
        )
    }
}
/// Host-measured successful upload receipts can be used by a later CLI process.
/// They live exclusively in the marked workspace, scoped to package/binding/config.
pub(crate) fn save_upload(
    workspace: &DevelopmentWorkspace,
    scope: &crate::media::LeaseScope,
    configuration_digest: &str,
    media_id: &str,
    media_digest: &str,
    reference: &str,
    policy: &SnapshotPolicy,
) -> Result<()> {
    let value = serde_json::json!({"package":scope.package,"bindingId":scope.binding_id,"configurationDigest":configuration_digest,"mediaId":media_id,"mediaDigest":media_digest,"uploadRef":reference});
    policy.check(&value)?;
    let path = upload_path(workspace, scope, configuration_digest, reference)?;
    let bytes = serde_json::to_vec(&value).unwrap();
    if bytes.len() as u64
        > resource_policy()["limits"]["maxStateBytes"]
            .as_u64()
            .unwrap()
    {
        return Err(fail(
            "BUDGET_EXCEEDED",
            "Upload receipt exceeds state byte budget",
        ));
    }
    if path.exists() {
        if bounded(&path, "maxStateBytes")? == bytes {
            return Ok(());
        }
        return Err(fail(
            "INVALID_INPUT",
            "Upload reference was already issued for different media",
        ));
    }
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())
        .map_err(|_| fail("RESOURCE_UNAVAILABLE", "Upload receipt staging failed"))?;
    std::io::Write::write_all(&mut file, &bytes)
        .map_err(|_| fail("RESOURCE_UNAVAILABLE", "Upload receipt write failed"))?;
    file.as_file()
        .sync_all()
        .map_err(|_| fail("RESOURCE_UNAVAILABLE", "Upload receipt sync failed"))?;
    file.persist_noclobber(path)
        .map_err(|_| fail("RESOURCE_UNAVAILABLE", "Upload receipt commit failed"))?;
    Ok(())
}
pub(crate) fn read_upload(
    workspace: &DevelopmentWorkspace,
    scope: &crate::media::LeaseScope,
    configuration_digest: &str,
    reference: &str,
    policy: &SnapshotPolicy,
) -> Result<bgx_adapter_v2_validator::media::UploadProvenance> {
    let path = upload_path(workspace, scope, configuration_digest, reference)?;
    let value: serde_json::Value = serde_json::from_slice(&bounded(&path, "maxStateBytes")?)
        .map_err(|_| fail("INVALID_INPUT", "Upload receipt is corrupt"))?;
    policy.check(&value)?;
    if value.as_object().is_none_or(|o| o.len() != 6)
        || value["package"] != serde_json::to_value(&scope.package).unwrap()
        || value["bindingId"] != scope.binding_id
        || value["configurationDigest"] != configuration_digest
        || value["uploadRef"] != reference
        || value["mediaId"].as_str().is_none_or(str::is_empty)
        || value["mediaDigest"].as_str().is_none_or(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(fail(
            "PERMISSION_DENIED",
            "Upload receipt belongs to another binding or configuration",
        ));
    }
    Ok(bgx_adapter_v2_validator::media::UploadProvenance {
        media_digest: value["mediaDigest"].as_str().unwrap().into(),
        package: scope.package.clone(),
        binding_id: scope.binding_id.clone(),
        configuration_digest: configuration_digest.into(),
        media_id: value["mediaId"].as_str().unwrap().into(),
    })
}
fn upload_path(
    workspace: &DevelopmentWorkspace,
    scope: &crate::media::LeaseScope,
    configuration_digest: &str,
    reference: &str,
) -> Result<PathBuf> {
    let directory = workspace.root().join("upload-receipts");
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(_) => {
            return Err(fail(
                "RESOURCE_UNAVAILABLE",
                "Upload receipt directory is unavailable",
            ))
        }
    }
    if directory
        .canonicalize()
        .ok()
        .as_deref()
        .and_then(Path::parent)
        != Some(workspace.root())
    {
        return Err(fail(
            "PERMISSION_DENIED",
            "Upload receipt directory escapes development workspace",
        ));
    }
    let key = serde_json::to_vec(&serde_json::json!([
        scope.package,
        scope.binding_id,
        configuration_digest,
        reference
    ]))
    .unwrap();
    let path = directory.join(format!("{}.json", hash_bytes(&key)));
    if path.exists()
        && path.canonicalize().ok().as_deref().and_then(Path::parent) != Some(directory.as_path())
    {
        return Err(fail(
            "PERMISSION_DENIED",
            "Upload receipt file escapes development workspace",
        ));
    }
    Ok(path)
}
pub struct LoadedPackage {
    pub(crate) root: PathBuf,
    pub(crate) verified: Arc<VerifiedPackage>,
}
impl LoadedPackage {
    pub fn verified(&self) -> &VerifiedPackage {
        &self.verified
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn profile_bytes(&self) -> Result<Vec<u8>> {
        let entry = self.verified.manifest()["entry"].as_str().unwrap();
        let path = self.root.join(entry);
        reject_package_path(&path)?;
        let bytes = bounded(&path, "maxStateBytes")?;
        if hash_bytes(&bytes) != self.verified.manifest()["artifactSha256"] {
            return Err(fail(
                "INVALID_PACKAGE",
                "Profile changed after package verification",
            ));
        }
        Ok(bytes)
    }
    /// Streams every file hash and rejects links/unlisted files. No trusted
    /// digest may be supplied by callers in place of measuring actual bytes.
    pub fn load(root: &Path, trust: &TrustContext) -> Result<Self> {
        if !root.is_absolute() {
            return Err(fail(
                "INVALID_PACKAGE",
                "Package directory must be explicit and absolute",
            ));
        }
        reject_package_path(root)?;
        preflight_package_tree(root)?;
        let root = root
            .canonicalize()
            .map_err(|_| fail("INVALID_PACKAGE", "Package directory is unavailable"))?;
        let protocol: serde_json::Value = serde_json::from_str(include_str!(
            "../../../protocol/vendor-adapter-protocol.json"
        ))
        .unwrap();
        let inventory_name = protocol["signature"]["hashManifest"].as_str().unwrap();
        let signature_name = protocol["signature"]["signatureFile"].as_str().unwrap();
        let manifest = bounded(&root.join("manifest.json"), "maxStateBytes")?;
        let inventory = bounded(&root.join(inventory_name), "maxFrameBytes")?;
        let declared = parse_hash_inventory(
            std::str::from_utf8(&inventory)
                .map_err(|_| fail("INVALID_PACKAGE", "Package inventory is not UTF8"))?,
        )
        .map_err(|_| fail("INVALID_PACKAGE", "Package inventory is invalid"))?;
        let signature = match fs::metadata(root.join(signature_name)) {
            Ok(_) => Some(
                String::from_utf8(bounded(&root.join(signature_name), "maxStateBytes")?)
                    .map_err(|_| fail("INVALID_PACKAGE", "Package signature is not UTF8"))?,
            ),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(_) => return Err(fail("INVALID_PACKAGE", "Package signature is unavailable")),
        };
        let mut measured = BTreeMap::new();
        let mut pending = vec![root.clone()];
        let mut inventory_bytes = 0usize;
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)
                .map_err(|_| fail("INVALID_PACKAGE", "Package file list is unavailable"))?
            {
                let entry =
                    entry.map_err(|_| fail("INVALID_PACKAGE", "Package file is unavailable"))?;
                let file_type = entry
                    .file_type()
                    .map_err(|_| fail("INVALID_PACKAGE", "Package file type is unavailable"))?;
                if file_type.is_symlink() {
                    return Err(fail("INVALID_PACKAGE", "Package links are forbidden"));
                }
                let path = entry.path();
                let resolved = path
                    .canonicalize()
                    .map_err(|_| fail("INVALID_PACKAGE", "Package path cannot be resolved"))?;
                if !resolved.starts_with(&root) {
                    return Err(fail(
                        "INVALID_PACKAGE",
                        "Package file escapes its directory",
                    ));
                }
                if file_type.is_dir() {
                    pending.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    return Err(fail(
                        "INVALID_PACKAGE",
                        "Package entry is not a regular file",
                    ));
                }
                let name = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_str()
                    .ok_or_else(|| fail("INVALID_PACKAGE", "Package filename is not UTF8"))?
                    .replace('\\', "/");
                if name == inventory_name || name == signature_name {
                    continue;
                }
                inventory_bytes = inventory_bytes.saturating_add(name.len() + 65);
                if inventory_bytes
                    > resource_policy()["limits"]["maxFrameBytes"]
                        .as_u64()
                        .unwrap() as usize
                {
                    return Err(fail(
                        "BUDGET_EXCEEDED",
                        "Package inventory exceeds public frame budget",
                    ));
                }
                if !declared.contains_key(&name) {
                    return Err(fail("INVALID_PACKAGE", "Package contains an unlisted file"));
                }
                let mut file = File::open(&path)
                    .map_err(|_| fail("INVALID_PACKAGE", "Package file cannot be read"))?;
                let mut hash = Sha256::new();
                let mut chunk = vec![
                    0;
                    resource_policy()["limits"]["maxMediaChunkBytes"]
                        .as_u64()
                        .unwrap() as usize
                ];
                loop {
                    let n = file
                        .read(&mut chunk)
                        .map_err(|_| fail("INVALID_PACKAGE", "Package hashing failed"))?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&chunk[..n]);
                }
                measured.insert(name, hex::encode(hash.finalize()));
            }
        }
        if measured.get("manifest.json") != Some(&hash_bytes(&manifest)) {
            return Err(fail(
                "INVALID_PACKAGE",
                "Package manifest changed during verification",
            ));
        }
        let verified = verify_package(
            &PackageContents {
                manifest_bytes: &manifest,
                hash_manifest_bytes: &inventory,
                signature: signature.as_deref(),
                measured_file_hashes: &measured,
            },
            trust,
        )
        .map_err(|e| crate::checkpoint::CheckpointError {
            code: e.code.into(),
            message: "Package verification failed",
        })?;
        Ok(Self {
            root,
            verified: Arc::new(verified),
        })
    }
}
fn package_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn reject_package_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(fail(
            "INVALID_PACKAGE",
            "Package path must be explicit, absolute and normalized",
        ));
    }
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|_| fail("INVALID_PACKAGE", "Package path is unavailable"))?;
        if package_link(&metadata) {
            return Err(fail(
                "INVALID_PACKAGE",
                "Package links and reparse points are forbidden",
            ));
        }
    }
    Ok(())
}
/// Inspect directory entries without opening file contents. This runs before
/// manifest/inventory/signature parsing so native callers share the CLI guard.
fn preflight_package_tree(root: &Path) -> Result<()> {
    let mut pending = vec![root.to_owned()];
    let mut path_bytes = 0usize;
    let budget = resource_policy()["limits"]["maxFrameBytes"]
        .as_u64()
        .unwrap() as usize;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)
            .map_err(|_| fail("INVALID_PACKAGE", "Package directory cannot be listed"))?
        {
            let entry =
                entry.map_err(|_| fail("INVALID_PACKAGE", "Package entry cannot be inspected"))?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|_| fail("INVALID_PACKAGE", "Package entry is unavailable"))?;
            if package_link(&metadata) {
                return Err(fail(
                    "INVALID_PACKAGE",
                    "Package links and reparse points are forbidden",
                ));
            }
            path_bytes = path_bytes
                .saturating_add(entry.path().as_os_str().len())
                .saturating_add(65);
            if path_bytes > budget {
                return Err(fail(
                    "BUDGET_EXCEEDED",
                    "Package file list exceeds public frame budget",
                ));
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if !metadata.is_file() {
                return Err(fail(
                    "INVALID_PACKAGE",
                    "Package entry is not a regular file",
                ));
            }
        }
    }
    Ok(())
}

fn bounded(path: &Path, budget: &str) -> Result<Vec<u8>> {
    let max = resource_policy()["limits"][budget].as_u64().unwrap();
    let file = File::open(path)
        .map_err(|_| fail("INVALID_PACKAGE", "Required package file is unavailable"))?;
    let mut bytes = vec![];
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("INVALID_PACKAGE", "Required package file cannot be read"))?;
    if bytes.len() as u64 > max {
        return Err(fail(
            "BUDGET_EXCEEDED",
            "Required package file exceeds public resource budget",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod package_path_tests {
    use super::*;
    fn link_directory(source: &Path, destination: &Path) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(destination)
                .arg(source)
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "Junction fixture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(source, destination).unwrap();
    }
    #[test]
    fn native_loader_rejects_root_and_child_redirects_before_manifest_read() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("package");
        let outside = temp.path().join("outside");
        fs::create_dir(&package).unwrap();
        fs::create_dir(&outside).unwrap();
        // Both directories intentionally have NO manifest. A contents-first
        // implementation would return "Required package file is unavailable".
        let root_link = temp.path().join("linked-package");
        link_directory(&package, &root_link);
        let error = LoadedPackage::load(&root_link, &TrustContext::default())
            .err()
            .unwrap();
        assert_eq!(
            error.message,
            "Package links and reparse points are forbidden"
        );
        fs::remove_dir(&root_link).unwrap();
        let child_link = package.join("redirect");
        link_directory(&outside, &child_link);
        let error = LoadedPackage::load(&package, &TrustContext::default())
            .err()
            .unwrap();
        assert_eq!(
            error.message,
            "Package links and reparse points are forbidden"
        );
        fs::remove_dir(&child_link).unwrap();
        assert!(outside.is_dir());
    }
}
