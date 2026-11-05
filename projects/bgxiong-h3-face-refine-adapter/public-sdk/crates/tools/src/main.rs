//! Explicit standalone development commands, without application discovery.
mod author;
mod conform;
mod host_rows;
mod host_channel;
mod package;
mod run;
mod retained;
mod transcript;
mod workspace;
use bgx_adapter_v2_validator::package::{PublisherTrust, TrustContext};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub path: String,
    pub message: String,
    pub hint: &'static str,
    pub exit: u8,
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn error(
    code: &'static str,
    path: impl Into<String>,
    message: impl Into<String>,
    hint: &'static str,
) -> Error {
    Error {
        code,
        path: path.into(),
        message: message.into(),
        hint,
        exit: 1,
    }
}
fn usage(message: impl Into<String>) -> Error {
    Error {
        exit: 2,
        ..error(
            "INVALID_ARGUMENT",
            "/arguments",
            message,
            "Use --help for explicit command arguments",
        )
    }
}
const HELP:&str="SDK v2 development CLI; does not install adapters into the application.\nretained info | inspect | export (read-only confirmed media; see RETAINED-MEDIA.zh-CN.md)\ninit --sdk ABS --dir NEW_ABS [--template speech]\nbuild --sdk ABS --project ABS [--offline]\ncheck --sdk ABS --worker ABS --project ABS [trust flags]\npack --sdk ABS --project ABS --out NEW_ABS [trust flags] [--signing-key ABS (--unencrypted-key | --password-file ABS)]\nTrust flags: --publisher-id ID --key-id ID --plugin-prefix PREFIX --public-key ABS\ncaps|probe|upload|submit|query|cancel --sdk ABS --worker ABS --package ABS --workspace ABS --config ABS --input ABS --output ABS [--create-workspace] [--request-id ORIGINAL] [--host-checkpoints] [--host-channel-v1]\n--host-channel-v1: await a channel_accept for before-send, then persist and acknowledge each outbound request before HTTP. media-stream is not implemented in S0.\n--host-checkpoints: checkpoint rows are served by the embedding host over stdio JSON lines ({hostCheckpoint:{op:get|insert|swap,...}}); the last stdout line stays the Response.\nRuntime input/output: public Invocation/Response JSON. Output must be in workspace/receipts.\nrecord (six-operation flags) --transcript NEW_ABS --seed U64 --now-unix-ms U64\nreplay --sdk ABS --worker ABS --package ABS --workspace ABS --config ABS --transcript ABS --output ABS\nconform (replay paths, replacing --transcript) --manifest ABS --evidence-root ABS\nReplay is offline, executes the original guest, and is labelled MOCK. Case command/status evidence declarations remain distinguished from actual execution.\nExit codes: 0 success, 1 failure, 2 argument error.";
struct Args {
    values: BTreeMap<String, String>,
    flags: BTreeSet<String>,
}
impl Args {
    fn parse(args: &[String], allowed: &[&str], flags: &[&str]) -> Result<Self> {
        let mut p = Self {
            values: BTreeMap::new(),
            flags: BTreeSet::new(),
        };
        let mut i = 0;
        while i < args.len() {
            let name = &args[i];
            if flags.contains(&name.as_str()) {
                if !p.flags.insert(name.clone()) {
                    return Err(usage(format!("Duplicate {name}")));
                }
                i += 1;
                continue;
            }
            if !allowed.contains(&name.as_str()) {
                return Err(usage(format!("Unknown argument {name}")));
            }
            let value = args
                .get(i + 1)
                .filter(|v| !v.starts_with("--"))
                .ok_or_else(|| usage(format!("Missing value for {name}")))?;
            if p.values.insert(name.clone(), value.clone()).is_some() {
                return Err(usage(format!("Duplicate {name}")));
            }
            i += 2;
        }
        Ok(p)
    }
    fn value(&self, name: &str) -> Result<&str> {
        self.values
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| usage(format!("Required {name}")))
    }
    fn path(&self, name: &str) -> Result<PathBuf> {
        absolute(self.value(name)?, name)
    }
    fn optional_path(&self, name: &str) -> Result<Option<PathBuf>> {
        self.values.get(name).map(|s| absolute(s, name)).transpose()
    }
}
fn absolute(value: &str, field: &str) -> Result<PathBuf> {
    let p = PathBuf::from(value);
    if !p.is_absolute() {
        return Err(usage(format!("{field} must be absolute")));
    }
    Ok(p)
}
fn trust(args: &Args) -> Result<TrustContext> {
    let mut t = TrustContext::default();
    let catalog: Value =
        serde_json::from_str(include_str!("../../../protocol/host-feature-catalog.json"))
            .expect("public feature catalog");
    t.supported_features = catalog["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_owned())
        .collect();
    let names = [
        "--publisher-id",
        "--key-id",
        "--plugin-prefix",
        "--public-key",
    ];
    if names.iter().any(|n| args.values.contains_key(*n)) {
        let path = args.path("--public-key")?;
        use std::io::Read;
        let limit = bgx_adapter_v2_ports::resource_policy()["limits"]["maxStateBytes"]
            .as_u64()
            .unwrap();
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
            .map_err(|e| {
                error(
                    "INVALID_INPUT",
                    path.display().to_string(),
                    e.to_string(),
                    "Supply an explicit minisign public key file",
                )
            })?;
        if bytes.len() as u64 > limit {
            return Err(error(
                "BUDGET_EXCEEDED",
                "--public-key",
                "Public key file is too large",
                "Supply only the public key box",
            ));
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            error(
                "INVALID_INPUT",
                "--public-key",
                "Public key must be UTF-8",
                "Supply a valid public key box",
            )
        })?;
        let _validated_key = minisign::PublicKey::from_box(
            minisign::PublicKeyBox::from_string(&text).map_err(|_| {
                error(
                    "INVALID_INPUT",
                    "--public-key",
                    "Invalid minisign public key",
                    "Export a valid public key box",
                )
            })?,
        )
        .map_err(|_| {
            error(
                "INVALID_INPUT",
                "--public-key",
                "Invalid minisign public key",
                "Export a valid public key box",
            )
        })?;
        let public_key = text;
        t.publishers.push(PublisherTrust {
            publisher_id: args.value(names[0])?.into(),
            key_id: args.value(names[1])?.into(),
            plugin_prefixes: vec![args.value(names[2])?.into()],
            public_key,
        });
        t.is_fresh = true;
    }
    Ok(t)
}
async fn check(sdk: &Path, worker: &Path, project: &Path, trust: &TrustContext) -> Result<Value> {
    use bgx_adapter_v2_reference_host::{
        config::ReferenceConfig, media::DevelopmentWorkspace, policy::DevelopmentNetworkGrant,
        ReferenceHost,
    };
    use bgx_adapter_v2_transport::{Control, Options};
    use bgx_adapter_v2_validator::capabilities::Binding;
    use std::time::{Duration, Instant};
    let runtime = package::stage_runtime(project, trust)?;
    let loaded = package::load_checked(runtime.path(), trust)?;
    let features = loaded.verified().enabled_features().clone();
    let runtime_language = if loaded.verified().manifest()["language"] == "profile" {
        "profile"
    } else {
        "wasm"
    };
    let binding = Binding::new("sdk-package-check".into(), loaded.verified(), 1, json!({}))
        .map_err(|e| {
            error(
                "INVALID_INPUT",
                e.path,
                e.reason,
                "Correct the package binding",
            )
        })?;
    let work = tempfile::tempdir().map_err(|e| {
        error(
            "RESOURCE_UNAVAILABLE",
            "/workspace",
            e.to_string(),
            "Provide temporary disk space",
        )
    })?;
    let workspace = DevelopmentWorkspace::create(work.path()).map_err(|e| {
        error(
            "RESOURCE_UNAVAILABLE",
            "/workspace",
            e.message,
            "Use a new empty development directory",
        )
    })?;
    let d = Duration::from_secs(5);
    let config = ReferenceConfig {
        workspace,
        sdk_root: sdk.into(),
        worker: worker.into(),
        transport: Options {
            handshake_timeout: d,
            write_timeout: d,
            shutdown_timeout: d,
            cancel_grace: Duration::from_millis(100),
        },
        network: DevelopmentNetworkGrant {
            origins: vec![],
            allow_private: false,
            allow_embedded_authorization: false,
            denied_hosts: vec![],
            denied_networks: vec![],
        },
        credentials: vec![],
        media_ttl: Duration::from_secs(60),
        probe: None,
        configuration_ref: "sdk-package-check-config".into(),
    };
    let host = ReferenceHost::new(config, loaded, binding).map_err(|e| {
        error(
            "INVALID_INPUT",
            "/worker",
            e.to_string(),
            "Use the explicit worker inside this SDK release",
        )
    })?;
    let attempt=async{for(op,payload)in[("capabilities",json!({})),("probe",json!({"schemaVersion":2,"kind":"package_validation","configuration":{}}))]{let invocation=serde_json::from_value(json!({"protocolVersion":2,"operation":op,"profileId":"sdk-package-check","bindingId":"sdk-package-check","capabilitySlot":"generic","requestId":format!("sdk-package-check-{op}"),"budgetMs":5000,"negotiatedVersion":2,"enabledFeatures":features,"payloadJson":payload.to_string()})).map_err(|e|error("INTERNAL_ERROR","/invocation",e.to_string(),"Check the SDK release"))?;let receipt=host.invoke(invocation,None,None,Control::new(Some(Instant::now()+Duration::from_secs(15))),None).await.map_err(|e|error("INVALID_PACKAGE","/runtime",e.to_string(),"Correct the adapter; package checks have no network or credentials"))?;let response=serde_json::to_value(receipt.response).unwrap();if response.get("error").is_some(){return Err(error("INVALID_PACKAGE","/runtime",response["error"].to_string(),"Correct the adapter error response"));}let data:Value=serde_json::from_str(response["dataJson"].as_str().unwrap()).map_err(|_|error("INVALID_PACKAGE","/dataJson","Invalid JSON","Return schema-valid JSON"))?;if op=="probe"&&data["packageValid"]!=true{return Err(error("INVALID_PACKAGE","/packageValid","Package validation did not succeed","Correct package validation issues"));}}Ok(json!({"status":"VALIDATED","checks":["inventory","schema","signature-policy",format!("{runtime_language}-capabilities"),format!("{runtime_language}-package-validation")],"network":"disabled"}))}.await;
    let idle = host.wait_idle().await.map_err(|e| {
        error(
            "RESOURCE_UNAVAILABLE",
            "/worker",
            e.to_string(),
            "Inspect SDK cleanup diagnostics",
        )
    });
    match (attempt, idle) {
        (Err(e), _) | (_, Err(e)) => Err(e),
        (Ok(v), Ok(())) => Ok(v),
    }
}
fn reject_output_alias(output: &Path, inputs: &[PathBuf]) -> Result<()> {
    let resolved_output = output.canonicalize().ok();
    if inputs.iter().any(|input| {
        output == input
            || resolved_output
                .as_ref()
                .zip(input.canonicalize().ok().as_ref())
                .is_some_and(|(a, b)| a == b)
    }) {
        return Err(error(
            "INVALID_INPUT",
            "--output",
            "Output aliases an input or evidence artifact",
            "Choose a distinct receipt output path",
        ));
    }
    Ok(())
}
async fn record_command(args: &[String]) -> Result<Value> {
    let allowed = [
        "--sdk",
        "--worker",
        "--package",
        "--workspace",
        "--config",
        "--input",
        "--output",
        "--request-id",
        "--transcript",
        "--seed",
        "--now-unix-ms",
    ];
    let parsed = Args::parse(&args[1..], &allowed, &["--create-workspace"])?;
    let path = parsed.path("--transcript")?;
    let seed = parsed
        .value("--seed")?
        .parse::<u64>()
        .map_err(|_| usage("--seed must be an unsigned integer"))?;
    let now_unix_ms = parsed
        .value("--now-unix-ms")?
        .parse::<u64>()
        .map_err(|_| usage("--now-unix-ms must be an unsigned integer"))?;
    let input: Value =
        serde_json::from_slice(&workspace::read(&parsed.path("--input")?, "maxFrameBytes")?)
            .map_err(|_| {
                error(
                    "INVALID_INPUT",
                    "--input",
                    "Input must be a public Invocation JSON object",
                    "Correct request.json",
                )
            })?;
    bgx_adapter_v2_validator::SchemaRegistry::validate_public("invocation", &input).map_err(
        |e| {
            error(
                e.code,
                e.path,
                e.reason,
                "Correct request.json against the public Invocation schema",
            )
        },
    )?;
    let mut forwarded = vec![input["operation"].as_str().unwrap().to_owned()];
    let mut i = 1;
    while i < args.len() {
        if ["--transcript", "--seed", "--now-unix-ms"].contains(&args[i].as_str()) {
            i += 2;
            continue;
        }
        forwarded.push(args[i].clone());
        if !["--create-workspace", "--host-checkpoints", "--host-channel-v1"].contains(&args[i].as_str()) {
            forwarded.push(args[i + 1].clone());
            i += 1;
        }
        i += 1;
    }
    run::execute_recorded(
        &forwarded,
        &path,
        transcript::MockInputs { seed, now_unix_ms },
    )
    .await
}
async fn replay_command(args: &[String]) -> Result<Value> {
    let command = &args[0];
    let mut allowed = vec![
        "--sdk",
        "--worker",
        "--package",
        "--workspace",
        "--config",
        "--output",
    ];
    if command == "replay" {
        allowed.push("--transcript");
    } else {
        allowed.extend(["--manifest", "--evidence-root"]);
    }
    let p = Args::parse(&args[1..], &allowed, &[])?;
    let sdk = p.path("--sdk")?;
    let worker = p.path("--worker")?;
    let package_path = p.path("--package")?;
    let config_path = p.path("--config")?;
    let workspace_path = p.path("--workspace")?;
    let output = p.path("--output")?;
    let config = workspace::Config::load(&config_path)?;
    let workspace = workspace::open(&workspace_path, false)?;
    workspace::contained_output(&workspace, &output)?;
    let mut inputs = vec![config_path, package_path.join("manifest.json")];
    let source = p.path(if command == "replay" {
        "--transcript"
    } else {
        "--manifest"
    })?;
    inputs.push(source.clone());
    reject_output_alias(&output, &inputs)?;
    // Provisioning only reads explicit development files. No real broker or
    // network policy executor is constructed for replay/conformance.
    let provision = config.provision(workspace.clone(), sdk.clone(), worker.clone())?;
    let policy = provision.snapshot_policy().map_err(|e| {
        error(
            "INVALID_INPUT",
            "/credentials",
            e.to_string(),
            "Use explicit nonsecret fixtures and credential references",
        )
    })?;
    let package = package::load_checked(&package_path, &config.trust())?;
    let report = if command == "replay" {
        let file = transcript::TranscriptFile::load(&source, &policy)?;
        serde_json::to_value(
            transcript::replay(
                &sdk,
                &worker,
                &package,
                &file,
                provision.transport.clone(),
                &policy,
            )
            .await?,
        )
        .unwrap()
    } else {
        let manifest = conform::EvidenceManifest::load(&source)?;
        let evidence_root = p.path("--evidence-root")?;
        for case in &manifest.cases {
            for artifact in &case.artifacts {
                inputs.push(evidence_root.join(&artifact.path));
            }
        }
        reject_output_alias(&output, &inputs)?;
        serde_json::to_value(
            conform::conform(
                &sdk,
                &worker,
                &package,
                &manifest,
                &evidence_root,
                provision.transport.clone(),
                &policy,
            )
            .await?,
        )
        .unwrap()
    };
    transcript::check_nonsecret(&policy, &report)?;
    workspace::atomic_json(&workspace, &output, &report)?;
    Ok(report)
}
async fn execute(args: &[String]) -> Result<Value> {
    let Some(command) = args.first() else {
        return Err(usage("A command is required"));
    };
    if matches!(
        command.as_str(),
        "caps" | "capabilities" | "probe" | "upload" | "submit" | "query" | "cancel"
    ) {
        return run::execute(args).await;
    }
    if command == "retained" {
        return retained::execute(&args[1..]);
    }
    if command == "record" {
        return record_command(args).await;
    }
    if matches!(command.as_str(), "replay" | "conform") {
        return replay_command(args).await;
    }
    let common = [
        "--sdk",
        "--project",
        "--publisher-id",
        "--key-id",
        "--plugin-prefix",
        "--public-key",
    ];
    match command.as_str() {
        "init" => {
            let p = Args::parse(&args[1..], &["--sdk", "--dir", "--template"], &[])?;
            match p.values.get("--template").map(String::as_str) {
                None => author::init(&p.path("--sdk")?, &p.path("--dir")?)?,
                Some("speech") => author::init_speech(&p.path("--sdk")?, &p.path("--dir")?)?,
                Some(_) => return Err(usage("Unknown template; supported: speech")),
            }
            Ok(json!({"status":"CREATED","directory":p.value("--dir")?}))
        }
        "build" => {
            let p = Args::parse(&args[1..], &["--sdk", "--project"], &["--offline"])?;
            author::build(
                &p.path("--sdk")?,
                &p.path("--project")?,
                p.flags.contains("--offline"),
            )?;
            Ok(json!({"status":"BUILT","project":p.value("--project")?}))
        }
        "check" => {
            let mut allowed = common.to_vec();
            allowed.push("--worker");
            let p = Args::parse(&args[1..], &allowed, &[])?;
            check(
                &p.path("--sdk")?,
                &p.path("--worker")?,
                &p.path("--project")?,
                &trust(&p)?,
            )
            .await
        }
        "pack" => {
            let mut allowed = common.to_vec();
            allowed.extend(["--out", "--signing-key", "--password-file"]);
            let p = Args::parse(&args[1..], &allowed, &["--unencrypted-key"])?;
            let key = p.optional_path("--signing-key")?;
            let password = p.optional_path("--password-file")?;
            let unencrypted = p.flags.contains("--unencrypted-key");
            if key.is_none() && (password.is_some() || unencrypted) {
                return Err(usage("Signing mode requires --signing-key"));
            }
            if key.is_some() && (password.is_some() == unencrypted) {
                return Err(usage(
                    "Signing needs exactly one of --password-file or --unencrypted-key",
                ));
            }
            let signing = key.as_deref().map(|secret_key| package::SigningOptions {
                secret_key,
                password_file: password.as_deref(),
                unencrypted_key: unencrypted,
            });
            package::pack(
                &p.path("--sdk")?,
                &p.path("--project")?,
                &p.path("--out")?,
                signing.as_ref(),
                &trust(&p)?,
            )?;
            Ok(json!({"status":"PACKED","directory":p.value("--out")?}))
        }
        _ => Err(usage(format!("Unknown command {command}"))),
    }
}
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{HELP}");
        return;
    }
    match execute(&args).await {
        Ok(v) => {
            println!("{}", serde_json::to_string(&v).unwrap());
            if v.get("error").is_some()
                || v.get("passed") == Some(&Value::Bool(false))
                || v.get("conformant") == Some(&Value::Bool(false))
                || v.get("toolStatus").and_then(Value::as_str) == Some("partial")
            {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!(
                "{}",
                json!({"code":e.code,"path":e.path,"message":e.message,"hint":e.hint})
            );
            std::process::exit(i32::from(e.exit));
        }
    }
}
