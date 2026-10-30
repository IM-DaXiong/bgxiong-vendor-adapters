//! Development media store. Run blocking file/probe work on the embedding host's
//! blocking executor. Every streaming loop observes its request Control; chunks
//! and probe output use the published resource budgets. Paths never enter wire DTOs.
use bgx_adapter_v2_ports::{
    decimal_u64, resource_policy,
    wire::{
        host::HostError, CheckpointV2MediaIdentitiesItem as MediaIdentity,
        HostPortCallV2MediaStat as MediaStat,
    },
};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::{
    identity::PackageIdentity,
    media::{valid_basename, MediaFacts, MediaLease, MediaValidationContext},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use tempfile::TempPath;
use uuid::Uuid;

type Result<T> = std::result::Result<T, HostError>;
fn error(code: &str, message: &str) -> HostError {
    serde_json::from_value(json!({"code":code,"message":message})).expect("public host error")
}
fn io_error(_: io::Error) -> HostError {
    error(
        "RESOURCE_UNAVAILABLE",
        "Development media storage is unavailable",
    )
}
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn budget(name: &str) -> usize {
    resource_policy()["limits"][name]
        .as_u64()
        .expect("published budget")
        .try_into()
        .expect("native budget")
}
fn check(control: &Control) -> Result<()> {
    if control.is_cancelled() {
        return Err(error("CANCELLED", "Local media operation was cancelled"));
    }
    if control.deadline.is_some_and(|d| Instant::now() >= d) {
        return Err(error("TIMEOUT", "Media request deadline exhausted"));
    }
    Ok(())
}

#[derive(Clone)]
pub struct DevelopmentWorkspace {
    root: PathBuf,
}
impl DevelopmentWorkspace {
    /// An existing empty directory is accepted; populated directories are never
    /// adopted or cleaned. This prevents pointing the broker at production data.
    pub fn create(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err(error(
                "INVALID_INPUT",
                "Development workspace must be an explicit absolute path",
            ));
        }
        if !root.exists() {
            fs::create_dir(root).map_err(io_error)?;
        }
        if fs::read_dir(root).map_err(io_error)?.next().is_some() {
            return Err(error(
                "PERMISSION_DENIED",
                "New development workspace must be empty",
            ));
        }
        let root = root.canonicalize().map_err(io_error)?;
        let marker = json!({"schemaVersion":2,"kind":"sdk-development-workspace","id":Uuid::new_v4().to_string()});
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(root.join(".bgx-sdk-workspace.json"))
            .map_err(io_error)?;
        file.write_all(&serde_json::to_vec(&marker).unwrap())
            .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        for directory in ["objects", "grants", "staging"] {
            fs::create_dir(root.join(directory)).map_err(io_error)?;
        }
        Ok(Self { root })
    }
    pub fn open(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err(error(
                "INVALID_INPUT",
                "Development workspace path must be absolute",
            ));
        }
        let root = root.canonicalize().map_err(io_error)?;
        let marker = bounded_json(
            &root.join(".bgx-sdk-workspace.json"),
            budget("maxStateBytes"),
        )?;
        if marker.as_object().is_none_or(|v| v.len() != 3)
            || marker["schemaVersion"] != 2
            || marker["kind"] != "sdk-development-workspace"
            || marker["id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .is_none()
        {
            return Err(error(
                "PERMISSION_DENIED",
                "Directory is not a valid SDK development workspace",
            ));
        }
        for directory in ["objects", "grants", "staging"] {
            let path = root.join(directory);
            if !path.is_dir()
                || path.canonicalize().map_err(io_error)?.parent() != Some(root.as_path())
            {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Development store directories escape their workspace",
                ));
            }
        }
        Ok(Self { root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn contained(&self, path: &Path) -> Result<PathBuf> {
        let path = path.canonicalize().map_err(io_error)?;
        if !path.starts_with(&self.root) || !path.is_file() {
            return Err(error(
                "PERMISSION_DENIED",
                "Media source is outside the explicit development workspace",
            ));
        }
        Ok(path)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseScope {
    pub package: PackageIdentity,
    pub binding_id: String,
    pub request_id: String,
}
impl LeaseScope {
    fn valid(&self) -> bool {
        !self.package.plugin_id.is_empty()
            && !self.binding_id.is_empty()
            && !self.request_id.is_empty()
    }
}
#[derive(Clone)]
pub struct ProbeTool {
    pub executable: PathBuf,
    pub timeout: Duration,
    pub frame_timestamps: bool,
}
#[derive(Debug, Clone)]
pub struct LeasedMedia {
    pub handle: String,
    pub identity: MediaIdentity,
    pub stat: MediaStat,
    pub file_name: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Grant {
    package: PackageIdentity,
    binding_id: String,
    identity: MediaIdentity,
    stat: MediaStat,
    file_name: String,
}
struct Ready {
    grant: Grant,
    file: File,
    modified: SystemTime,
}
struct Staged {
    file: Option<File>,
    path: Option<TempPath>,
    hasher: Sha256,
    length: u64,
    kind: String,
    mime_hint: String,
    media_id: String,
    auto_kind: bool,
}
impl Staged {
    fn cleanup(&mut self) -> Result<()> {
        drop(self.file.take());
        if let Some(path) = self.path.take() {
            path.close().map_err(io_error)?;
        }
        Ok(())
    }
}
impl Drop for Staged {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("SDK media staging cleanup failed: {}", error.r#message);
        }
    }
}
enum State {
    Ready(Ready),
    Writing(Staged),
}
struct Lease {
    scope: LeaseScope,
    expires: Instant,
    state: State,
}
pub struct MediaBroker {
    workspace: DevelopmentWorkspace,
    session: PathBuf,
    leases: HashMap<String, Lease>,
    ttl: Duration,
    probe: Option<ProbeTool>,
}
impl MediaBroker {
    pub fn new(
        workspace: DevelopmentWorkspace,
        ttl: Duration,
        probe: Option<ProbeTool>,
    ) -> Result<Self> {
        if ttl.is_zero() || Instant::now().checked_add(ttl).is_none() {
            return Err(error(
                "INVALID_INPUT",
                "Media lease duration must be positive and representable",
            ));
        }
        if let Some(tool) = &probe {
            if !tool.executable.is_absolute()
                || !tool.executable.is_file()
                || tool.timeout.is_zero()
                || Instant::now().checked_add(tool.timeout).is_none()
            {
                return Err(error(
                    "INVALID_INPUT",
                    "Media probe requires an explicit existing tool path and positive deadline",
                ));
            }
        }
        let session = workspace
            .root
            .join("staging")
            .join(Uuid::new_v4().to_string());
        fs::create_dir(&session).map_err(io_error)?;
        Ok(Self {
            workspace,
            session,
            leases: HashMap::new(),
            ttl,
            probe,
        })
    }
    fn begin(
        &mut self,
        scope: &LeaseScope,
        media_id: &str,
        kind: &str,
        mime_hint: &str,
    ) -> Result<String> {
        self.reap_expired()?;
        if !scope.valid()
            || media_id.is_empty()
            || !matches!(kind, "image" | "audio" | "video" | "file")
            || mime_hint.is_empty()
        {
            return Err(error(
                "INVALID_INPUT",
                "Media scope, identity, kind or MIME is invalid",
            ));
        }
        let (file, path) = tempfile::NamedTempFile::new_in(&self.session)
            .map_err(io_error)?
            .into_parts();
        let handle = Uuid::new_v4().to_string();
        self.leases.insert(
            handle.clone(),
            Lease {
                scope: scope.clone(),
                expires: Instant::now() + self.ttl,
                state: State::Writing(Staged {
                    file: Some(file),
                    path: Some(path),
                    hasher: Sha256::new(),
                    length: 0,
                    kind: kind.into(),
                    mime_hint: mime_hint.into(),
                    media_id: media_id.into(),
                    auto_kind: false,
                }),
            },
        );
        Ok(handle)
    }
    pub fn create(&mut self, scope: &LeaseScope, kind: &str, mime_hint: &str) -> Result<String> {
        self.begin(scope, &Uuid::new_v4().to_string(), kind, mime_hint)
    }
    /// Host-only download sink: discover its kind from completed bytes, never
    /// from an untrusted HTTP Content-Type or a guessed extension.
    pub fn create_detected(&mut self, scope: &LeaseScope) -> Result<String> {
        let handle = self.create(scope, "file", "application/octet-stream")?;
        if let State::Writing(staged) = &mut self.leases.get_mut(&handle).unwrap().state {
            staged.auto_kind = true;
        }
        Ok(handle)
    }
    fn lease(&self, scope: &LeaseScope, handle: &str) -> Result<&Lease> {
        let lease = self.leases.get(handle).ok_or_else(|| {
            error(
                "PERMISSION_DENIED",
                "Media handle is unavailable or expired",
            )
        })?;
        if &lease.scope != scope || Instant::now() >= lease.expires {
            return Err(error(
                "PERMISSION_DENIED",
                "Media handle does not belong to this live request",
            ));
        }
        Ok(lease)
    }
    pub fn write(
        &mut self,
        scope: &LeaseScope,
        handle: &str,
        bytes: &[u8],
        control: &Control,
    ) -> Result<()> {
        check(control)?;
        self.lease(scope, handle)?;
        if bytes.len() > budget("maxMediaChunkBytes") {
            return Err(error(
                "BUDGET_EXCEEDED",
                "Media write exceeds the published chunk budget",
            ));
        }
        let mut lease = self.leases.remove(handle).expect("checked lease");
        let State::Writing(staged) = &mut lease.state else {
            self.leases.insert(handle.into(), lease);
            return Err(error("INVALID_INPUT", "Finished media cannot be appended"));
        };
        let next = staged
            .length
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| error("BUDGET_EXCEEDED", "Media byte count overflow"))?;
        staged
            .file
            .as_mut()
            .expect("live staging file")
            .write_all(bytes)
            .map_err(io_error)?;
        staged.hasher.update(bytes);
        staged.length = next;
        check(control)?;
        self.leases.insert(handle.into(), lease);
        Ok(())
    }
    pub fn import_file(
        &mut self,
        scope: &LeaseScope,
        media_id: &str,
        path: &Path,
        kind: &str,
        control: &Control,
    ) -> Result<LeasedMedia> {
        check(control)?;
        let source = self.workspace.contained(path)?;
        let mut source = File::open(source).map_err(io_error)?;
        let handle = self.begin(scope, media_id, kind, "application/octet-stream")?;
        let imported = (|| {
            let mut buffer = vec![0u8; budget("maxMediaChunkBytes")];
            loop {
                check(control)?;
                let count = source.read(&mut buffer).map_err(io_error)?;
                if count == 0 {
                    break;
                }
                self.write(scope, &handle, &buffer[..count], control)?;
            }
            self.finish(scope, &handle, control)
        })();
        if imported.is_err() {
            if let Some(mut lease) = self.leases.remove(&handle) {
                if let State::Writing(staged) = &mut lease.state {
                    staged.cleanup()?;
                }
            }
        }
        imported
    }
    pub fn finish(
        &mut self,
        scope: &LeaseScope,
        handle: &str,
        control: &Control,
    ) -> Result<LeasedMedia> {
        check(control)?;
        self.lease(scope, handle)?;
        let mut lease = self.leases.remove(handle).expect("checked lease");
        let State::Writing(staged) = &mut lease.state else {
            self.leases.insert(handle.into(), lease);
            return Err(error("INVALID_INPUT", "Media was already finalized"));
        };
        staged
            .file
            .as_mut()
            .expect("staging file")
            .flush()
            .map_err(io_error)?;
        staged.file.as_ref().unwrap().sync_all().map_err(io_error)?;
        let digest = hex::encode(staged.hasher.clone().finalize());
        let path = staged.path.as_ref().unwrap();
        if staged.auto_kind {
            staged.kind = infer::get_from_path(path)
                .map_err(io_error)?
                .map(|v| media_kind(v.mime_type()).to_string())
                .unwrap_or_else(|| "file".into());
        }
        let (stat, extension) = inspect(
            path,
            &staged.kind,
            &staged.mime_hint,
            staged.length,
            &digest,
            &staged.media_id,
            self.probe.as_ref(),
            control,
        )?;
        let identity: MediaIdentity = serde_json::from_value(
            json!({"id":staged.media_id,"digest":digest,"kind":stat.r#kind}),
        )
        .expect("same closed media kind");
        let file_name = format!("{digest}.{extension}");
        let object = self.workspace.root.join("objects").join(&file_name);
        drop(staged.file.take());
        let path = staged.path.take().unwrap();
        match path.persist_noclobber(&object) {
            Ok(()) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                verify_object(&object, &digest, staged.length, control)?;
                drop(error.path);
            }
            Err(_) => {
                return Err(error(
                    "RESOURCE_UNAVAILABLE",
                    "Cannot publish completed development media",
                ))
            }
        }
        let grant = Grant {
            package: scope.package.clone(),
            binding_id: scope.binding_id.clone(),
            identity,
            stat,
            file_name,
        };
        let record_path = self.grant_path(scope, &grant.identity);
        let record = serde_json::to_vec(&grant).expect("media grant serializes");
        if record.len() > budget("maxFrameBytes") {
            return Err(error(
                "BUDGET_EXCEEDED",
                "Media metadata exceeds the published frame budget",
            ));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(self.workspace.root.join("grants"))
            .map_err(io_error)?;
        temporary.write_all(&record).map_err(io_error)?;
        temporary.as_file().sync_all().map_err(io_error)?;
        match temporary.persist_noclobber(&record_path) {
            Ok(_) => {}
            Err(existing) if existing.error.kind() == io::ErrorKind::AlreadyExists => {
                let previous = bounded_json(&record_path, budget("maxFrameBytes"))?;
                if previous != serde_json::to_value(&grant).unwrap() {
                    return Err(error(
                        "INVALID_INPUT",
                        "Existing media identity has different measured facts",
                    ));
                }
            }
            Err(_) => {
                return Err(error(
                    "RESOURCE_UNAVAILABLE",
                    "Cannot persist development media identity",
                ))
            }
        }
        let file = File::open(&object).map_err(io_error)?;
        let modified = file
            .metadata()
            .and_then(|m| m.modified())
            .map_err(io_error)?;
        let result = LeasedMedia {
            handle: handle.into(),
            identity: grant.identity.clone(),
            stat: grant.stat.clone(),
            file_name: grant.file_name.clone(),
        };
        lease.state = State::Ready(Ready {
            grant,
            file,
            modified,
        });
        self.leases.insert(handle.into(), lease);
        Ok(result)
    }
    fn grant_path(&self, scope: &LeaseScope, identity: &MediaIdentity) -> PathBuf {
        let key = serde_json::to_vec(&json!([
            scope.package,
            scope.binding_id,
            identity.r#id,
            identity.r#digest
        ]))
        .unwrap();
        self.workspace
            .root
            .join("grants")
            .join(format!("{}.json", sha(&key)))
    }
    pub fn rehydrate(
        &mut self,
        scope: &LeaseScope,
        identity: &MediaIdentity,
        control: &Control,
    ) -> Result<LeasedMedia> {
        check(control)?;
        self.reap_expired()?;
        if !scope.valid() {
            return Err(error("INVALID_INPUT", "Media lease scope is invalid"));
        }
        let grant: Grant = serde_json::from_value(bounded_json(
            &self.grant_path(scope, identity),
            budget("maxFrameBytes"),
        )?)
        .map_err(|_| error("INVALID_INPUT", "Stored media identity is malformed"))?;
        if grant.package != scope.package
            || grant.binding_id != scope.binding_id
            || grant.identity != *identity
            || !valid_basename(&grant.file_name)
            || !grant
                .file_name
                .starts_with(&format!("{}.", identity.r#digest))
        {
            return Err(error(
                "PERMISSION_DENIED",
                "Saved media belongs to another package or binding",
            ));
        }
        let object = self
            .workspace
            .contained(&self.workspace.root.join("objects").join(&grant.file_name))?;
        let length = decimal_u64(&grant.stat.r#byte_length)
            .map_err(|_| error("INVALID_INPUT", "Stored media length is invalid"))?;
        verify_object(&object, &identity.r#digest, length, control)?;
        let (measured, extension) = inspect(
            &object,
            &serde_json::to_value(&grant.stat.r#kind)
                .unwrap()
                .as_str()
                .unwrap(),
            &grant.stat.r#mime,
            length,
            &identity.r#digest,
            &identity.r#id,
            self.probe.as_ref(),
            control,
        )?;
        if measured != grant.stat || grant.file_name != format!("{}.{extension}", identity.r#digest)
        {
            return Err(error(
                "INVALID_INPUT",
                "Saved media metadata no longer matches its content",
            ));
        }
        let file = File::open(object).map_err(io_error)?;
        let modified = file
            .metadata()
            .and_then(|m| m.modified())
            .map_err(io_error)?;
        let handle = Uuid::new_v4().to_string();
        let result = LeasedMedia {
            handle: handle.clone(),
            identity: identity.clone(),
            stat: grant.stat.clone(),
            file_name: grant.file_name.clone(),
        };
        self.leases.insert(
            handle,
            Lease {
                scope: scope.clone(),
                expires: Instant::now() + self.ttl,
                state: State::Ready(Ready {
                    grant,
                    file,
                    modified,
                }),
            },
        );
        Ok(result)
    }
    /// Restore an SDK-confirmed standard descriptor from this broker's durable store.
    /// The embedding host must establish request ownership from its confirmed journal;
    /// this method measures content and restores a fresh lease, never the old handle.
    pub fn rehydrate_descriptor(
        &mut self,
        scope: &LeaseScope,
        descriptor: &bgx_adapter_v2_ports::wire::MediaInputV2,
        control: &Control,
    ) -> Result<LeasedMedia> {
        let descriptor_value = serde_json::to_value(descriptor).map_err(|_| error("INVALID_INPUT", "Invalid media descriptor"))?;
        bgx_adapter_v2_validator::SchemaRegistry::validate_public("media-input", &descriptor_value)
            .map_err(|_| error("INVALID_INPUT", "Media descriptor violates the standard schema"))?;
        // Store layout is private to this native broker; consumers never parse filenames/grants.
        let digest = descriptor.r#file_name.split_once('.').map(|(digest, _)| digest)
            .filter(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            .ok_or_else(|| error("INVALID_INPUT", "Descriptor has no SDK durable content identity"))?;
        let identity: MediaIdentity = serde_json::from_value(json!({"id":descriptor.r#id,"digest":digest,"kind":descriptor.r#kind}))
            .map_err(|_| error("INVALID_INPUT", "Invalid durable media identity"))?;
        let lease = self.rehydrate(scope, &identity, control)?;
        let stat = serde_json::to_value(&lease.stat).map_err(|_| error("INVALID_INPUT", "Invalid measured media"))?;
        if ["id", "kind", "mime", "byteLength"].iter().any(|key| stat[*key] != descriptor_value[*key])
            || lease.file_name != descriptor.r#file_name {
            self.release(scope, &lease.handle)?;
            return Err(error("PERMISSION_DENIED", "Descriptor differs from measured durable media"));
        }
        Ok(lease)
    }
    pub fn info(&self, scope: &LeaseScope, handle: &str) -> Result<MediaStat> {
        let State::Ready(ready) = &self.lease(scope, handle)?.state else {
            return Err(error(
                "INVALID_INPUT",
                "Media output has not been finalized",
            ));
        };
        Ok(ready.grant.stat.clone())
    }
    pub fn identity(&self, scope: &LeaseScope, handle: &str) -> Result<MediaIdentity> {
        let State::Ready(ready) = &self.lease(scope, handle)?.state else {
            return Err(error(
                "INVALID_INPUT",
                "Unfinished media has no durable identity",
            ));
        };
        Ok(ready.grant.identity.clone())
    }
    pub fn file_name(&self, scope: &LeaseScope, handle: &str) -> Result<String> {
        let State::Ready(ready) = &self.lease(scope, handle)?.state else {
            return Err(error(
                "INVALID_INPUT",
                "Unfinished media has no measured file name",
            ));
        };
        Ok(ready.grant.file_name.clone())
    }
    pub fn read(
        &mut self,
        scope: &LeaseScope,
        handle: &str,
        offset: u64,
        max_bytes: usize,
        control: &Control,
    ) -> Result<Vec<u8>> {
        check(control)?;
        self.lease(scope, handle)?;
        if max_bytes > budget("maxMediaChunkBytes") {
            return Err(error(
                "BUDGET_EXCEEDED",
                "Media read exceeds the published chunk budget",
            ));
        }
        let State::Ready(ready) = &mut self.leases.get_mut(handle).unwrap().state else {
            return Err(error("INVALID_INPUT", "Unfinished media cannot be read"));
        };
        let length = decimal_u64(&ready.grant.stat.r#byte_length).expect("broker length");
        let metadata = ready.file.metadata().map_err(io_error)?;
        if metadata.len() != length || metadata.modified().map_err(io_error)? != ready.modified {
            return Err(error(
                "INVALID_INPUT",
                "Leased media changed after content verification",
            ));
        }
        if offset > length {
            return Err(error(
                "INVALID_INPUT",
                "Media offset is outside its measured byte range",
            ));
        }
        let count = (length - offset).min(max_bytes as u64) as usize;
        let mut bytes = vec![0; count];
        ready.file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        ready.file.read_exact(&mut bytes).map_err(io_error)?;
        check(control)?;
        Ok(bytes)
    }
    pub fn release(&mut self, scope: &LeaseScope, handle: &str) -> Result<()> {
        self.lease(scope, handle)?;
        let mut lease = self.leases.remove(handle).unwrap();
        if let State::Writing(staged) = &mut lease.state {
            staged.cleanup()?;
        }
        Ok(())
    }
    /// Host-owned cleanup is idempotent, including a failed finalize which has
    /// already consumed its staging lease. Existing foreign handles still fail.
    pub fn discard(&mut self, scope: &LeaseScope, handle: &str) -> Result<()> {
        let Some(lease) = self.leases.get(handle) else {
            return Ok(());
        };
        if &lease.scope != scope {
            return Err(error(
                "PERMISSION_DENIED",
                "Cannot discard another request's media",
            ));
        }
        let mut lease = self.leases.remove(handle).unwrap();
        if let State::Writing(staged) = &mut lease.state {
            staged.cleanup()?;
        }
        Ok(())
    }
    pub fn release_request(&mut self, scope: &LeaseScope) -> Result<()> {
        let handles: Vec<_> = self
            .leases
            .iter()
            .filter(|(_, l)| &l.scope == scope)
            .map(|(h, _)| h.clone())
            .collect();
        for h in handles {
            let mut lease = self.leases.remove(&h).unwrap();
            if let State::Writing(staged) = &mut lease.state {
                staged.cleanup()?;
            }
        }
        Ok(())
    }
    pub fn reap_expired(&mut self) -> Result<()> {
        let expired: Vec<_> = self
            .leases
            .iter()
            .filter(|(_, l)| Instant::now() >= l.expires)
            .map(|(h, _)| h.clone())
            .collect();
        for h in expired {
            let mut lease = self.leases.remove(&h).unwrap();
            if let State::Writing(staged) = &mut lease.state {
                staged.cleanup()?;
            }
        }
        Ok(())
    }
    pub fn validation_context(&self, scope: &LeaseScope) -> MediaValidationContext {
        let mut leases_by_handle = BTreeMap::new();
        for (handle, lease) in &self.leases {
            if &lease.scope != scope || Instant::now() >= lease.expires {
                continue;
            }
            let State::Ready(ready) = &lease.state else {
                continue;
            };
            let stat = &ready.grant.stat;
            leases_by_handle.insert(
                handle.clone(),
                MediaLease {
                    package: scope.package.clone(),
                    binding_id: scope.binding_id.clone(),
                    request_id: scope.request_id.clone(),
                    media_id: ready.grant.identity.r#id.clone(),
                    live: true,
                    facts: MediaFacts {
                        digest: stat.r#sha256.clone(),
                        kind: serde_json::to_value(&stat.r#kind)
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .into(),
                        mime: stat.r#mime.clone(),
                        byte_length: decimal_u64(&stat.r#byte_length).unwrap(),
                        file_name: ready.grant.file_name.clone(),
                        width: stat.r#width,
                        height: stat.r#height,
                        duration_ms: stat
                            .r#duration_ms
                            .as_deref()
                            .map(|s| decimal_u64(s).unwrap()),
                        timestamps_ms: stat
                            .r#timestamps_ms
                            .as_ref()
                            .map(|v| v.iter().map(|s| decimal_u64(s).unwrap()).collect()),
                    },
                },
            );
        }
        MediaValidationContext {
            package: scope.package.clone(),
            binding_id: scope.binding_id.clone(),
            request_id: scope.request_id.clone(),
            leases_by_handle,
            uploads_by_ref: BTreeMap::new(),
        }
    }
    pub fn close(mut self) -> Result<()> {
        let leases = std::mem::take(&mut self.leases);
        for (_, mut lease) in leases {
            if let State::Writing(staged) = &mut lease.state {
                staged.cleanup()?;
            }
        }
        fs::remove_dir(&self.session).map_err(io_error)?;
        Ok(())
    }
}
impl Drop for MediaBroker {
    fn drop(&mut self) {
        self.leases.clear();
        if self.session.exists() {
            if let Err(error) = fs::remove_dir(&self.session) {
                eprintln!("SDK media session cleanup failed: {error}");
            }
        }
    }
}
fn bounded_json(path: &Path, max: usize) -> Result<Value> {
    let mut bytes = vec![];
    File::open(path)
        .map_err(io_error)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > max {
        return Err(error(
            "BUDGET_EXCEEDED",
            "Development metadata exceeds its public resource budget",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| error("INVALID_INPUT", "Development metadata is invalid JSON"))
}
fn verify_object(path: &Path, digest: &str, length: u64, control: &Control) -> Result<()> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut hasher = Sha256::new();
    let mut bytes = vec![0; budget("maxMediaChunkBytes")];
    let mut count = 0u64;
    loop {
        check(control)?;
        let read = file.read(&mut bytes).map_err(io_error)?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or_else(|| error("BUDGET_EXCEEDED", "Media length overflow"))?;
        hasher.update(&bytes[..read]);
    }
    if count != length || hex::encode(hasher.finalize()) != digest {
        return Err(error("INVALID_INPUT", "Saved media content digest differs"));
    }
    Ok(())
}
fn inspect(
    path: &Path,
    kind: &str,
    mime_hint: &str,
    length: u64,
    digest: &str,
    media_id: &str,
    tool: Option<&ProbeTool>,
    control: &Control,
) -> Result<(MediaStat, String)> {
    check(control)?;
    let found = infer::get_from_path(path).map_err(io_error)?;
    let (mime, extension) = match found {
        Some(found) => (found.mime_type().to_string(), found.extension().to_string()),
        None if kind == "file" && mime_hint == "application/octet-stream" => {
            ("application/octet-stream".into(), "bin".into())
        }
        None => {
            return Err(error(
                "INVALID_INPUT",
                "Media format cannot be identified from its bytes",
            ))
        }
    };
    let detected = media_kind(&mime);
    if kind != "file" && kind != detected {
        return Err(error(
            "INVALID_INPUT",
            "Media kind differs from its byte signature",
        ));
    }
    let (mut width, mut height) = (None, None);
    let (mut duration, mut timestamps) = (None, None);
    if detected == "image" {
        let dimensions = image::ImageReader::open(path)
            .map_err(io_error)?
            .with_guessed_format()
            .map_err(io_error)?
            .into_dimensions()
            .map_err(|_| {
                error(
                    "INVALID_INPUT",
                    "Image dimensions cannot be read from the actual content",
                )
            })?;
        width = Some(dimensions.0);
        height = Some(dimensions.1);
    }
    if matches!(detected, "video" | "audio") {
        if let Some(tool) = tool {
            let facts = probe(path, tool, control)?;
            let stream = facts["streams"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["codec_type"] == detected))
                .ok_or_else(|| {
                    error(
                        "INVALID_INPUT",
                        "Probe returned no stream matching the actual media kind",
                    )
                })?;
            if detected == "video" {
                width = Some(
                    stream["width"]
                        .as_u64()
                        .and_then(|v| v.try_into().ok())
                        .ok_or_else(|| error("INVALID_INPUT", "Probed video width is invalid"))?,
                );
                height = Some(
                    stream["height"]
                        .as_u64()
                        .and_then(|v| v.try_into().ok())
                        .ok_or_else(|| error("INVALID_INPUT", "Probed video height is invalid"))?,
                );
            }
            if let Some(value) = stream
                .get("duration")
                .or_else(|| facts.get("format").and_then(|v| v.get("duration")))
            {
                duration = Some(
                    milliseconds(value.as_str().ok_or_else(|| {
                        error("INVALID_INPUT", "Probe duration is not a decimal string")
                    })?)?
                    .to_string(),
                );
            }
            if tool.frame_timestamps && detected == "video" {
                let mut values = vec![];
                for frame in facts["frames"].as_array().ok_or_else(|| {
                    error("INVALID_INPUT", "Probe omitted requested frame timestamps")
                })? {
                    if frame["media_type"] != "video" {
                        continue;
                    }
                    values.push(signed_milliseconds(
                        frame["best_effort_timestamp_time"]
                            .as_str()
                            .ok_or_else(|| error("INVALID_INPUT", "Frame timestamp unavailable"))?,
                    )?);
                }
                let origin = *values.first().ok_or_else(|| {
                    error(
                        "INVALID_INPUT",
                        "Video probe returned no actual frame timestamps",
                    )
                })?;
                let values:Vec<u64>=values.into_iter().map(|v|u64::try_from(v-origin).map_err(|_|error("INVALID_INPUT","Frame timestamp is before the selected source origin or exceeds u64"))).collect::<Result<_>>()?;
                if values.windows(2).any(|w| w[0] > w[1])
                    || values.iter().any(|v| {
                        duration
                            .as_ref()
                            .and_then(|d| decimal_u64(d).ok())
                            .is_some_and(|d| *v > d)
                    })
                {
                    return Err(error(
                        "INVALID_INPUT",
                        "Actual frame timestamps are outside the source timeline",
                    ));
                }
                timestamps = Some(
                    values
                        .into_iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
    let stat=serde_json::from_value(json!({"id":media_id,"fileName":format!("{digest}.{extension}"),"kind":kind,"mime":mime,"byteLength":length.to_string(),"sha256":digest,"width":width,"height":height,"durationMs":duration,"timestampsMs":timestamps})).expect("measured media DTO");
    Ok((stat, extension))
}
fn milliseconds(value: &str) -> Result<u64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(error(
            "INVALID_INPUT",
            "Probe time is not an unsigned decimal",
        ));
    }
    let whole = whole
        .parse::<u64>()
        .map_err(|_| error("INVALID_INPUT", "Probe time exceeds u64"))?;
    let mut ms = 0u64;
    let mut digits = fraction.bytes();
    for _ in 0..3 {
        ms = ms * 10 + digits.next().map(|b| (b - b'0') as u64).unwrap_or(0);
    }
    whole
        .checked_mul(1000)
        .and_then(|w| w.checked_add(ms))
        .ok_or_else(|| error("INVALID_INPUT", "Probe milliseconds overflow"))
}
fn signed_milliseconds(value: &str) -> Result<i128> {
    match value.strip_prefix('-') {
        Some(unsigned) => Ok(-i128::from(milliseconds(unsigned)?)),
        None => Ok(i128::from(milliseconds(value)?)),
    }
}
fn media_kind(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("video/") {
        "video"
    } else if mime.starts_with("audio/") {
        "audio"
    } else {
        "file"
    }
}
fn probe(path: &Path, tool: &ProbeTool, control: &Control) -> Result<Value> {
    let deadline = Instant::now()
        .checked_add(tool.timeout)
        .ok_or_else(|| error("INVALID_INPUT", "Probe deadline overflow"))?;
    let entries = if tool.frame_timestamps {
        "stream=codec_type,width,height,duration:format=duration:frame=media_type,best_effort_timestamp_time"
    } else {
        "stream=codec_type,width,height,duration:format=duration"
    };
    let mut command = Command::new(&tool.executable);
    let detected = infer::get_from_path(path)
        .map_err(io_error)?
        .ok_or_else(|| error("INVALID_INPUT", "Probe source format is unknown"))?;
    let selected = if detected.mime_type().starts_with("video/") {
        "v:0"
    } else {
        "a:0"
    };
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-select_streams",
            selected,
            "-show_entries",
            entries,
            "-of",
            "json",
            "-i",
        ])
        .arg(path)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(io_error)?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let overflow = Arc::new(AtomicBool::new(false));
    let out_overflow = overflow.clone();
    let output = thread::spawn(move || {
        let max = budget("maxFrameBytes");
        let mut data = vec![];
        let result = stdout.by_ref().take(max as u64 + 1).read_to_end(&mut data);
        if data.len() > max {
            out_overflow.store(true, Ordering::SeqCst);
        }
        result.map(|_| data)
    });
    let diagnostic = thread::spawn(move || {
        let max = budget("maxDiagnosticBytes");
        let mut tail = vec![];
        let mut buffer = vec![0; max];
        loop {
            let count = stderr.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            tail.extend_from_slice(&buffer[..count]);
            if tail.len() > max {
                tail.drain(..tail.len() - max);
            }
        }
        Ok::<_, io::Error>(tail)
    });
    let mut failure = None;
    let status = loop {
        if let Err(error) = check(control) {
            failure = Some(error);
            let _ = child.kill();
            break child.wait();
        }
        if overflow.load(Ordering::SeqCst) {
            failure = Some(error(
                "BUDGET_EXCEEDED",
                "Probe output exceeds the public frame budget",
            ));
            let _ = child.kill();
            break child.wait();
        }
        if Instant::now() >= deadline {
            failure = Some(error("TIMEOUT", "Explicit media probe deadline exhausted"));
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
        }
    };
    let bytes = output
        .join()
        .map_err(|_| error("INTERNAL_ERROR", "Probe output reader failed"))?
        .map_err(io_error)?;
    let diagnostic = diagnostic
        .join()
        .map_err(|_| error("INTERNAL_ERROR", "Probe diagnostic reader failed"))?
        .map_err(io_error)?;
    if let Some(error) = failure {
        return Err(error);
    }
    if overflow.load(Ordering::SeqCst) {
        return Err(error(
            "BUDGET_EXCEEDED",
            "Probe output exceeds the public frame budget",
        ));
    }
    if !status.map_err(io_error)?.success() {
        eprintln!(
            "SDK media probe failed: {}",
            String::from_utf8_lossy(&diagnostic)
        );
        return Err(error(
            "INVALID_INPUT",
            "Media probe could not inspect this content",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| error("INVALID_INPUT", "Media probe returned malformed JSON"))
}
