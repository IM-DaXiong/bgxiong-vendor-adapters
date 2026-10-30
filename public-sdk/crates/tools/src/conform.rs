//! Public conformance evidence integrity plus actual offline transcript execution.
//! Command/status fields are declarations, not proof that this process ran them.
use crate::transcript::{self, TranscriptFile};
use bgx_adapter_v2_ports::{resource_policy, wire::guest::valid_package_path};
use bgx_adapter_v2_reference_host::{checkpoint::SnapshotPolicy, config::LoadedPackage};
use bgx_adapter_v2_transport::Options;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

fn invalid(message: impl Into<String>) -> crate::Error {
    crate::error("INVALID_INPUT","conformance",message,"Provide explicit case declarations and measured evidence for the fixed public conformance catalog; missing cases remain NOT_RUN.")
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
    #[serde(rename = "NOT_RUN")]
    NotRun,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[serde(rename = "MOCK")]
    Mock,
    #[serde(rename = "LIVE")]
    Live,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactKind {
    #[serde(rename = "EVIDENCE")]
    Evidence,
    #[serde(rename = "TRANSCRIPT")]
    Transcript,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    pub path: String,
    pub sha256: String,
    pub kind: ArtifactKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseEvidence {
    pub id: String,
    pub status: Status,
    pub mode: Mode,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub artifacts: Vec<Artifact>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceManifest {
    pub schema_version: u64,
    pub cases: Vec<CaseEvidence>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    schema_version: u64,
    contract_release: String,
    cases: Vec<CatalogCase>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogCase {
    id: String,
    title: String,
    required: bool,
}
fn catalog() -> crate::Result<Catalog> {
    let catalog: Catalog =
        serde_json::from_str(include_str!("../../../protocol/conformance-cases.json"))
            .map_err(|_| invalid("Embedded public conformance catalog is invalid"))?;
    let mut ids = BTreeSet::new();
    if catalog.schema_version != 2
        || catalog.contract_release != "2.0.0"
        || catalog.cases.is_empty()
        || catalog
            .cases
            .iter()
            .any(|case| case.id.is_empty() || !ids.insert(&case.id))
    {
        return Err(invalid("Embedded public conformance catalog is incomplete"));
    }
    Ok(catalog)
}
impl EvidenceManifest {
    pub fn load(path: &Path) -> crate::Result<Self> {
        crate::workspace::absolute(path, "manifest")?;
        crate::workspace::plain_path(path)?;
        serde_json::from_slice(&crate::workspace::read(path, "maxFrameBytes")?)
            .map_err(|_| invalid("Conformance manifest is not a valid closed evidence declaration"))
    }
}
fn artifact_path(root: &Path, name: &str) -> crate::Result<PathBuf> {
    if !valid_package_path(name) {
        return Err(invalid(
            "Evidence paths must be safe paths relative to the explicit evidence root",
        ));
    }
    let path = root.join(name);
    crate::workspace::plain_path(&path)?;
    let resolved = path
        .canonicalize()
        .map_err(|_| invalid("Evidence artifact is unavailable"))?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(invalid(
            "Evidence artifact escapes its explicit root or is not a regular file",
        ));
    }
    Ok(resolved)
}
fn measure(path: &Path) -> crate::Result<String> {
    let mut file =
        File::open(path).map_err(|_| invalid("Cannot open the explicit evidence artifact"))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![
        0u8;
        resource_policy()["limits"]["maxHttpChunkBytes"]
            .as_u64()
            .unwrap() as usize
    ];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| invalid("Cannot stream the explicit evidence artifact"))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Source {
    EvidenceDeclaration,
    ActuallyExecuted,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactReport {
    pub path: String,
    pub expected_sha256: String,
    pub measured_sha256: Option<String>,
    pub kind: ArtifactKind,
    pub matched: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseReport {
    pub id: String,
    pub title: String,
    pub required: bool,
    pub declared_status: Status,
    pub mode: Mode,
    pub source: Source,
    pub passed: bool,
    pub evidence_sha256: Option<String>,
    pub failures: Vec<String>,
    pub artifacts: Vec<ArtifactReport>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConformanceReport {
    pub scope: &'static str,
    pub integrity_complete: bool,
    pub conformant: bool,
    pub required_cases: usize,
    pub cases: Vec<CaseReport>,
    pub replays: Vec<Value>,
    pub live_declarations: Vec<String>,
    pub live_actually_verified: bool,
}
fn root(path: &Path) -> crate::Result<PathBuf> {
    crate::workspace::absolute(path, "evidenceRoot")?;
    crate::workspace::plain_path(path)?;
    let path = path
        .canonicalize()
        .map_err(|_| invalid("Explicit evidence root is unavailable"))?;
    if !path.is_dir() {
        return Err(invalid("Evidence root must be a directory"));
    }
    Ok(path)
}
pub fn evaluate(
    manifest: &EvidenceManifest,
    evidence_root: &Path,
) -> crate::Result<ConformanceReport> {
    if manifest.schema_version != 1 {
        return Err(invalid("Unsupported conformance evidence version"));
    }
    let catalog = catalog()?;
    let root = root(evidence_root)?;
    let known: BTreeSet<_> = catalog.cases.iter().map(|case| case.id.as_str()).collect();
    let mut declarations = BTreeMap::new();
    for case in &manifest.cases {
        if !known.contains(case.id.as_str())
            || declarations.insert(case.id.as_str(), case).is_some()
        {
            return Err(invalid(
                "Unknown or duplicate public conformance case identifier",
            ));
        }
    }
    let mut reports = Vec::new();
    let mut live = Vec::new();
    for case in catalog.cases {
        let mut report = CaseReport {
            id: case.id.clone(),
            title: case.title,
            required: case.required,
            declared_status: Status::NotRun,
            mode: Mode::Mock,
            source: Source::EvidenceDeclaration,
            passed: false,
            evidence_sha256: None,
            failures: Vec::new(),
            artifacts: Vec::new(),
        };
        if let Some(declaration) = declarations.get(case.id.as_str()) {
            report.declared_status = declaration.status.clone();
            report.mode = declaration.mode.clone();
            if declaration.mode == Mode::Live {
                live.push(case.id.clone());
            }
            if declaration.status != Status::Pass {
                report.failures.push(if declaration.reason.is_empty() {
                    "Case has not been declared PASS".into()
                } else {
                    declaration.reason.clone()
                });
            }
            if declaration.command.is_empty() || declaration.command.iter().any(String::is_empty) {
                report
                    .failures
                    .push("Executed command declaration is missing".into());
            }
            if declaration.exit_code != Some(0) {
                report
                    .failures
                    .push("Evidence declaration does not report command exit code 0".into());
            }
            if declaration.artifacts.is_empty() {
                report
                    .failures
                    .push("Measured evidence artifacts are missing".into());
            }
            let mut paths = BTreeSet::new();
            for artifact in &declaration.artifacts {
                if !paths.insert(&artifact.path) {
                    report.failures.push("Duplicate evidence artifact".into());
                }
                let result = artifact_path(&root, &artifact.path).and_then(|path| measure(&path));
                let measured = result.as_ref().ok().cloned();
                let matched = measured.as_ref() == Some(&artifact.sha256);
                if !matched {
                    report.failures.push(match result {
                        Ok(_) => format!("Evidence digest differs: {}", artifact.path),
                        Err(e) => format!("Evidence unavailable: {} ({})", artifact.path, e.code),
                    });
                }
                report.artifacts.push(ArtifactReport {
                    path: artifact.path.clone(),
                    expected_sha256: artifact.sha256.clone(),
                    measured_sha256: measured,
                    kind: artifact.kind.clone(),
                    matched,
                });
            }
            report.evidence_sha256 = Some(bgx_adapter_v2_validator::hash_bytes(
                &serde_json::to_vec(declaration)
                    .map_err(|_| invalid("Cannot hash case evidence declaration"))?,
            ));
            report.passed = report.failures.is_empty();
        } else {
            report
                .failures
                .push("Required public case is missing and remains NOT_RUN".into());
        }
        reports.push(report);
    }
    let required = reports.iter().filter(|r| r.required).count();
    let integrity = reports.iter().filter(|r| r.required).all(|r| r.passed);
    Ok(ConformanceReport {
        scope: "EVIDENCE_INTEGRITY_ONLY",
        integrity_complete: integrity,
        conformant: false,
        required_cases: required,
        cases: reports,
        replays: Vec::new(),
        live_declarations: live,
        live_actually_verified: false,
    })
}
pub async fn conform(
    sdk: &Path,
    worker: &Path,
    package: &LoadedPackage,
    manifest: &EvidenceManifest,
    evidence_root: &Path,
    options: Options,
    policy: &SnapshotPolicy,
) -> crate::Result<ConformanceReport> {
    transcript::check_nonsecret(
        policy,
        &serde_json::to_value(manifest)
            .map_err(|_| invalid("Cannot encode case evidence declarations"))?,
    )?;
    let root = root(evidence_root)?;
    let mut report = evaluate(manifest, &root)?;
    report.scope = "MOCK_CONFORMANCE_WITH_DECLARED_EVIDENCE";
    let mut replayed = BTreeMap::<String, std::result::Result<Value, String>>::new();
    for case in &mut report.cases {
        if !case.passed {
            continue;
        }
        let mut actual = false;
        for artifact in &case.artifacts {
            if artifact.kind != ArtifactKind::Transcript || !artifact.matched {
                continue;
            }
            if !replayed.contains_key(&artifact.path) {
                let run = async {
                    let path = artifact_path(&root, &artifact.path)?;
                    let tape =
                        TranscriptFile::load_matching(&path, policy, &artifact.expected_sha256)?;
                    let run =
                        transcript::replay(sdk, worker, package, &tape, options.clone(), policy)
                            .await?;
                    serde_json::to_value(run)
                        .map_err(|_| invalid("Cannot encode actual replay evidence"))
                }
                .await
                .map_err(|e| format!("{}: {}", e.code, e.message));
                replayed.insert(artifact.path.clone(), run);
            }
            match &replayed[&artifact.path] {
                Ok(_) => actual = true,
                Err(reason) => case
                    .failures
                    .push(format!("Actual transcript replay failed: {reason}")),
            }
        }
        if case.id == "C28" {
            if case.mode == Mode::Live {
                case.failures
                    .push("A MOCK transcript cannot verify a LIVE replay declaration".into());
            }
            if actual && case.failures.is_empty() {
                case.source = Source::ActuallyExecuted;
            } else {
                case.failures.push("C28 requires actual execution of a matching transcript with no broker or network".into());
            }
        }
        case.passed = case.failures.is_empty();
    }
    report.replays = replayed
        .into_iter()
        .filter_map(|(path, result)| {
            result.ok().map(
                |run| serde_json::json!({"artifact":path,"source":"ACTUALLY_EXECUTED","run":run}),
            )
        })
        .collect();
    report.conformant = report.integrity_complete
        && report.cases.iter().filter(|r| r.required).all(|r| r.passed)
        && report
            .cases
            .iter()
            .any(|r| r.id == "C28" && r.passed && matches!(r.source, Source::ActuallyExecuted));
    Ok(report)
}
