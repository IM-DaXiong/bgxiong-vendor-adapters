//! Explicit, closed development provisioning; never searches application data.
use bgx_adapter_v2_ports::resource_policy;
use bgx_adapter_v2_reference_host::{
    config::{CredentialProvision, ReferenceConfig},
    credential::DevelopmentSecret,
    media::{DevelopmentWorkspace, ProbeTool},
    policy::DevelopmentNetworkGrant,
};
use bgx_adapter_v2_transport::Options;
use bgx_adapter_v2_validator::package::{PublisherTrust, TrustContext};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub binding_id: String,
    pub binding_revision: u64,
    pub configuration_ref: String,
    pub configuration: Value,
    pub trust: Trust,
    pub network: Network,
    pub credentials: Vec<Credential>,
    pub media: Vec<Media>,
    pub media_ttl_ms: u64,
    pub transport: Transport,
    pub probe: Option<Probe>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Trust {
    publishers: Vec<Publisher>,
    revoked_plugins: BTreeSet<String>,
    revoked_versions: BTreeSet<(String, String)>,
    revoked_keys: BTreeSet<String>,
    denylisted_hosts: BTreeSet<String>,
    is_fresh: bool,
    supported_features: BTreeSet<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Publisher {
    publisher_id: String,
    key_id: String,
    public_key: String,
    plugin_prefixes: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Network {
    origins: Vec<String>,
    allow_private: bool,
    allow_embedded_authorization: bool,
    denied_hosts: Vec<String>,
    denied_networks: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Credential {
    reference: String,
    secret_file: PathBuf,
    public_id: Option<String>,
    custom_header: Option<String>,
    modes: BTreeSet<String>,
    origins: Vec<String>,
    ttl_ms: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Media {
    pub id: String,
    pub path: PathBuf,
    pub kind: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transport {
    handshake_timeout_ms: u64,
    write_timeout_ms: u64,
    shutdown_timeout_ms: u64,
    cancel_grace_ms: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Probe {
    executable: PathBuf,
    timeout_ms: u64,
    frame_timestamps: bool,
}

pub fn invalid(path: impl Into<String>, message: impl Into<String>) -> crate::Error {
    crate::error(
        "INVALID_INPUT",
        path,
        message,
        "Use explicit absolute paths and the documented SDK development configuration.",
    )
}
pub fn absolute(path: &Path, label: &str) -> crate::Result<()> {
    if !path.is_absolute() {
        return Err(invalid(label, "An explicit absolute path is required"));
    }
    Ok(())
}
/// Inspect the original lexical ancestors before canonicalization can hide links.
pub fn plain_path(path: &Path) -> crate::Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                let linked = meta.file_type().is_symlink();
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    linked || meta.file_attributes() & 0x400 != 0
                };
                if linked {
                    return Err(invalid(
                        "workspace",
                        "Development write paths cannot contain links or reparse points",
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(invalid(
                    "workspace",
                    "Cannot inspect the explicit development write path",
                ))
            }
        }
    }
    Ok(())
}
pub fn read(path: &Path, budget: &str) -> crate::Result<Zeroizing<Vec<u8>>> {
    absolute(path, "file")?;
    let limit = resource_policy()["limits"][budget]
        .as_u64()
        .expect("public budget");
    let file =
        File::open(path).map_err(|_| invalid("file", "The explicit input file cannot be read"))?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("file", "Reading the explicit input file failed"))?;
    if bytes.len() as u64 > limit {
        return Err(crate::error(
            "BUDGET_EXCEEDED",
            "file",
            "Input file exceeds the published SDK byte budget",
            "Reduce the input within the public SDK resource policy.",
        ));
    }
    Ok(bytes)
}
fn duration(value: u64) -> crate::Result<Duration> {
    if value == 0 {
        return Err(invalid(
            "config",
            "All development time budgets must be positive",
        ));
    }
    Ok(Duration::from_millis(value))
}
impl Config {
    pub fn load(path: &Path) -> crate::Result<Self> {
        let value: Self = serde_json::from_slice(&read(path, "maxStateBytes")?).map_err(|_| {
            invalid(
                "config",
                "Development configuration is not a valid closed configuration object",
            )
        })?;
        if value.binding_id.is_empty()
            || value.configuration_ref.is_empty()
            || !value.configuration.is_object()
        {
            return Err(invalid(
                "config",
                "Binding and nonsecret configuration must be explicitly declared",
            ));
        }
        let mut ids = BTreeSet::new();
        for media in &value.media {
            absolute(&media.path, "config.media.path")?;
            if media.id.is_empty() || !ids.insert(&media.id) {
                return Err(invalid(
                    "config.media",
                    "Media identifiers must be nonempty and unique",
                ));
            }
        }
        Ok(value)
    }
    pub fn probe_tool(&self) -> crate::Result<Option<ProbeTool>> {
        self
            .probe
            .as_ref()
            .map(|p| {
                absolute(&p.executable, "config.probe.executable")?;
                Ok(ProbeTool {
                    executable: p.executable.clone(),
                    timeout: duration(p.timeout_ms)?,
                    frame_timestamps: p.frame_timestamps,
                })
            })
            .transpose()
    }
    pub fn trust(&self) -> TrustContext {
        TrustContext {
            publishers: self
                .trust
                .publishers
                .iter()
                .map(|p| PublisherTrust {
                    publisher_id: p.publisher_id.clone(),
                    key_id: p.key_id.clone(),
                    public_key: p.public_key.clone(),
                    plugin_prefixes: p.plugin_prefixes.clone(),
                })
                .collect(),
            revoked_plugins: self.trust.revoked_plugins.clone(),
            revoked_versions: self.trust.revoked_versions.clone(),
            revoked_keys: self.trust.revoked_keys.clone(),
            denylisted_hosts: self.trust.denylisted_hosts.clone(),
            is_fresh: self.trust.is_fresh,
            supported_features: self.trust.supported_features.clone(),
        }
    }
    pub fn provision(
        &self,
        workspace: DevelopmentWorkspace,
        sdk_root: PathBuf,
        worker: PathBuf,
    ) -> crate::Result<ReferenceConfig> {
        let mut references = BTreeSet::new();
        let mut credentials = Vec::new();
        for c in &self.credentials {
            if c.reference.is_empty() || !references.insert(&c.reference) {
                return Err(invalid(
                    "config.credentials",
                    "Credential references must be nonempty and unique",
                ));
            }
            credentials.push(CredentialProvision {
                reference: c.reference.clone(),
                secret: DevelopmentSecret {
                    bytes: read(&c.secret_file, "maxStateBytes")?,
                    public_id: c.public_id.clone(),
                    custom_header: c.custom_header.clone(),
                },
                modes: c.modes.clone(),
                origins: c.origins.clone(),
                ttl: duration(c.ttl_ms)?,
            });
        }
        let denied_networks = self
            .network
            .denied_networks
            .iter()
            .map(|n| {
                n.parse().map_err(|_| {
                    invalid(
                        "config.network.deniedNetworks",
                        "Network deny entries must be valid IP networks",
                    )
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let probe = self.probe_tool()?;
        Ok(ReferenceConfig {
            workspace,
            sdk_root,
            worker,
            transport: Options {
                handshake_timeout: duration(self.transport.handshake_timeout_ms)?,
                write_timeout: duration(self.transport.write_timeout_ms)?,
                shutdown_timeout: duration(self.transport.shutdown_timeout_ms)?,
                cancel_grace: duration(self.transport.cancel_grace_ms)?,
            },
            network: DevelopmentNetworkGrant {
                origins: self.network.origins.clone(),
                allow_private: self.network.allow_private,
                allow_embedded_authorization: self.network.allow_embedded_authorization,
                denied_hosts: self.network.denied_hosts.clone(),
                denied_networks,
            },
            credentials,
            media_ttl: duration(self.media_ttl_ms)?,
            probe,
            configuration_ref: self.configuration_ref.clone(),
        })
    }
}
pub fn open(path: &Path, create: bool) -> crate::Result<DevelopmentWorkspace> {
    absolute(path, "workspace")?;
    plain_path(path)?;
    if !create {
        plain_path(&path.join(".bgx-sdk-workspace.json"))?;
    }
    let result = if create {
        DevelopmentWorkspace::create(path)
    } else {
        DevelopmentWorkspace::open(path)
    };
    let workspace = result.map_err(|_| {
        invalid(
            "workspace",
            "The selected directory is not an eligible SDK development workspace",
        )
    })?;
    if !create {
        let mut pending = vec![workspace.root().to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory)
                .map_err(|_| invalid("workspace", "Cannot inspect development workspace entries"))?
            {
                let entry = entry.map_err(|_| {
                    invalid("workspace", "Cannot inspect a development workspace entry")
                })?;
                plain_path(&entry.path())?;
                if entry
                    .file_type()
                    .map_err(|_| {
                        invalid(
                            "workspace",
                            "Cannot inspect development workspace entry type",
                        )
                    })?
                    .is_dir()
                {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(workspace)
}
/// Receipts and diagnostics remain in the explicitly marked development workspace.
fn contained_file(workspace: &DevelopmentWorkspace, path: &Path) -> crate::Result<()> {
    absolute(path, "output")?;
    plain_path(path)?;
    let parent = path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| invalid("output", "Output parent directory must already exist"))?;
    if !parent.starts_with(workspace.root()) || path.file_name().is_none() {
        return Err(invalid(
            "output",
            "Output must be inside the explicit SDK development workspace",
        ));
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || !path
                .canonicalize()
                .is_ok_and(|p| p.starts_with(workspace.root()))
        {
            return Err(invalid(
                "output",
                "Output must be a regular file within the development workspace",
            ));
        }
    }
    Ok(())
}
pub fn contained_output(workspace: &DevelopmentWorkspace, path: &Path) -> crate::Result<()> {
    let receipts = workspace.root().join("receipts");
    match fs::create_dir(&receipts) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => {
            return Err(invalid(
                "output",
                "Cannot create the development receipt directory",
            ))
        }
    }
    if receipts
        .canonicalize()
        .ok()
        .as_deref()
        .and_then(Path::parent)
        != Some(workspace.root())
    {
        return Err(invalid(
            "output",
            "Receipt directory escapes the development workspace",
        ));
    }
    contained_file(workspace, path)?;
    let parent = path
        .parent()
        .unwrap()
        .canonicalize()
        .map_err(|_| invalid("output", "Output directory is unavailable"))?;
    if !parent.starts_with(&receipts) || path.extension().and_then(|v| v.to_str()) != Some("json") {
        return Err(invalid(
            "output",
            "Output must be a JSON file within the explicit workspace receipts directory",
        ));
    }
    Ok(())
}
pub fn atomic_json(
    workspace: &DevelopmentWorkspace,
    path: &Path,
    value: &Value,
) -> crate::Result<()> {
    contained_file(workspace, path)?;
    let mut staged = tempfile::NamedTempFile::new_in(path.parent().unwrap())
        .map_err(|_| invalid("output", "Cannot stage the development receipt"))?;
    serde_json::to_writer(staged.as_file_mut(), value)
        .map_err(|_| invalid("output", "Cannot encode the development receipt"))?;
    staged
        .write_all(b"\n")
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|_| invalid("output", "Cannot synchronize the development receipt"))?;
    staged
        .persist(path)
        .map_err(|_| invalid("output", "Cannot atomically commit the development receipt"))?;
    Ok(())
}
