//! Author projects use only the explicitly selected public SDK and sealed mirrors.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use bgx_adapter_v2_ports::wire::guest::valid_package_path;
use bgx_adapter_v2_validator::{hash_bytes, SchemaRegistry};
use serde::Deserialize;
use serde_json::Value;

const EDITABLE: &[&str] = &["Cargo.toml", "src/lib.rs", "manifest.json", "README.md"];
const REPAIR_TEMPLATE: &str = "Restore the complete matching public SDK template; do not edit generated mirrors or their lock.";
const REPAIR_PATH: &str =
    "Use a normal directory and portable relative paths without symbolic links or reparse points.";
const REPAIR_CARGO: &str = "Read the project SDK command log; install the pinned Rust toolchain and wasm32-wasip2 target, and explicitly cache locked dependencies before an offline build.";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TemplateLock {
    schema_version: u64,
    sdk_release: String,
    editable_author_files: Vec<String>,
    author_dependency_lock: String,
    mirror_root: String,
    generated_files: Vec<GeneratedFile>,
    dependency_nodes: u64,
    network_policy: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GeneratedFile {
    target: String,
    sources: Vec<Source>,
    sha256: String,
    transform: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    path: String,
    sha256: String,
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> crate::Error {
    crate::error(
        "AUTHOR_IO",
        path.display().to_string(),
        error.to_string(),
        REPAIR_PATH,
    )
}

fn invalid(path: &Path, reason: impl Into<String>) -> crate::Error {
    crate::error(
        "INVALID_TEMPLATE",
        path.display().to_string(),
        reason,
        REPAIR_TEMPLATE,
    )
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and other reparse points also redirect filesystem access.
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// Inspect every existing component before canonicalization can conceal a link.
fn no_links(path: &Path) -> crate::Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(crate::error(
            "UNSAFE_PATH",
            path.display().to_string(),
            "An absolute normalized path is required.",
            REPAIR_PATH,
        ));
    }
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(crate::error(
                    "UNSAFE_PATH",
                    ancestor.display().to_string(),
                    "Symbolic links and filesystem reparse points are not allowed.",
                    REPAIR_PATH,
                ))
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(io_error(ancestor, error)),
        }
    }
    Ok(())
}

fn directory(path: &Path) -> crate::Result<PathBuf> {
    no_links(path)?;
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    if !fs::metadata(&canonical)
        .map_err(|error| io_error(path, error))?
        .is_dir()
    {
        return Err(crate::error(
            "UNSAFE_PATH",
            path.display().to_string(),
            "Expected a directory.",
            REPAIR_PATH,
        ));
    }
    Ok(canonical)
}

fn inside(root: &Path, relative: &str) -> crate::Result<PathBuf> {
    if !valid_package_path(relative) {
        return Err(crate::error(
            "UNSAFE_PATH",
            relative,
            "Invalid portable package path.",
            REPAIR_PATH,
        ));
    }
    let path = root.join(relative);
    no_links(&path)?;
    if let Ok(canonical) = fs::canonicalize(&path) {
        if !canonical.starts_with(root) {
            return Err(crate::error(
                "UNSAFE_PATH",
                relative,
                "Path leaves the selected root.",
                REPAIR_PATH,
            ));
        }
    }
    Ok(path)
}

fn read_file(root: &Path, relative: &str) -> crate::Result<Vec<u8>> {
    let path = inside(root, relative)?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(crate::error(
            "UNSAFE_PATH",
            path.display().to_string(),
            "Expected a regular file.",
            REPAIR_PATH,
        ));
    }
    fs::read(&path).map_err(|error| io_error(&path, error))
}

fn checked_template(sdk: &Path) -> crate::Result<BTreeMap<String, Vec<u8>>> {
    let lock_path = sdk.join("templates/template-lock.json");
    let lock_bytes = read_file(sdk, "templates/template-lock.json")?;
    let lock: TemplateLock = serde_json::from_slice(&lock_bytes)
        .map_err(|error| invalid(&lock_path, error.to_string()))?;
    if lock.schema_version != 2
        || lock.sdk_release != serde_json::from_str::<serde_json::Value>(include_str!("../../../protocol/vendor-adapter-protocol.json")).expect("public protocol")["contractRelease"].as_str().expect("contract release")
        || lock.author_dependency_lock != "Cargo.lock"
        || lock.mirror_root != "vendor-sdk"
        || lock.dependency_nodes == 0
        || lock.network_policy.is_empty()
        || lock.editable_author_files.len() != EDITABLE.len()
        || lock
            .editable_author_files
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != EDITABLE.iter().copied().collect()
    {
        return Err(invalid(
            &lock_path,
            "Template lock does not describe this public Rust author template.",
        ));
    }
    if read_file(sdk, "templates/rust/template-lock.json")? != lock_bytes {
        return Err(invalid(
            &lock_path,
            "Public and template-local mirror locks differ.",
        ));
    }
    let template = directory(&sdk.join("templates/rust"))?;
    let mut files = BTreeMap::new();
    let mut names = BTreeSet::new();
    for relative in EDITABLE {
        files.insert((*relative).to_owned(), read_file(&template, relative)?);
        names.insert(relative.to_ascii_lowercase());
    }
    names.insert("template-lock.json".into());
    let mut has_lock = false;
    let mut has_wit = false;
    let mut has_rust = false;
    for generated in &lock.generated_files {
        let relative = &generated.target;
        if !valid_package_path(relative)
            || !names.insert(relative.to_ascii_lowercase())
            || !(relative.starts_with("vendor-sdk/") || relative == "Cargo.lock")
            || generated.sources.is_empty()
            || generated.transform.is_empty()
        {
            return Err(invalid(
                &lock_path,
                format!("Unsafe or duplicate generated target: {relative}"),
            ));
        }
        let content = read_file(&template, relative)?;
        if hash_bytes(&content) != generated.sha256 {
            return Err(invalid(
                &template.join(relative),
                "Generated template file differs from its sealed hash.",
            ));
        }
        let mut sources = BTreeSet::new();
        for source in &generated.sources {
            if !valid_package_path(&source.path)
                || !sources.insert(source.path.to_ascii_lowercase())
            {
                return Err(invalid(
                    &lock_path,
                    "Generated source path is unsafe or duplicated.",
                ));
            }
            let original = read_file(sdk, &source.path)?;
            if hash_bytes(&original) != source.sha256 {
                return Err(invalid(
                    &sdk.join(&source.path),
                    "Public SDK source differs from the template source hash.",
                ));
            }
            if generated.transform == "byte-copy"
                && (generated.sources.len() != 1 || original != content)
            {
                return Err(invalid(
                    &template.join(relative),
                    "Byte-copy mirror differs from the public SDK source.",
                ));
            }
        }
        has_lock |= relative == "Cargo.lock";
        has_wit |= relative == "vendor-sdk/wit/vendor-adapter.wit";
        has_rust |= relative == "vendor-sdk/sdk/rust/src/lib.rs";
        files.insert(relative.to_owned(), content);
    }
    if !has_lock || !has_wit || !has_rust {
        return Err(invalid(
            &lock_path,
            "The author dependency lock, public Rust SDK or public WIT mirror is missing.",
        ));
    }
    files.insert("template-lock.json".into(), lock_bytes);
    Ok(files)
}

/// Create a new project only after every public source and generated hash passed.
pub fn init(sdk: &Path, destination: &Path) -> crate::Result<()> {
    init_template(sdk, destination, false)
}
pub fn init_speech(sdk: &Path, destination: &Path) -> crate::Result<()> {
    init_template(sdk, destination, true)
}
fn init_template(sdk: &Path, destination: &Path, speech: bool) -> crate::Result<()> {
    let sdk = directory(sdk)?;
    let mut files = checked_template(&sdk)?;
    if speech {
        for name in ["src/lib.rs", "src/capabilities.json", "manifest.json", "README.md"] {
            files.insert(name.into(), read_file(&sdk, &format!("templates/tts-adapter/{name}"))?);
        }
    }
    no_links(destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| io_error(destination, "Destination has no parent directory."))?;
    let parent = directory(parent)?;
    let name = destination
        .file_name()
        .ok_or_else(|| io_error(destination, "Destination has no directory name."))?;
    let destination = parent.join(name);
    // create_dir is deliberately exclusive: an existing empty directory is also refused.
    fs::create_dir(&destination).map_err(|error| {
        crate::error(
            "DESTINATION_EXISTS_OR_UNWRITABLE",
            destination.display().to_string(),
            error.to_string(),
            "Choose a new, nonexistent project directory under an existing parent.",
        )
    })?;
    let copied: crate::Result<()> = (|| {
        for (relative, bytes) in files {
            let path = inside(&destination, &relative)?;
            let parent = path
                .parent()
                .ok_or_else(|| io_error(&path, "File has no parent."))?;
            fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
            no_links(&path)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| io_error(&path, error))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| io_error(&path, error))?;
        }
        Ok(())
    })();
    if let Err(error) = copied {
        // Only this call's exclusively created destination can be cleaned up.
        let canonical = directory(&destination)?;
        if canonical != destination || canonical.parent() != Some(parent.as_path()) {
            return Err(io_error(
                &destination,
                "Incomplete project cleanup refused: destination identity changed.",
            ));
        }
        fs::remove_dir_all(&canonical).map_err(|cleanup| {
            io_error(
                &canonical,
                format!(
                    "Initialization failed ({}); incomplete project cleanup also failed: {cleanup}",
                    error.message
                ),
            )
        })?;
        return Err(error);
    }
    Ok(())
}

/// Author source and dependency lock are editable; public SDK/WIT mirrors are sealed.
pub fn validate_mirrors(sdk: &Path, project: &Path) -> crate::Result<()> {
    let sdk = directory(sdk)?;
    let project = directory(project)?;
    let files = checked_template(&sdk)?;
    for (relative, expected) in files {
        if EDITABLE.contains(&relative.as_str()) {
            continue;
        }
        let actual = read_file(&project, &relative)?;
        if relative == "Cargo.lock" {
            continue;
        }
        if actual != expected {
            return Err(invalid(&project.join(relative), "Author project generated mirror or template lock differs from the selected public SDK."));
        }
    }
    Ok(())
}

fn create_directory(root: &Path, relative: &str) -> crate::Result<PathBuf> {
    let path = inside(root, relative)?;
    fs::create_dir_all(&path).map_err(|error| io_error(&path, error))?;
    let canonical = directory(&path)?;
    if !canonical.starts_with(root) {
        return Err(io_error(&path, "Created directory leaves project root."));
    }
    Ok(canonical)
}

fn reject_implicit_cargo_files(run_directory: &Path, home: &Path) -> crate::Result<()> {
    let mut candidates = Vec::new();
    for ancestor in run_directory.ancestors() {
        candidates.extend([
            ancestor.join(".cargo/config"),
            ancestor.join(".cargo/config.toml"),
        ]);
    }
    candidates.extend(
        ["config", "config.toml", "credentials", "credentials.toml"].map(|name| home.join(name)),
    );
    for path in candidates {
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(crate::error("CARGO_CONFIGURATION_PRESENT", path.display().to_string(), "Cargo would read an implicit configuration or credential file; the file was not opened.", "Use an author project without ancestor .cargo/config files and without config or credential files in its explicit .bgx-sdk/cargo-home.")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(io_error(&path, error)),
        }
    }
    Ok(())
}

/// Native toolchains need ordinary absolute drive/UNC paths. Safety checks
/// continue to use canonical paths; MSVC LINK misparses long rustc object names
/// prefixed with \\?\ even when the identical ordinary absolute name works.
fn cargo_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();
        if let Some(Component::Prefix(prefix)) = components.next() {
            let ordinary = match prefix.kind() {
                Prefix::VerbatimDisk(drive) => Some(PathBuf::from(format!("{}:", drive as char))),
                Prefix::VerbatimUNC(server, share) => {
                    let mut value = std::ffi::OsString::from(r"\\");
                    value.push(server);
                    value.push(r"\");
                    value.push(share);
                    Some(PathBuf::from(value))
                }
                _ => None,
            };
            if let Some(mut ordinary) = ordinary {
                for part in components {
                    ordinary.push(part.as_os_str());
                }
                return ordinary;
            }
        }
    }
    path.to_owned()
}

fn cargo_command(run_directory: &Path, home: &Path, target: &Path) -> Command {
    let mut command = Command::new("cargo");
    command.env_clear();
    // Dependency cache/configuration and build outputs are explicit project state.
    // All Cargo configuration search locations are rejected before spawning.
    // Host build scripts/proc macros still require the native toolchain during a
    // WASM build. Windows/MSVC discovery needs SystemDrive and installation
    // locations (vswhere uses ProgramFiles). Preserve OS/profile locations,
    // never compiler/Cargo options, wrappers, proxy settings or credentials.
    for name in [
        "PATH",
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "TEMP",
        "TMP",
        "PATHEXT",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "LOCALAPPDATA",
        "APPDATA",
        "USERPROFILE",
        "HOME",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if let Some(home) = std::env::var_os("RUSTUP_HOME") {
        command.env("RUSTUP_HOME", home);
    } else if let Some(profile) =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
    {
        command.env("RUSTUP_HOME", PathBuf::from(profile).join(".rustup"));
    }
    command
        .current_dir(cargo_path(run_directory))
        .env("CARGO_HOME", cargo_path(home))
        .env("CARGO_TARGET_DIR", cargo_path(target))
        .env("RUSTUP_TOOLCHAIN", "1.95.0")
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

/// rustc can report the same source as relative or absolute between rebuilds.
/// Normalize both forms before code generation so caller locations cannot leak
/// build roots or change the artifact. These flags are generated here; ambient
/// compiler flags remain excluded by cargo_command's cleared environment.
fn source_remapping(sdk: &Path, project: &Path) -> crate::Result<String> {
    let mut flags = vec!["--remap-path-prefix==/bgx-author".to_owned()];
    // rustc uses the last matching mapping. A project nested in the SDK must
    // retain its author identity, so project mappings follow SDK mappings.
    for (root, replacement) in [(sdk, "/bgx-sdk"), (project, "/bgx-author")] {
        let canonical = root
            .to_str()
            .ok_or_else(|| io_error(root, "Build source path is not UTF8."))?;
        if canonical.contains('\u{1f}') {
            return Err(io_error(
                root,
                "Build source path contains the compiler flag separator.",
            ));
        }
        let ordinary = if let Some(unc) = canonical.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            canonical
                .strip_prefix(r"\\?\")
                .unwrap_or(canonical)
                .to_owned()
        };
        for spelling in [
            ordinary.replace('\\', "/"),
            ordinary,
            canonical.replace('\\', "/"),
            canonical.to_owned(),
        ] {
            flags.push(format!("--remap-path-prefix={spelling}={replacement}"));
        }
    }
    Ok(flags.join("\u{1f}"))
}

fn run_cargo(mut command: Command, log: &Path, stdout: Option<&Path>) -> crate::Result<()> {
    no_links(log)?;
    let log_file = File::create(log).map_err(|error| io_error(log, error))?;
    let output = if let Some(path) = stdout {
        no_links(path)?;
        File::create(path).map_err(|error| io_error(path, error))?
    } else {
        log_file.try_clone().map_err(|error| io_error(log, error))?
    };
    command
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(log_file));
    let status = command.status().map_err(|error| {
        crate::error(
            "CARGO_FAILED",
            log.display().to_string(),
            format!("Cargo could not start: {error}"),
            REPAIR_CARGO,
        )
    })?;
    if !status.success() {
        return Err(crate::error(
            "CARGO_FAILED",
            log.display().to_string(),
            format!("Cargo exited with {status}; no new artifact was published."),
            REPAIR_CARGO,
        ));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> crate::Result<()> {
    no_links(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| io_error(path, "File has no parent directory."))?;
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| io_error(path, error))?;
    staged
        .write_all(bytes)
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|error| io_error(path, error))?;
    no_links(path)?;
    staged
        .persist(path)
        .map_err(|error| io_error(path, error.error))?;
    Ok(())
}

/// Compile only the selected author's cdylib, using the locked public closure.
pub fn build(sdk: &Path, project: &Path, offline: bool) -> crate::Result<()> {
    let sdk = directory(sdk)?;
    let project = directory(project)?;
    let mirror_lock = inside(&project, "template-lock.json")?;
    let has_mirror_lock = mirror_lock
        .try_exists()
        .map_err(|error| io_error(&mirror_lock, error))?;
    if has_mirror_lock {
        validate_mirrors(&sdk, &project)?;
    }
    let manifest_path = inside(&project, "manifest.json")?;
    let original_manifest_bytes = read_file(&project, "manifest.json")?;
    let manifest: Value = serde_json::from_slice(&original_manifest_bytes).map_err(|error| {
        crate::error(
            "INVALID_PACKAGE",
            manifest_path.display().to_string(),
            error.to_string(),
            "Correct manifest.json against the public manifest schema.",
        )
    })?;
    SchemaRegistry::validate_public("manifest", &manifest).map_err(|error| {
        crate::error(
            error.code,
            format!("manifest.json{}", error.path),
            error.reason,
            "Correct the indicated field against the public manifest schema.",
        )
    })?;
    if manifest["language"] != "rust" {
        return Err(crate::error(
            "UNSUPPORTED_LANGUAGE",
            "manifest.json/language",
            "The author build command requires a Rust project.",
            "Use a Rust source project or check an existing precompiled package.",
        ));
    }
    let (inventory_name, signature_name) = crate::package::names();
    let signature = inside(&project, signature_name)?;
    if signature
        .try_exists()
        .map_err(|error| io_error(&signature, error))?
    {
        return Err(crate::error("SIGNED_PACKAGE_REBUILD", signature.display().to_string(), "The existing package signature would be invalidated by rebuilding.", "Keep the signed package intact; rebuild an unsigned author project, then explicitly sign the new package."));
    }
    let entry = manifest["entry"]
        .as_str()
        .expect("manifest schema requires entry")
        .to_owned();
    let artifact = inside(&project, &entry)?;
    let entry_folded = entry.to_ascii_lowercase();
    if EDITABLE
        .iter()
        .any(|path| path.to_ascii_lowercase() == entry_folded)
        || [
            inventory_name,
            signature_name,
            "template-lock.json",
            "cargo.lock",
        ]
        .contains(&entry_folded.as_str())
        || entry_folded.starts_with("vendor-sdk/")
        || entry_folded.starts_with("src/")
        || entry_folded.starts_with(".bgx-sdk/")
    {
        return Err(crate::error(
            "INVALID_PACKAGE",
            "manifest.json/entry",
            "Artifact entry overlaps author source or SDK metadata.",
            "Choose a separate portable artifact path, for example adapter.wasm.",
        ));
    }
    read_file(&project, "Cargo.toml")?;
    read_file(&project, "Cargo.lock")?;
    let logs = create_directory(&project, ".bgx-sdk/logs")?;
    let home = create_directory(&project, ".bgx-sdk/cargo-home")?;
    let target = create_directory(&project, ".bgx-sdk/target")?;
    let run_directory = create_directory(&project, ".bgx-sdk/cargo-run")?;
    reject_implicit_cargo_files(&run_directory, &home)?;
    let metadata_json = inside(&logs, "metadata.json")?;
    let cargo_manifest = inside(&project, "Cargo.toml")?;
    let mut metadata = cargo_command(&run_directory, &home, &target);
    metadata
        .args([
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--filter-platform",
            "wasm32-wasip2",
            "--manifest-path",
        ])
        .arg(cargo_path(&cargo_manifest));
    if offline {
        metadata.arg("--offline");
    }
    run_cargo(metadata, &logs.join("metadata.log"), Some(&metadata_json))?;
    let metadata: Value = serde_json::from_slice(&read_file(&logs, "metadata.json")?)
        .map_err(|error| io_error(&metadata_json, error))?;
    let metadata_error = || {
        crate::error("INVALID_CARGO_METADATA", metadata_json.display().to_string(), "Cargo metadata must identify one project cdylib and an isolated path dependency graph.", "Use an independent Rust package with one cdylib target and only public SDK path dependencies.")
    };
    let workspace = metadata["workspace_root"]
        .as_str()
        .ok_or_else(metadata_error)?;
    if directory(Path::new(workspace))? != project {
        return Err(metadata_error());
    }
    let reported_target = metadata["target_directory"]
        .as_str()
        .ok_or_else(metadata_error)?;
    if directory(Path::new(reported_target))? != target {
        return Err(metadata_error());
    }
    let mut root = None;
    for package in metadata["packages"].as_array().ok_or_else(metadata_error)? {
        if package["source"].is_null() {
            let path = Path::new(
                package["manifest_path"]
                    .as_str()
                    .ok_or_else(metadata_error)?,
            );
            no_links(path)?;
            let path = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
            if !path.starts_with(&project) && !path.starts_with(&sdk) {
                return Err(crate::error("DEPENDENCY_OUTSIDE_PUBLIC_ROOTS", path.display().to_string(), "Cargo path dependency leaves the author project and selected public SDK.", "Remove repository-private dependencies; use project-local sources or the selected public SDK."));
            }
            for target in package["targets"].as_array().ok_or_else(metadata_error)? {
                let source = Path::new(target["src_path"].as_str().ok_or_else(metadata_error)?);
                no_links(source)?;
                let source = fs::canonicalize(source).map_err(|error| io_error(source, error))?;
                if !source.starts_with(&project) && !source.starts_with(&sdk) {
                    return Err(crate::error("DEPENDENCY_OUTSIDE_PUBLIC_ROOTS", source.display().to_string(), "Cargo target source leaves the author project and selected public SDK.", "Keep source files and build scripts inside the author project or selected public SDK."));
                }
            }
            if path == cargo_manifest {
                if root.replace(package).is_some() {
                    return Err(metadata_error());
                }
            }
        } else if package["source"].as_str()
            != Some("registry+https://github.com/rust-lang/crates.io-index")
        {
            return Err(crate::error("DEPENDENCY_OUTSIDE_PUBLIC_ROOTS", package["name"].as_str().unwrap_or("Cargo dependency"), "Dependency is not from the public crates.io registry or an allowed project/SDK path.", "Use locked public registry dependencies or project-local/public-SDK sources."));
        }
    }
    let root = root.ok_or_else(metadata_error)?;
    let cdylibs = root["targets"]
        .as_array()
        .ok_or_else(metadata_error)?
        .iter()
        .filter(|target| {
            target["crate_types"]
                .as_array()
                .is_some_and(|types| types.iter().any(|kind| kind == "cdylib"))
        })
        .collect::<Vec<_>>();
    if cdylibs.len() != 1 {
        return Err(metadata_error());
    }
    let name = cdylibs[0]["name"].as_str().ok_or_else(metadata_error)?;
    let wasm_name = format!("{name}.wasm");
    if name.contains('/') || !valid_package_path(&wasm_name) {
        return Err(metadata_error());
    }
    let package_id = root["id"].as_str().ok_or_else(metadata_error)?;
    reject_implicit_cargo_files(&run_directory, &home)?;
    let mut compile = cargo_command(&run_directory, &home, &target);
    compile.env("CARGO_ENCODED_RUSTFLAGS", source_remapping(&sdk, &project)?);
    compile
        .args([
            "build",
            "--locked",
            "--target",
            "wasm32-wasip2",
            "--release",
            "--lib",
            "--manifest-path",
        ])
        .arg(cargo_path(&cargo_manifest))
        .arg("--package")
        .arg(package_id);
    if offline {
        compile.arg("--offline");
    }
    run_cargo(compile, &logs.join("build.log"), None)?;
    let output_relative = format!("wasm32-wasip2/release/{wasm_name}");
    let bytes = read_file(&target, &output_relative)?;
    if !bytes.starts_with(b"\0asm") {
        return Err(crate::error(
            "INVALID_ARTIFACT",
            output_relative,
            "Cargo output is not a WebAssembly binary.",
            "Inspect the project SDK build log and the Rust cdylib target.",
        ));
    }
    // Recheck the manifest before publication, so concurrent edits are never lost.
    if read_file(&project, "manifest.json")? != original_manifest_bytes {
        return Err(crate::error(
            "AUTHOR_PROJECT_CHANGED",
            manifest_path.display().to_string(),
            "Manifest changed during compilation; no new artifact was published.",
            "Finish editing the manifest and build again.",
        ));
    }
    if has_mirror_lock {
        validate_mirrors(&sdk, &project)?;
    }
    let artifact_parent = artifact.parent().ok_or_else(metadata_error)?;
    fs::create_dir_all(artifact_parent).map_err(|error| io_error(artifact_parent, error))?;
    no_links(artifact_parent)?;
    atomic_write(&artifact, &bytes)?;
    crate::package::refresh_inventory(&project)
}
