//! One external author lifecycle shares its dependency cache and compiled guest.
//! Network access occurs only in the explicitly labelled first online build.
use bgx_adapter_v2_ports::wire::guest::parse_hash_inventory;
use bgx_adapter_v2_validator::hash_bytes;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Author {
    root: PathBuf,
    sdk: PathBuf,
    worker: PathBuf,
    project: PathBuf,
    step: usize,
}

fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.stdin(Stdio::null());
}

impl Author {
    fn record(&mut self, label: &str, executable: &Path, args: &[OsString], output: &Output) {
        self.step += 1;
        let evidence = self
            .root
            .join("evidence")
            .join(format!("{:02}-{label}", self.step));
        fs::create_dir_all(&evidence).unwrap();
        fs::write(
            evidence.join("command.json"),
            serde_json::to_vec_pretty(&json!({
                "executable": executable,
                "arguments": args.iter().map(|s|s.to_string_lossy()).collect::<Vec<_>>(),
                "exitCode": output.status.code(),
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
            }))
            .unwrap(),
        )
        .unwrap();
        let logs = self.project.join(".bgx-sdk/logs");
        if logs.is_dir() {
            let saved_logs = evidence.join("project-logs");
            fs::create_dir(&saved_logs).unwrap();
            for entry in fs::read_dir(logs).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() {
                    fs::copy(entry.path(), saved_logs.join(entry.file_name())).unwrap();
                }
            }
        }
        eprintln!(
            "[author-e2e] {label}: {} (evidence {})",
            output.status,
            evidence.display()
        );
        if executable != Path::new("cargo") || !output.status.success() {
            eprintln!(
                "stdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn cli(
        &mut self,
        label: &str,
        operation: &str,
        paths: &[(&str, &Path)],
        flags: &[&str],
        expected_exit: i32,
    ) -> Value {
        let executable = Path::new(env!("CARGO_BIN_EXE_bgx-adapter-v2"));
        let mut args = vec![
            OsString::from(operation),
            OsString::from("--sdk"),
            self.sdk.as_os_str().to_owned(),
        ];
        for (name, path) in paths {
            args.push(OsString::from(*name));
            args.push(path.as_os_str().to_owned());
        }
        args.extend(flags.iter().map(|flag| OsString::from(*flag)));
        let mut command = Command::new(executable);
        hidden(&mut command);
        let output = command
            .args(&args)
            .current_dir(&self.root)
            .output()
            .unwrap();
        self.record(label, executable, &args, &output);
        assert_eq!(
            output.status.code(),
            Some(expected_exit),
            "{label}: unexpected exit\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = if expected_exit == 0 {
            &output.stdout
        } else {
            &output.stderr
        };
        serde_json::from_slice(bytes).unwrap_or_else(|error| {
            panic!(
                "{label}: CLI did not return structured JSON ({error})\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn build(&mut self, label: &str, offline: bool, exit: i32) -> Value {
        let project = self.project.clone();
        self.cli(
            label,
            "build",
            &[("--project", &project)],
            if offline { &["--offline"] } else { &[] },
            exit,
        )
    }

    fn check(&mut self, label: &str, project: &Path, exit: i32) -> Value {
        let worker = self.worker.clone();
        self.cli(
            label,
            "check",
            &[("--worker", &worker), ("--project", project)],
            &[],
            exit,
        )
    }

    fn author_updates_dependency_lock(&mut self) {
        // This is an explicit author action, not a CLI repair or a relaxed build.
        // Only metadata runs here; the boundary probe must never be compiled.
        let run_directory = self.root.join("explicit-lock-update-cwd");
        fs::create_dir(&run_directory).unwrap();
        let args = vec![
            OsString::from("metadata"),
            OsString::from("--offline"),
            OsString::from("--format-version"),
            OsString::from("1"),
            OsString::from("--filter-platform"),
            OsString::from("wasm32-wasip2"),
            OsString::from("--manifest-path"),
            self.project.join("Cargo.toml").into_os_string(),
        ];
        let mut command = Command::new("cargo");
        hidden(&mut command);
        command.env_clear();
        for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "PATHEXT"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let rustup = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(
                    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).expect(
                        "Rustup home must be explicit or discoverable for fixture metadata",
                    ),
                )
                .join(".rustup")
            });
        command
            .env("RUSTUP_HOME", rustup)
            .env("RUSTUP_TOOLCHAIN", "1.95.0")
            .env("CARGO_HOME", self.project.join(".bgx-sdk/cargo-home"))
            .env("CARGO_TARGET_DIR", self.project.join(".bgx-sdk/target"))
            .env("CARGO_TERM_COLOR", "never");
        let output = command
            .args(&args)
            .current_dir(run_directory)
            .output()
            .unwrap();
        self.record(
            "explicit-author-lock-update",
            Path::new("cargo"),
            &args,
            &output,
        );
        assert!(
            output.status.success(),
            "Explicit fixture lock update failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
        let probe = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == "sdk-boundary-probe")
            .expect("The locked dependency graph must actually contain the outside path package");
        assert!(probe["source"].is_null());
        assert_eq!(
            Path::new(probe["manifest_path"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            self.root
                .join("outside-crate/Cargo.toml")
                .canonicalize()
                .unwrap()
        );
    }
}

fn runtime_files(project: &Path) -> BTreeMap<String, Vec<u8>> {
    let manifest_bytes = fs::read(project.join("manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let protocol: Value = serde_json::from_str(include_str!(
        "../../../protocol/vendor-adapter-protocol.json"
    ))
    .unwrap();
    let inventory_name = protocol["signature"]["hashManifest"].as_str().unwrap();
    BTreeMap::from([
        ("manifest.json".into(), manifest_bytes),
        (
            manifest["entry"].as_str().unwrap().into(),
            fs::read(project.join(manifest["entry"].as_str().unwrap())).unwrap(),
        ),
        (
            inventory_name.into(),
            fs::read(project.join(inventory_name)).unwrap(),
        ),
    ])
}

fn assert_runtime_bytes(actual: BTreeMap<String, Vec<u8>>, expected: &BTreeMap<String, Vec<u8>>) {
    let summary = |files: &BTreeMap<String, Vec<u8>>| {
        files
            .iter()
            .map(|(name, bytes)| (name.clone(), (bytes.len(), hash_bytes(bytes))))
            .collect::<BTreeMap<_, _>>()
    };
    // Compare original bytes, not merely their hashes; keep failure logs bounded.
    assert!(
        actual == *expected,
        "Runtime bytes changed: actual={:?}; expected={:?}",
        summary(&actual),
        summary(expected)
    );
}

fn validated(receipt: &Value) {
    assert_eq!(receipt["status"], "VALIDATED");
    assert_eq!(receipt["network"], "disabled");
    let checks = receipt["checks"].as_array().unwrap();
    for check in [
        "inventory",
        "schema",
        "signature-policy",
        "wasm-capabilities",
        "wasm-package-validation",
    ] {
        assert!(
            checks.iter().any(|actual| actual == check),
            "Missing executed check {check}: {receipt}"
        );
    }
}

fn dump_logs(root: &Path) {
    let logs = root.join("project/.bgx-sdk/logs");
    if let Ok(entries) = fs::read_dir(logs) {
        for entry in entries {
            let Ok(entry) = entry else { continue };
            if entry.path().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "log")
            {
                eprintln!(
                    "\n--- {} ---\n{}",
                    entry.path().display(),
                    String::from_utf8_lossy(&fs::read(entry.path()).unwrap_or_default())
                );
            }
        }
    }
}

fn lifecycle(root: &Path) {
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let worker = sdk.join("target/debug").join(format!(
        "bgx-adapter-v2-worker{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        worker.is_file(),
        "Build the explicit SDK worker before this integration test: {}",
        worker.display()
    );
    assert!(
        !root.starts_with(&sdk),
        "Author test must use an external directory"
    );
    let project = root.join("project");
    let mut author = Author {
        root: root.into(),
        sdk,
        worker,
        project: project.clone(),
        step: 0,
    };
    let created = author.cli(
        "fresh-external-init",
        "init",
        &[("--dir", &project)],
        &[],
        0,
    );
    assert_eq!(created["status"], "CREATED");
    assert!(!project.join(".bgx-sdk").exists());
    let manifest_before = fs::read(project.join("manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_before).unwrap();
    let entry = manifest["entry"].as_str().unwrap().to_owned();
    assert!(!project.join(&entry).exists());

    // An author-selected lib name proves artifact discovery does not assume the template package name.
    let cargo_manifest = project.join("Cargo.toml");
    let cargo = fs::read_to_string(&cargo_manifest)
        .unwrap()
        .replace("[lib]\n", "[lib]\nname = \"external_author_renamed\"\n");
    fs::write(&cargo_manifest, &cargo).unwrap();
    let offline_failure = author.build("fresh-cache-offline-refused", true, 1);
    assert_eq!(offline_failure["code"], "CARGO_FAILED");
    assert!(!project.join(&entry).exists());
    assert_eq!(
        fs::read(project.join("manifest.json")).unwrap(),
        manifest_before
    );
    assert_eq!(
        author.build("explicit-first-online-build", false, 0)["status"],
        "BUILT"
    );
    let release = runtime_files(&project);
    let built_manifest: Value = serde_json::from_slice(&release["manifest.json"]).unwrap();
    let wasm = &release[&entry];
    assert!(wasm.starts_with(b"\0asm"));
    assert_eq!(built_manifest["artifactSha256"], hash_bytes(wasm));
    assert_eq!(
        *wasm,
        fs::read(
            project.join(".bgx-sdk/target/wasm32-wasip2/release/external_author_renamed.wasm")
        )
        .unwrap()
    );
    let protocol: Value = serde_json::from_str(include_str!(
        "../../../protocol/vendor-adapter-protocol.json"
    ))
    .unwrap();
    let inventory_name = protocol["signature"]["hashManifest"].as_str().unwrap();
    let inventory =
        parse_hash_inventory(std::str::from_utf8(&release[inventory_name]).unwrap()).unwrap();
    assert_eq!(inventory.get(&entry), Some(&hash_bytes(wasm)));
    assert_eq!(
        inventory.get("manifest.json"),
        Some(&hash_bytes(&release["manifest.json"]))
    );
    eprintln!("[author-e2e] actual WASM sha256={}", hash_bytes(wasm));
    assert_eq!(
        author.build("cached-locked-offline-build", true, 0)["status"],
        "BUILT"
    );
    assert_runtime_bytes(runtime_files(&project), &release);
    // Force rustc to rebuild unchanged source, rather than proving only a
    // cached Cargo no-op. Paths are compiler-generated data, never author data.
    let source_path = project.join("src/lib.rs");
    let source_bytes = fs::read(&source_path).unwrap();
    fs::write(&source_path, &source_bytes).unwrap();
    assert_eq!(
        author.build("forced-unchanged-source-rebuild", true, 0)["status"],
        "BUILT"
    );
    assert_runtime_bytes(runtime_files(&project), &release);
    assert_eq!(fs::read(&source_path).unwrap(), source_bytes);
    let private_root = project.to_str().unwrap().trim_start_matches(r"\\?\");
    assert!(
        !wasm
            .windows(private_root.len())
            .any(|part| part == private_root.as_bytes()),
        "WASM embeds the private author root"
    );
    for (label, config) in [
        ("ancestor-cargo-config-refused", root.join(".cargo/config")),
        (
            "ancestor-cargo-config-toml-refused",
            root.join(".cargo/config.toml"),
        ),
        (
            "explicit-home-config-refused",
            project.join(".bgx-sdk/cargo-home/config.toml"),
        ),
        (
            "explicit-home-credentials-refused",
            project.join(".bgx-sdk/cargo-home/credentials.toml"),
        ),
    ] {
        let sentinel = b"private-test-sentinel: deliberately invalid TOML must never be opened\n";
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, sentinel).unwrap();
        let refusal = author.build(label, true, 1);
        assert_eq!(refusal["code"], "CARGO_CONFIGURATION_PRESENT");
        assert_eq!(
            Path::new(refusal["path"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            config.canonicalize().unwrap()
        );
        assert!(
            !refusal.to_string().contains("private-test-sentinel"),
            "Configuration contents must not enter diagnostics"
        );
        assert_eq!(fs::read(&config).unwrap(), sentinel);
        assert_runtime_bytes(runtime_files(&project), &release);
        fs::remove_file(config).unwrap();
    }
    validated(&author.check("source-package-wasm-check", &project, 0));

    let packed = root.join("packed");
    assert_eq!(
        author.cli(
            "pack-new-output",
            "pack",
            &[("--project", &project), ("--out", &packed)],
            &[],
            0
        )["status"],
        "PACKED"
    );
    assert_runtime_bytes(runtime_files(&packed), &release);
    assert!(!packed.join("Cargo.toml").exists());
    assert!(!packed.join("vendor-sdk").exists());
    assert!(!packed.join(".bgx-sdk").exists());
    validated(&author.check("precompiled-package-wasm-check", &packed, 0));

    // Exercise the actual public-key CLI boundary, not only the signing library.
    let source_path = project.join("src/lib.rs");
    let original_source = fs::read_to_string(&source_path).unwrap();
    fs::write(
        &source_path,
        original_source.replace("local.sdk.echo", "tests.publisher"),
    )
    .unwrap();
    let mut signing_manifest = built_manifest.clone();
    signing_manifest["pluginId"] = json!("tests.publisher");
    signing_manifest["publisherId"] = json!("tests.publisher");
    signing_manifest["publisherKeyId"] = json!("author-cli-test-key");
    fs::write(
        project.join("manifest.json"),
        serde_json::to_vec_pretty(&signing_manifest).unwrap(),
    )
    .unwrap();
    assert_eq!(
        author.build("explicit-publisher-locked-build", true, 0)["status"],
        "BUILT"
    );
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let secret_path = root.join("test-only.key");
    let public_path = root.join("test-only.pub");
    fs::write(&secret_path, keys.sk.to_box(None).unwrap().to_string()).unwrap();
    fs::write(&public_path, keys.pk.to_box().unwrap().to_string()).unwrap();
    let signed = root.join("signed-package");
    let trust_flags = [
        "--publisher-id",
        "tests.publisher",
        "--key-id",
        "author-cli-test-key",
        "--plugin-prefix",
        "tests.publisher",
    ];
    let mut sign_flags = trust_flags.to_vec();
    sign_flags.push("--unencrypted-key");
    assert_eq!(
        author.cli(
            "cli-minisign-pack",
            "pack",
            &[
                ("--project", &project),
                ("--out", &signed),
                ("--signing-key", &secret_path),
                ("--public-key", &public_path)
            ],
            &sign_flags,
            0
        )["status"],
        "PACKED"
    );
    let worker = author.worker.clone();
    validated(&author.cli(
        "cli-signed-wasm-check",
        "check",
        &[
            ("--project", &signed),
            ("--worker", &worker),
            ("--public-key", &public_path),
        ],
        &trust_flags,
        0,
    ));
    let denied = author.check("cli-signed-absent-trust-refused", &signed, 1);
    assert_eq!(denied["code"], "UNAUTHORIZED");
    fs::write(&source_path, original_source).unwrap();
    for (name, bytes) in &release {
        fs::write(project.join(name), bytes).unwrap();
    }

    let sentinel = packed.join("sentinel.keep");
    fs::write(&sentinel, b"caller-owned sentinel\n").unwrap();
    assert_eq!(
        author.cli(
            "existing-pack-output-refused",
            "pack",
            &[("--project", &project), ("--out", &packed)],
            &[],
            1
        )["code"],
        "INVALID_PACKAGE"
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"caller-owned sentinel\n");
    assert_runtime_bytes(runtime_files(&packed), &release);
    fs::remove_file(&sentinel).unwrap();
    let mut tampered = release[&entry].clone();
    tampered[0] ^= 1;
    fs::write(packed.join(&entry), &tampered).unwrap();
    assert_eq!(
        author.check("tampered-wasm-inventory-refused", &packed, 1)["code"],
        "INVALID_PACKAGE"
    );
    assert_eq!(
        fs::read(packed.join(inventory_name)).unwrap(),
        release[inventory_name]
    );
    assert_eq!(fs::read(packed.join(&entry)).unwrap(), tampered);

    let existing = root.join("existing-author");
    fs::create_dir(&existing).unwrap();
    fs::write(
        existing.join("sentinel.keep"),
        b"existing author directory\n",
    )
    .unwrap();
    assert_eq!(
        author.cli(
            "existing-init-directory-refused",
            "init",
            &[("--dir", &existing)],
            &[],
            1
        )["code"],
        "DESTINATION_EXISTS_OR_UNWRITABLE"
    );
    assert_eq!(
        fs::read(existing.join("sentinel.keep")).unwrap(),
        b"existing author directory\n"
    );
    assert_eq!(fs::read_dir(&existing).unwrap().count(), 1);
    let sdk_without_templates = root.join("sdk-without-templates");
    fs::create_dir(&sdk_without_templates).unwrap();
    let original_sdk = std::mem::replace(&mut author.sdk, sdk_without_templates);
    let missing_destination = root.join("missing-template-author");
    assert_eq!(
        author.cli(
            "missing-public-template-refused",
            "init",
            &[("--dir", &missing_destination)],
            &[],
            1
        )["code"],
        "AUTHOR_IO"
    );
    author.sdk = original_sdk;
    assert!(!missing_destination.exists());

    let wit = project.join("vendor-sdk/wit/vendor-adapter.wit");
    let wit_bytes = fs::read(&wit).unwrap();
    fs::remove_file(&wit).unwrap();
    let missing_wit = author.build("missing-wit-mirror-refused", true, 1);
    assert_eq!(missing_wit["code"], "AUTHOR_IO");
    assert_eq!(
        Path::new(missing_wit["path"].as_str().unwrap()).file_name(),
        wit.file_name()
    );
    assert_runtime_bytes(runtime_files(&project), &release);
    fs::write(&wit, wit_bytes).unwrap();
    let sdk_mirror = project.join("vendor-sdk/sdk/rust/src/lib.rs");
    let sdk_bytes = fs::read(&sdk_mirror).unwrap();
    let mut modified = sdk_bytes.clone();
    modified.extend_from_slice(b"\n// rejected public SDK modification\n");
    fs::write(&sdk_mirror, modified).unwrap();
    assert_eq!(
        author.build("modified-sdk-mirror-refused", true, 1)["code"],
        "INVALID_TEMPLATE"
    );
    assert_runtime_bytes(runtime_files(&project), &release);
    fs::write(&sdk_mirror, sdk_bytes).unwrap();

    let outside = root.join("outside-crate");
    fs::create_dir_all(outside.join("src")).unwrap();
    fs::write(outside.join("Cargo.toml"), "[package]\nname = \"sdk-boundary-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n").unwrap();
    fs::write(
        outside.join("src/lib.rs"),
        "compile_error!(\"outside path dependency must be rejected before compilation\");\n",
    )
    .unwrap();
    let changed_cargo = cargo.replace("[dependencies]\n", "[dependencies]\nbgx_boundary_probe = { package = \"sdk-boundary-probe\", path = \"../outside-crate\" }\n");
    fs::write(&cargo_manifest, changed_cargo).unwrap();
    let original_lock = fs::read(project.join("Cargo.lock")).unwrap();
    author.author_updates_dependency_lock();
    assert_ne!(fs::read(project.join("Cargo.lock")).unwrap(), original_lock);
    let outside_failure = author.build("locked-outside-path-dependency-refused", true, 1);
    assert_eq!(outside_failure["code"], "DEPENDENCY_OUTSIDE_PUBLIC_ROOTS");
    assert_eq!(
        Path::new(outside_failure["path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        outside.join("Cargo.toml").canonicalize().unwrap()
    );
    assert_runtime_bytes(runtime_files(&project), &release);
}

#[test]
fn external_author_cli_lifecycle_and_failures_are_real() {
    let temporary = tempfile::Builder::new()
        .prefix("bgx-v2-author-e2e-")
        .tempdir()
        .unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let result = std::panic::catch_unwind(|| lifecycle(&root));
    if let Err(failure) = result {
        dump_logs(&root);
        let retained = temporary.keep();
        eprintln!(
            "[author-e2e] FAILED: retained complete fixture, CLI outputs and Cargo logs at {}",
            retained.display()
        );
        std::panic::resume_unwind(failure);
    }
}
