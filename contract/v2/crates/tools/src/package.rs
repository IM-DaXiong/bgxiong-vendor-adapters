//! Runtime package preparation. All contract and trust decisions use the public
//! validator; this module only measures files, stages them, and signs inventories.
use bgx_adapter_v2_ports::{
    resource_policy,
    wire::guest::{parse_hash_inventory, valid_package_path},
};
use bgx_adapter_v2_reference_host::config::LoadedPackage;
use bgx_adapter_v2_validator::{
    hash_bytes,
    package::{verify_package, PackageContents, TrustContext},
    ContractError, SchemaRegistry,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub struct SigningOptions<'a> {
    pub secret_key: &'a Path,
    pub password_file: Option<&'a Path>,
    pub unencrypted_key: bool,
}

const FIX: &str = "Correct the indicated package field or file and run the command again.";
const SIGN_FIX: &str = "Supply an explicit key and exactly one of --unencrypted-key or --password-file, plus matching explicit publisher trust.";

fn invalid(path: impl Into<String>, message: impl Into<String>) -> crate::Error {
    crate::error("INVALID_PACKAGE", path, message, FIX)
}
fn contract(error: ContractError) -> crate::Error {
    crate::error(error.code, error.path, error.reason, FIX)
}
pub(crate) fn names() -> (&'static str, &'static str) {
    static PROTOCOL: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let protocol = PROTOCOL.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../protocol/vendor-adapter-protocol.json"
        ))
        .expect("published protocol")
    });
    (
        protocol["signature"]["hashManifest"].as_str().unwrap(),
        protocol["signature"]["signatureFile"].as_str().unwrap(),
    )
}
fn budget(name: &str) -> u64 {
    resource_policy()["limits"][name].as_u64().unwrap()
}

fn reject_link(path: &Path) -> crate::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).map_err(|e| {
        invalid(
            path.display().to_string(),
            format!("Cannot inspect package path: {e}"),
        )
    })?;
    let linked = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        linked || metadata.file_attributes() & 0x400 != 0
    };
    if linked {
        return Err(invalid(
            path.display().to_string(),
            "Symbolic links and reparse points are forbidden",
        ));
    }
    Ok(metadata)
}
fn root_directory(path: &Path) -> crate::Result<PathBuf> {
    if !reject_link(path)?.is_dir() {
        return Err(invalid(
            path.display().to_string(),
            "Package root must be a directory",
        ));
    }
    path.canonicalize().map_err(|e| {
        invalid(
            path.display().to_string(),
            format!("Cannot resolve package directory: {e}"),
        )
    })
}
fn selected_file(root: &Path, name: &str) -> crate::Result<PathBuf> {
    if !valid_package_path(name) {
        return Err(invalid(
            "/entry",
            "Package path must be a safe relative path",
        ));
    }
    let mut path = root.to_path_buf();
    let parts: Vec<_> = name.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let metadata = reject_link(&path)?;
        if index + 1 == parts.len() {
            if !metadata.is_file() {
                return Err(invalid(name, "Package entry must be a regular file"));
            }
        } else if !metadata.is_dir() {
            return Err(invalid(name, "Package parent must be a directory"));
        }
        if !path
            .canonicalize()
            .map_err(|e| invalid(name, format!("Cannot resolve package path: {e}")))?
            .starts_with(root)
        {
            return Err(invalid(name, "Package path escapes its root"));
        }
    }
    Ok(path)
}
fn read_bounded(path: &Path, limit: u64) -> crate::Result<Vec<u8>> {
    let metadata = reject_link(path)?;
    if !metadata.is_file() {
        return Err(invalid(
            path.display().to_string(),
            "Expected a regular file",
        ));
    }
    let file = File::open(path).map_err(|e| {
        invalid(
            path.display().to_string(),
            format!("Cannot read package file: {e}"),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|e| {
        invalid(
            path.display().to_string(),
            format!("Cannot read package file: {e}"),
        )
    })?;
    if bytes.len() as u64 > limit {
        return Err(crate::error(
            "BUDGET_EXCEEDED",
            path.display().to_string(),
            "File exceeds the public resource budget",
            FIX,
        ));
    }
    Ok(bytes)
}
fn manifest(root: &Path) -> crate::Result<(Vec<u8>, Value, String)> {
    let bytes = read_bounded(
        &selected_file(root, "manifest.json")?,
        budget("maxStateBytes"),
    )?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| invalid("/manifest", format!("Invalid manifest JSON: {e}")))?;
    SchemaRegistry::validate_public("manifest", &value).map_err(contract)?;
    let entry = value["entry"].as_str().unwrap().to_owned();
    let (inventory, signature) = names();
    if !valid_package_path(&entry)
        || ["manifest.json", inventory, signature].contains(&entry.as_str())
    {
        return Err(invalid(
            "/entry",
            "Entry must be a distinct safe runtime artifact path",
        ));
    }
    Ok((bytes, value, entry))
}
fn digest(path: &Path) -> crate::Result<String> {
    let mut file = File::open(path).map_err(|e| {
        invalid(
            path.display().to_string(),
            format!("Cannot hash package file: {e}"),
        )
    })?;
    let mut hash = Sha256::new();
    let mut chunk = vec![0; budget("maxMediaChunkBytes") as usize];
    loop {
        let count = file.read(&mut chunk).map_err(|e| {
            invalid(
                path.display().to_string(),
                format!("Cannot hash package file: {e}"),
            )
        })?;
        if count == 0 {
            break;
        }
        hash.update(&chunk[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}
fn optional_file(root: &Path, name: &str) -> crate::Result<Option<PathBuf>> {
    match fs::symlink_metadata(root.join(name)) {
        Ok(_) => selected_file(root, name).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(invalid(
            name,
            format!("Cannot inspect optional package file: {e}"),
        )),
    }
}
fn atomic_write(path: &Path, bytes: &[u8]) -> crate::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid(path.display().to_string(), "File has no parent directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
        invalid(
            path.display().to_string(),
            format!("Cannot stage file: {e}"),
        )
    })?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| {
            invalid(
                path.display().to_string(),
                format!("Cannot write file: {e}"),
            )
        })?;
    temporary.persist(path).map_err(|e| {
        invalid(
            path.display().to_string(),
            format!("Cannot publish file: {}", e.error),
        )
    })?;
    Ok(())
}

/// Refresh only the two runtime inventory entries. Development sources are not
/// runtime files. Publisher identity is preserved, and existing signatures are
/// never silently invalidated or removed.
pub fn refresh_inventory(project: &Path) -> crate::Result<()> {
    let root = root_directory(project)?;
    let (inventory_name, signature_name) = names();
    if optional_file(&root, signature_name)?.is_some() {
        return Err(invalid(signature_name, "Existing signature would be invalidated; rebuild an explicitly unsigned source and sign the new package"));
    }
    let (_, mut value, entry) = manifest(&root)?;
    let artifact = selected_file(&root, &entry)?;
    let artifact_hash = digest(&artifact)?;
    value["artifactSha256"] = Value::String(artifact_hash.clone());
    SchemaRegistry::validate_public("manifest", &value).map_err(contract)?;
    let mut bytes = serde_json::to_vec_pretty(&value)
        .map_err(|e| invalid("/manifest", format!("Cannot encode manifest: {e}")))?;
    bytes.push(b'\n');
    let inventory = BTreeMap::from([
        ("manifest.json".to_owned(), hash_bytes(&bytes)),
        (entry, artifact_hash),
    ])
    .into_iter()
    .map(|(file, hash)| format!("{hash} {file}\n"))
    .collect::<String>();
    parse_hash_inventory(&inventory).map_err(|why| invalid(inventory_name, why))?;
    if bytes.len() as u64 > budget("maxStateBytes")
        || inventory.len() as u64 > budget("maxFrameBytes")
    {
        return Err(crate::error(
            "BUDGET_EXCEEDED",
            "/manifest",
            "Generated package metadata exceeds the public resource budget",
            FIX,
        ));
    }
    if let Some(path) = optional_file(&root, inventory_name)? {
        reject_link(&path)?;
    }
    atomic_write(&root.join("manifest.json"), &bytes)?;
    atomic_write(&root.join(inventory_name), inventory.as_bytes())
}

/// Preflight the entire runtime tree before any manifest/inventory content is
/// read. LoadedPackage then remeasures the same files using the shared loader.
fn preflight_tree(root: &Path) -> crate::Result<BTreeMap<String, String>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeMap::new();
    let mut all_names = BTreeSet::new();
    let mut inventory_size = 0u64;
    let (inventory_name, signature_name) = names();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|e| {
            invalid(
                directory.display().to_string(),
                format!("Cannot list package: {e}"),
            )
        })? {
            let entry = entry.map_err(|e| {
                invalid(
                    directory.display().to_string(),
                    format!("Cannot list package entry: {e}"),
                )
            })?;
            let path = entry.path();
            let metadata = reject_link(&path)?;
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .ok_or_else(|| invalid("/", "Package names must be UTF8"))?
                .replace('\\', "/");
            if !valid_package_path(&name) || !all_names.insert(name.clone()) {
                return Err(invalid(name, "Invalid or duplicate package path"));
            }
            if !path
                .canonicalize()
                .map_err(|e| invalid(&name, format!("Cannot resolve package path: {e}")))?
                .starts_with(root)
            {
                return Err(invalid(name, "Package path escapes root"));
            }
            inventory_size = inventory_size.saturating_add(name.len() as u64 + 65);
            if inventory_size > budget("maxFrameBytes") {
                return Err(crate::error(
                    "BUDGET_EXCEEDED",
                    "/",
                    "Package file list exceeds public resource budget",
                    FIX,
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                if name != inventory_name && name != signature_name {
                    files.insert(name, path);
                }
            } else {
                return Err(invalid(
                    name,
                    "Package entries must be regular files or directories",
                ));
            }
        }
    }
    // File content is touched only after every path has passed the link check.
    files
        .into_iter()
        .map(|(name, path)| digest(&path).map(|hash| (name, hash)))
        .collect()
}

pub fn load_checked(project: &Path, trust: &TrustContext) -> crate::Result<LoadedPackage> {
    let root = root_directory(project)?;
    let measured = preflight_tree(&root)?;
    let (manifest_bytes, _, entry) = manifest(&root)?;
    let (inventory_name, signature_name) = names();
    let expected: BTreeSet<_> = ["manifest.json".to_owned(), entry].into_iter().collect();
    if measured.keys().cloned().collect::<BTreeSet<_>>() != expected {
        return Err(invalid("/manifest.sha256", "Runtime package must contain only manifest and entry, plus inventory and optional signature"));
    }
    let inventory = read_bounded(
        &selected_file(&root, inventory_name)?,
        budget("maxFrameBytes"),
    )?;
    let signature = optional_file(&root, signature_name)?
        .map(|path| read_bounded(&path, budget("maxStateBytes")))
        .transpose()?;
    let signature = signature
        .as_ref()
        .map(|bytes| {
            std::str::from_utf8(bytes)
                .map_err(|_| invalid(signature_name, "Signature must be UTF8"))
        })
        .transpose()?;
    verify_package(
        &PackageContents {
            manifest_bytes: &manifest_bytes,
            hash_manifest_bytes: &inventory,
            signature,
            measured_file_hashes: &measured,
        },
        trust,
    )
    .map_err(contract)?;
    LoadedPackage::load(&root, trust).map_err(|e| {
        crate::error(
            "INVALID_PACKAGE",
            root.display().to_string(),
            format!("{}: {}", e.code, e.message),
            FIX,
        )
    })
}

fn copy_file(root: &Path, name: &str, destination: &Path) -> crate::Result<()> {
    let source = selected_file(root, name)?;
    let target = destination.join(name);
    fs::create_dir_all(target.parent().unwrap())
        .map_err(|e| invalid(name, format!("Cannot create runtime directory: {e}")))?;
    let mut input =
        File::open(source).map_err(|e| invalid(name, format!("Cannot read runtime file: {e}")))?;
    let mut output = File::options()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| invalid(name, format!("Cannot create runtime file: {e}")))?;
    std::io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .map_err(|e| invalid(name, format!("Cannot copy runtime file: {e}")))?;
    Ok(())
}
fn prepare_runtime(
    project: &Path,
    destination: &Path,
    signing: Option<&SigningOptions<'_>>,
    trust: &TrustContext,
) -> crate::Result<()> {
    let root = root_directory(project)?;
    let (manifest_bytes, value, entry) = manifest(&root)?;
    let (inventory_name, signature_name) = names();
    let inventory = read_bounded(
        &selected_file(&root, inventory_name)?,
        budget("maxFrameBytes"),
    )?;
    let inventory_text = std::str::from_utf8(&inventory)
        .map_err(|_| invalid(inventory_name, "Inventory must be UTF8"))?;
    let declared =
        parse_hash_inventory(inventory_text).map_err(|why| invalid(inventory_name, why))?;
    let expected: BTreeSet<_> = ["manifest.json".to_owned(), entry.clone()]
        .into_iter()
        .collect();
    if declared.keys().cloned().collect::<BTreeSet<_>>() != expected {
        return Err(invalid(
            inventory_name,
            "Inventory must list exactly manifest.json and the runtime entry",
        ));
    }
    copy_file(&root, "manifest.json", destination)?;
    copy_file(&root, &entry, destination)?;
    copy_file(&root, inventory_name, destination)?;
    let measured = BTreeMap::from([
        (
            "manifest.json".to_owned(),
            digest(&destination.join("manifest.json"))?,
        ),
        (entry.clone(), digest(&destination.join(&entry))?),
    ]);
    if measured != declared || measured.get("manifest.json") != Some(&hash_bytes(&manifest_bytes)) {
        return Err(invalid(
            inventory_name,
            "Inventory or files changed; rebuild or refresh the inventory before packaging",
        ));
    }
    if measured.get(&entry).map(String::as_str) != value["artifactSha256"].as_str() {
        return Err(invalid(
            "/artifactSha256",
            "Runtime artifact digest differs from the manifest; rebuild before packaging",
        ));
    }
    match signing {
        Some(options) => sign_inventory(destination, &value, options, trust)?,
        None => {
            if optional_file(&root, signature_name)?.is_some() {
                copy_file(&root, signature_name, destination)?;
            }
        }
    }
    load_checked(destination, trust)?;
    Ok(())
}

/// This performs static package verification only. The caller must invoke the
/// worker/reference host to claim a successful WASM capabilities check.
pub fn stage_runtime(project: &Path, trust: &TrustContext) -> crate::Result<tempfile::TempDir> {
    let temporary = tempfile::Builder::new()
        .prefix("bgx-v2-runtime-")
        .tempdir()
        .map_err(|e| invalid("/", format!("Cannot create runtime staging directory: {e}")))?;
    prepare_runtime(project, temporary.path(), None, trust)?;
    Ok(temporary)
}

fn sign_inventory(
    root: &Path,
    manifest: &Value,
    options: &SigningOptions<'_>,
    trust: &TrustContext,
) -> crate::Result<()> {
    let fail = |path: &str, message: &str| crate::error("UNAUTHORIZED", path, message, SIGN_FIX);
    if options.unencrypted_key == options.password_file.is_some() {
        return Err(fail(
            "/signing",
            "Signing requires exactly one explicit key decryption mode",
        ));
    }
    let publisher_id = manifest["publisherId"].as_str().ok_or_else(|| {
        fail(
            "/publisherId",
            "Signing requires an explicit publisherId in the manifest",
        )
    })?;
    let key_id = manifest["publisherKeyId"].as_str().ok_or_else(|| {
        fail(
            "/publisherKeyId",
            "Signing requires an explicit publisherKeyId in the manifest",
        )
    })?;
    let publisher = trust
        .publishers
        .iter()
        .find(|publisher| publisher.publisher_id == publisher_id && publisher.key_id == key_id)
        .ok_or_else(|| {
            fail(
                "/publisherKeyId",
                "Signing requires matching explicit publisher trust",
            )
        })?;
    let key_bytes = Zeroizing::new(read_bounded(options.secret_key, budget("maxStateBytes"))?);
    let key_text = std::str::from_utf8(&key_bytes).map_err(|_| {
        fail(
            "/signing/key",
            "Secret key must be a valid minisign key file",
        )
    })?;
    let boxed = minisign::SecretKeyBox::from_string(key_text)
        .map_err(|_| fail("/signing/key", "Secret key cannot be decoded"))?;
    let key = if options.unencrypted_key {
        boxed
            .into_unencrypted_secret_key()
            .map_err(|_| fail("/signing/key", "Secret key is encrypted or invalid"))?
    } else {
        let bytes = Zeroizing::new(read_bounded(
            options.password_file.unwrap(),
            budget("maxStateBytes"),
        )?);
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| fail("/signing/password", "Password file must be UTF8"))?;
        // Password files may have one conventional final newline; other bytes
        // (including spaces) are significant. Never pass None to minisign.
        let password = Zeroizing::new(
            text.strip_suffix("\r\n")
                .or_else(|| text.strip_suffix('\n'))
                .unwrap_or(text)
                .to_owned(),
        );
        boxed
            .into_secret_key(Some(password.to_string()))
            .map_err(|_| {
                fail(
                    "/signing/key",
                    "Secret key could not be decrypted with the explicit password file",
                )
            })?
    };
    let public = minisign::PublicKey::from_secret_key(&key)
        .map_err(|_| fail("/signing/key", "Cannot derive signing public key"))?;
    // PublisherTrust uses the complete .pub document, as required by the public
    // validator. Compare decoded key bytes so its untrusted comment has no role
    // in key identity.
    let trusted_public = minisign::PublicKeyBox::from_string(&publisher.public_key)
        .and_then(|boxed| boxed.into_public_key())
        .map_err(|_| {
            fail(
                "/signing/publicKey",
                "Explicit publisher public key must be a complete minisign .pub document",
            )
        })?;
    if public.to_bytes() != trusted_public.to_bytes() {
        return Err(fail(
            "/signing/key",
            "Signing key does not match the explicitly trusted public key",
        ));
    }
    let (inventory_name, signature_name) = names();
    let inventory = File::open(root.join(inventory_name))
        .map_err(|_| invalid(inventory_name, "Cannot read staged inventory"))?;
    let signature = minisign::sign(
        Some(&public),
        &key,
        inventory,
        Some("bgx-adapter-v2 package inventory"),
        Some("bgx-adapter-v2 package signature"),
    )
    .map_err(|_| fail("/signing", "Minisign could not sign the package inventory"))?;
    atomic_write(&root.join(signature_name), signature.to_string().as_bytes())
}

/// All validation finishes in a temporary runtime directory. create_dir claims
/// the destination without replacement on every platform. Publication is not an
/// atomic directory rename: consumers must wait for this command to succeed.
pub fn pack(
    _sdk: &Path,
    project: &Path,
    out: &Path,
    signing: Option<&SigningOptions<'_>>,
    trust: &TrustContext,
) -> crate::Result<()> {
    let parent = out.parent().ok_or_else(|| {
        invalid(
            out.display().to_string(),
            "Output directory needs an explicit parent",
        )
    })?;
    let parent = root_directory(parent)?;
    let name = out.file_name().ok_or_else(|| {
        invalid(
            out.display().to_string(),
            "Output directory needs a final name",
        )
    })?;
    let destination = parent.join(name);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(invalid(
            destination.display().to_string(),
            "Output already exists; choose a new directory",
        ));
    }
    let temporary = tempfile::Builder::new()
        .prefix(".bgx-v2-pack-")
        .tempdir_in(&parent)
        .map_err(|e| {
            invalid(
                parent.display().to_string(),
                format!("Cannot stage package: {e}"),
            )
        })?;
    prepare_runtime(project, temporary.path(), signing, trust)?;
    fs::create_dir(&destination).map_err(|e| {
        invalid(
            destination.display().to_string(),
            format!("Cannot claim new output directory: {e}"),
        )
    })?;
    let result: crate::Result<()> = (|| {
        let (_, _, entry) = manifest(temporary.path())?;
        let (inventory_name, signature_name) = names();
        for name in ["manifest.json", entry.as_str(), inventory_name] {
            copy_file(temporary.path(), name, &destination)?;
        }
        if optional_file(temporary.path(), signature_name)?.is_some() {
            copy_file(temporary.path(), signature_name, &destination)?;
        }
        load_checked(&destination, trust)?;
        Ok(())
    })();
    if let Err(error) = result {
        // Only remove the exact newly claimed path, never a caller's existing
        // output or a link swapped in by another process.
        let cleanup = reject_link(&destination).and_then(|metadata| {
            if !metadata.is_dir() {
                return Err(invalid(
                    destination.display().to_string(),
                    "Failed output is no longer a directory; cleanup refused",
                ));
            }
            let resolved = destination.canonicalize().map_err(|e| {
                invalid(destination.display().to_string(), format!("Cannot resolve failed output for cleanup: {e}"))
            })?;
            if resolved == parent || !resolved.starts_with(&parent) || resolved != destination {
                return Err(invalid(destination.display().to_string(), "Failed output no longer resolves to the newly claimed directory; cleanup refused"));
            }
            preflight_tree(&destination)?;
            fs::remove_dir_all(&destination).map_err(|e| {
                invalid(
                    destination.display().to_string(),
                    format!("Cannot clean failed output: {e}"),
                )
            })
        });
        if let Err(cleanup) = cleanup {
            return Err(invalid(
                destination.display().to_string(),
                format!(
                    "{}; failed output cleanup also failed: {}",
                    error.message, cleanup.message
                ),
            ));
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgx_adapter_v2_validator::package::PublisherTrust;

    fn fixture() -> tempfile::TempDir {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("manifest.json"),
            include_bytes!("../../../templates/rust/manifest.json"),
        )
        .unwrap();
        // These bytes test static package integrity only, never WASM execution.
        fs::write(
            project.path().join("adapter.wasm"),
            b"static integrity test artifact",
        )
        .unwrap();
        fs::write(project.path().join("source.rs"), b"not a runtime file").unwrap();
        refresh_inventory(project.path()).unwrap();
        project
    }
    fn edit_manifest(project: &Path, edit: impl FnOnce(&mut Value)) {
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(project.join("manifest.json")).unwrap()).unwrap();
        edit(&mut manifest);
        fs::write(
            project.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }
    fn signed_fixture() -> (tempfile::TempDir, PathBuf, TrustContext) {
        let project = fixture();
        edit_manifest(project.path(), |m| {
            m["pluginId"] = "tests.publisher.echo".into();
            m["publisherId"] = "tests.publisher".into();
            m["publisherKeyId"] = "test-key".into();
        });
        refresh_inventory(project.path()).unwrap();
        let keypair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let key = project.path().join("explicit-test.key");
        fs::write(
            &key,
            keypair.sk.to_box(Some("test key")).unwrap().to_bytes(),
        )
        .unwrap();
        let trust = TrustContext {
            publishers: vec![PublisherTrust {
                publisher_id: "tests.publisher".into(),
                key_id: "test-key".into(),
                public_key: keypair.pk.to_box().unwrap().into_string(),
                plugin_prefixes: vec!["tests.publisher".into()],
            }],
            is_fresh: true,
            ..TrustContext::default()
        };
        (project, key, trust)
    }

    #[test]
    fn runtime_staging_excludes_sources_and_existing_output_is_preserved() {
        let project = fixture();
        let trust = TrustContext::default();
        let runtime = stage_runtime(project.path(), &trust).unwrap();
        assert_eq!(fs::read_dir(runtime.path()).unwrap().count(), 3);
        assert!(!runtime.path().join("source.rs").exists());
        load_checked(runtime.path(), &trust).unwrap();
        let outputs = tempfile::tempdir().unwrap();
        let out = outputs.path().join("release");
        pack(project.path(), project.path(), &out, None, &trust).unwrap();
        let before = fs::read(out.join("manifest.json")).unwrap();
        fs::write(out.join("sentinel"), b"keep my existing output").unwrap();
        assert!(pack(project.path(), project.path(), &out, None, &trust).is_err());
        assert_eq!(fs::read(out.join("manifest.json")).unwrap(), before);
        assert_eq!(
            fs::read(out.join("sentinel")).unwrap(),
            b"keep my existing output"
        );
    }

    #[test]
    fn byte_tampering_duplicate_inventory_and_unsafe_paths_fail() {
        let trust = TrustContext::default();
        let project = fixture();
        fs::write(project.path().join("adapter.wasm"), b"changed").unwrap();
        assert!(stage_runtime(project.path(), &trust).is_err());
        refresh_inventory(project.path()).unwrap();
        let inventory = project.path().join(names().0);
        let valid = fs::read_to_string(&inventory).unwrap();
        fs::write(&inventory, format!("{valid}{valid}")).unwrap();
        assert!(stage_runtime(project.path(), &trust).is_err());
        fs::write(&inventory, format!("{} ../outside.wasm\n", "0".repeat(64))).unwrap();
        assert!(stage_runtime(project.path(), &trust).is_err());
        fs::write(&inventory, valid).unwrap();
        edit_manifest(project.path(), |m| m["entry"] = "../outside.wasm".into());
        assert!(refresh_inventory(project.path()).is_err());
    }

    #[test]
    fn runtime_loader_rejects_unlisted_files_and_manifest_inventory_mismatch() {
        let project = fixture();
        let trust = TrustContext::default();
        let runtime = stage_runtime(project.path(), &trust).unwrap();
        fs::write(runtime.path().join("unlisted.txt"), b"extra").unwrap();
        assert!(load_checked(runtime.path(), &trust).is_err());
        fs::remove_file(runtime.path().join("unlisted.txt")).unwrap();
        edit_manifest(runtime.path(), |m| {
            m["displayName"] = "Changed after hashing".into()
        });
        assert!(load_checked(runtime.path(), &trust).is_err());
    }

    #[test]
    fn minisign_roundtrip_requires_matching_explicit_trust_and_preserves_publisher() {
        let (project, key, mut trust) = signed_fixture();
        let before = fs::read(project.path().join("manifest.json")).unwrap();
        let options = SigningOptions {
            secret_key: &key,
            password_file: None,
            unencrypted_key: true,
        };
        let outputs = tempfile::tempdir().unwrap();
        let out = outputs.path().join("signed");
        pack(project.path(), project.path(), &out, Some(&options), &trust).unwrap();
        let loaded = load_checked(&out, &trust).unwrap();
        assert!(loaded.verified().is_trusted());
        assert_eq!(
            loaded.verified().manifest()["publisherId"],
            "tests.publisher"
        );
        assert_eq!(
            fs::read(project.path().join("manifest.json")).unwrap(),
            before
        );
        assert!(load_checked(&out, &TrustContext::default()).is_err());
        assert!(refresh_inventory(&out).is_err());
        trust.publishers[0].plugin_prefixes = vec!["unrelated.publisher".into()];
        assert!(load_checked(&out, &trust).is_err());
        trust.publishers[0].plugin_prefixes = vec!["tests.publisher".into()];
        trust.publishers[0].public_key = minisign::KeyPair::generate_unencrypted_keypair()
            .unwrap()
            .pk
            .to_box()
            .unwrap()
            .into_string();
        assert!(load_checked(&out, &trust).is_err());
        let rejected = outputs.path().join("wrong-key");
        assert!(pack(
            project.path(),
            project.path(),
            &rejected,
            Some(&options),
            &trust
        )
        .is_err());
        assert!(!rejected.exists());
    }

    #[test]
    fn signing_never_infers_password_mode_or_invents_publisher_identity() {
        let (project, key, mut trust) = signed_fixture();
        let outputs = tempfile::tempdir().unwrap();
        let out = outputs.path().join("no-mode");
        let options = SigningOptions {
            secret_key: &key,
            password_file: None,
            unencrypted_key: false,
        };
        assert!(pack(project.path(), project.path(), &out, Some(&options), &trust).is_err());
        assert!(!out.exists());
        let unsigned = fixture();
        let options = SigningOptions {
            secret_key: &key,
            password_file: None,
            unencrypted_key: true,
        };
        assert!(pack(
            unsigned.path(),
            unsigned.path(),
            &out,
            Some(&options),
            &trust
        )
        .is_err());
        let m: Value =
            serde_json::from_slice(&fs::read(unsigned.path().join("manifest.json")).unwrap())
                .unwrap();
        assert!(m["publisherId"].is_null());
        trust.publishers[0].public_key = trust.publishers[0]
            .public_key
            .lines()
            .nth(1)
            .unwrap()
            .to_owned();
        let error = pack(project.path(), project.path(), &out, Some(&options), &trust).unwrap_err();
        assert_eq!(error.path, "/signing/publicKey");
        assert!(!out.exists());
    }

    #[test]
    fn encrypted_minisign_key_uses_only_the_explicit_password_file() {
        let (project, key, mut trust) = signed_fixture();
        let password = Zeroizing::new("test secret with significant spaces ".to_owned());
        let keypair =
            minisign::KeyPair::generate_encrypted_keypair(Some(password.to_string())).unwrap();
        fs::write(
            &key,
            keypair
                .sk
                .to_box(Some("encrypted test key"))
                .unwrap()
                .to_bytes(),
        )
        .unwrap();
        trust.publishers[0].public_key = keypair.pk.to_box().unwrap().into_string();
        let password_file = project.path().join("explicit-test.password");
        let mut password_bytes = Zeroizing::new(password.as_bytes().to_vec());
        password_bytes.extend_from_slice(b"\r\n");
        fs::write(&password_file, &*password_bytes).unwrap();
        let options = SigningOptions {
            secret_key: &key,
            password_file: Some(&password_file),
            unencrypted_key: false,
        };
        let outputs = tempfile::tempdir().unwrap();
        let good = outputs.path().join("encrypted-signed");
        pack(
            project.path(),
            project.path(),
            &good,
            Some(&options),
            &trust,
        )
        .unwrap();
        assert!(load_checked(&good, &trust).unwrap().verified().is_trusted());
        fs::write(&password_file, b"incorrect password\n").unwrap();
        let bad = outputs.path().join("wrong-password");
        let error = pack(project.path(), project.path(), &bad, Some(&options), &trust).unwrap_err();
        assert_eq!(error.path, "/signing/key");
        assert!(!error.message.contains("incorrect password"));
        assert!(!error.message.contains(password.as_str()));
        assert!(!bad.exists());
        assert!(load_checked(&good, &trust).unwrap().verified().is_trusted());
    }

    #[test]
    fn linked_runtime_path_is_rejected_before_loading_contents() {
        let project = fixture();
        let outside = tempfile::tempdir().unwrap();
        let link = project.path().join("linked");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let status = std::process::Command::new("cmd.exe")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(outside.path())
                .creation_flags(0x08000000)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "test junction creation must succeed");
        }
        #[cfg(any(unix, windows))]
        {
            let mut m: Value =
                serde_json::from_slice(&fs::read(project.path().join("manifest.json")).unwrap())
                    .unwrap();
            m["entry"] = "linked/adapter.wasm".into();
            fs::write(outside.path().join("adapter.wasm"), b"outside content").unwrap();
            fs::write(
                project.path().join("manifest.json"),
                serde_json::to_vec(&m).unwrap(),
            )
            .unwrap();
            assert!(refresh_inventory(project.path()).is_err());
            assert!(load_checked(project.path(), &TrustContext::default()).is_err());
            // Remove only the link; TempDir must never traverse the outside tree.
            #[cfg(windows)]
            fs::remove_dir(&link).unwrap();
            #[cfg(unix)]
            fs::remove_file(&link).unwrap();
            assert_eq!(
                fs::read(outside.path().join("adapter.wasm")).unwrap(),
                b"outside content"
            );
        }
    }
}
