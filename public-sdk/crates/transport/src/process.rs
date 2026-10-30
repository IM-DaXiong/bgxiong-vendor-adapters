use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
};

/// Transport deadlines are explicit host resource policy, never business quotas.
#[derive(Clone)]
pub struct Options {
    pub handshake_timeout: Duration,
    pub write_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub cancel_grace: Duration,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultKind {
    InvalidConfiguration,
    Spawn,
    Handshake,
    Protocol,
    Exited,
    Io,
    Deadline,
    Cancelled,
    Supervisor,
}
#[derive(Debug, Clone)]
pub struct Fault {
    pub kind: FaultKind,
    pub message: String,
    /// A missing response after dispatch MUST NOT cause automatic submit retry.
    pub invocation_dispatched: bool,
    pub worker_pid: Option<u32>,
    /// Bounded diagnostic bytes for development logs, not user-facing copy.
    pub stderr_tail: Vec<u8>,
}
impl Fault {
    pub(crate) fn new(kind: FaultKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            invocation_dispatched: false,
            worker_pid: None,
            stderr_tail: vec![],
        }
    }
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}
impl std::error::Error for Fault {}
pub(crate) fn worker_path(sdk: &Path, worker: &Path, options: &Options) -> Result<PathBuf, Fault> {
    if [
        options.handshake_timeout,
        options.write_timeout,
        options.shutdown_timeout,
        options.cancel_grace,
    ]
    .iter()
    .any(Duration::is_zero)
    {
        return Err(Fault::new(
            FaultKind::InvalidConfiguration,
            "Transport deadlines must be explicit and positive",
        ));
    }
    if !sdk.is_absolute() || !worker.is_absolute() {
        return Err(Fault::new(
            FaultKind::InvalidConfiguration,
            "SDK and worker paths must be absolute",
        ));
    }
    let sdk = sdk.canonicalize().map_err(|_| {
        Fault::new(
            FaultKind::InvalidConfiguration,
            "Explicit SDK directory is unavailable",
        )
    })?;
    let worker = worker.canonicalize().map_err(|_| {
        Fault::new(
            FaultKind::InvalidConfiguration,
            "Explicit SDK worker is unavailable",
        )
    })?;
    if !worker.starts_with(&sdk) || !worker.is_file() {
        return Err(Fault::new(
            FaultKind::InvalidConfiguration,
            "Worker must be a file inside the selected SDK directory",
        ));
    }
    let policy = std::fs::read(sdk.join("protocol/schema-policy.json")).map_err(|_| {
        Fault::new(
            FaultKind::InvalidConfiguration,
            "Selected SDK has no public policy",
        )
    })?;
    if policy != include_bytes!("../../../protocol/schema-policy.json") {
        return Err(Fault::new(
            FaultKind::InvalidConfiguration,
            "Selected SDK policy differs from the linked transport",
        ));
    }
    Ok(worker)
}
pub(crate) fn spawn(worker: &Path, package: &Path) -> Result<Child, Fault> {
    if !package.is_absolute() {
        return Err(Fault::new(
            FaultKind::InvalidConfiguration,
            "Package root must be explicit and absolute",
        ));
    }
    let package = package.canonicalize().map_err(|_| {
        Fault::new(
            FaultKind::InvalidConfiguration,
            "Explicit package root is unavailable",
        )
    })?;
    if !package.is_dir() {
        return Err(Fault::new(
            FaultKind::InvalidConfiguration,
            "Package root is not a directory",
        ));
    }
    let mut command = Command::new(worker);
    command
        .arg("--package")
        .arg(&package)
        .current_dir(&package)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
    }
    command.spawn().map_err(|_| {
        Fault::new(
            FaultKind::Spawn,
            "Cannot start explicitly selected SDK worker",
        )
    })
}
pub(crate) async fn drain_tail<R: AsyncRead + Unpin>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let max = bgx_adapter_v2_ports::resource_policy()["limits"]["maxDiagnosticBytes"]
        .as_u64()
        .unwrap() as usize;
    let mut tail = Vec::new();
    let mut chunk = vec![0u8; max.max(1)];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        tail.extend_from_slice(&chunk[..count]);
        if tail.len() > max {
            tail.drain(..tail.len() - max);
        }
    }
    Ok(tail)
}
