//! Shared checkpoint rules over pluggable rows. The bundled rows are the development
//! fixture; an embedding host supplies its own durable rows. All calls are blocking.
//! No product database discovery, migrations or repair here.
use crate::media::DevelopmentWorkspace;
use bgx_adapter_v2_ports::{decimal_u64, resource_policy};
use bgx_adapter_v2_validator::SchemaRegistry;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointError {
    pub code: String,
    pub message: &'static str,
}
impl std::fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for CheckpointError {}
pub type Result<T> = std::result::Result<T, CheckpointError>;
pub(crate) fn fail(code: &'static str, message: &'static str) -> CheckpointError {
    CheckpointError {
        code: code.into(),
        message,
    }
}
fn storage(_: rusqlite::Error) -> CheckpointError {
    fail(
        "RESOURCE_UNAVAILABLE",
        "Development checkpoint storage is unavailable",
    )
}
fn limit() -> usize {
    resource_policy()["limits"]["maxStateBytes"]
        .as_u64()
        .unwrap() as usize
}

/// Explicit known credential material, provided by the trusted native host.
/// No Serialize or Debug; no field-name guesses or silent redaction.
pub struct SnapshotPolicy {
    secrets: Vec<Zeroizing<Vec<u8>>>,
}
impl SnapshotPolicy {
    pub fn new(secrets: Vec<Zeroizing<Vec<u8>>>) -> Result<Self> {
        if secrets.iter().any(|s| s.is_empty())
            || secrets
                .iter()
                .fold(0usize, |n, s| n.saturating_add(s.len()))
                > limit()
        {
            return Err(fail(
                "INVALID_INPUT",
                "Explicit secret policy is invalid or over budget",
            ));
        }
        Ok(Self { secrets })
    }
    pub fn check(&self, value: &Value) -> Result<()> {
        fn strings(value: &Value, test: &impl Fn(&str) -> bool) -> bool {
            match value {
                Value::String(s) => test(s),
                Value::Array(a) => a.iter().any(|v| strings(v, test)),
                Value::Object(o) => o.iter().any(|(k, v)| test(k) || strings(v, test)),
                _ => false,
            }
        }
        if strings(value, &|s| {
            self.secrets.iter().any(|secret| {
                s.as_bytes()
                    .windows(secret.len())
                    .any(|part| part == secret.as_slice())
            })
        }) {
            return Err(fail(
                "PERMISSION_DENIED",
                "Secret material cannot enter a checkpoint",
            ));
        }
        Ok(())
    }
    fn encode(&self, value: &Value) -> Result<Vec<u8>> {
        self.check(value)?;
        let bytes = serde_json::to_vec(value)
            .map_err(|_| fail("INVALID_INPUT", "Checkpoint cannot be serialized"))?;
        if bytes.len() > limit() {
            return Err(fail(
                "BUDGET_EXCEEDED",
                "Checkpoint exceeds public state byte budget",
            ));
        }
        Ok(bytes)
    }
}

pub(crate) struct Saved {
    pub checkpoint: Value,
    pub frozen: Value,
}
/// Row storage behind the shared checkpoint rules. A backend only persists bytes with an
/// exact compare-and-swap; schema, secret, budget and state rules stay in `CheckpointStore`,
/// so the development fixture and an embedding product host enforce the same contract.
pub trait CheckpointRows: Send {
    /// Stored (revision, checkpoint, frozen). Byte fields may be cut after `max + 1` bytes.
    fn get(&self, request_id: &str, max: usize) -> Result<Option<(String, Vec<u8>, Vec<u8>)>>;
    /// `Ok(false)` when the request already owns a row.
    fn insert(&mut self, request_id: &str, checkpoint: &[u8], frozen: &[u8]) -> Result<bool>;
    /// `Ok(false)` unless the stored row is exactly (`old_revision`, `old`).
    fn swap(
        &mut self,
        request_id: &str,
        old_revision: &str,
        old: &[u8],
        revision: &str,
        new: &[u8],
    ) -> Result<bool>;
}

/// The standalone development fixture (`checkpoint-fixtures.sqlite` in the marked workspace).
pub struct SqliteFixtureRows {
    connection: Connection,
}
impl SqliteFixtureRows {
    pub fn open(workspace: &DevelopmentWorkspace) -> Result<Self> {
        // Recheck marker/containment; never accept an arbitrary SQLite path.
        DevelopmentWorkspace::open(workspace.root()).map_err(|_| {
            fail(
                "PERMISSION_DENIED",
                "Development workspace marker is invalid",
            )
        })?;
        let path = workspace.root().join("checkpoint-fixtures.sqlite");
        let fresh = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                file.sync_all().map_err(|_| {
                    fail(
                        "RESOURCE_UNAVAILABLE",
                        "Checkpoint file cannot be synchronized",
                    )
                })?;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(_) => {
                return Err(fail(
                    "RESOURCE_UNAVAILABLE",
                    "Checkpoint file cannot be created",
                ))
            }
        };
        if path
            .canonicalize()
            .ok()
            .as_deref()
            .and_then(std::path::Path::parent)
            != Some(workspace.root())
        {
            return Err(fail(
                "PERMISSION_DENIED",
                "Checkpoint file escapes development workspace",
            ));
        }
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(storage)?;
        connection
            .busy_timeout(std::time::Duration::ZERO)
            .map_err(storage)?;
        connection
            .execute_batch(
                "PRAGMA synchronous=FULL; PRAGMA journal_mode=DELETE; PRAGMA trusted_schema=OFF;",
            )
            .map_err(storage)?;
        if fresh {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE checkpoint_fixture(request_id TEXT PRIMARY KEY NOT NULL, revision TEXT NOT NULL, checkpoint BLOB NOT NULL, frozen BLOB NOT NULL) STRICT; PRAGMA user_version=2; COMMIT;").map_err(storage)?;
        }
        Self::checked(connection)
    }
    /// Opens an existing marked fixture without creating or updating any checkpoint data.
    pub fn open_read_only(workspace: &DevelopmentWorkspace) -> Result<Self> {
        DevelopmentWorkspace::open(workspace.root()).map_err(|_| fail("PERMISSION_DENIED", "Invalid development workspace"))?;
        let path = workspace.root().join("checkpoint-fixtures.sqlite");
        if path.canonicalize().ok().as_ref() != Some(&path) || !path.is_file() {
            return Err(fail("PERMISSION_DENIED", "Original checkpoint fixture is missing or redirected"));
        }
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(storage)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF;").map_err(storage)?;
        Self::checked(connection)
    }
    fn checked(connection: Connection) -> Result<Self> {
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(storage)?;
        let schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='checkpoint_fixture' AND type='table'",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if version != 2 || schema != "CREATE TABLE checkpoint_fixture(request_id TEXT PRIMARY KEY NOT NULL, revision TEXT NOT NULL, checkpoint BLOB NOT NULL, frozen BLOB NOT NULL) STRICT" {
            return Err(fail("INVALID_INPUT", "Checkpoint fixture schema differs; no automatic migration is allowed"));
        }
        Ok(Self { connection })
    }
}
impl CheckpointRows for SqliteFixtureRows {
    fn get(&self, request_id: &str, max: usize) -> Result<Option<(String, Vec<u8>, Vec<u8>)>> {
        self.connection.query_row("SELECT revision,substr(checkpoint,1,?2),substr(frozen,1,?2) FROM checkpoint_fixture WHERE request_id=?1", params![request_id,(max+1) as i64], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)
    }
    fn insert(&mut self, request_id: &str, checkpoint: &[u8], frozen: &[u8]) -> Result<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM checkpoint_fixture WHERE request_id=?1)",
                [request_id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if exists {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO checkpoint_fixture VALUES(?1,'1',?2,?3)",
            params![request_id, checkpoint, frozen],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(true)
    }
    fn swap(
        &mut self,
        request_id: &str,
        old_revision: &str,
        old: &[u8],
        revision: &str,
        new: &[u8],
    ) -> Result<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let changed = tx.execute("UPDATE checkpoint_fixture SET revision=?1,checkpoint=?2 WHERE request_id=?3 AND revision=?4 AND checkpoint=?5", params![revision,new,request_id,old_revision,old]).map_err(storage)?;
        if changed != 1 {
            return Ok(false);
        }
        tx.commit().map_err(storage)?;
        Ok(true)
    }
}

pub struct CheckpointStore {
    rows: Box<dyn CheckpointRows>,
    policy: SnapshotPolicy,
}
impl CheckpointStore {
    /// Development fixture store in the marked workspace.
    pub fn open(workspace: &DevelopmentWorkspace, policy: SnapshotPolicy) -> Result<Self> {
        Ok(Self::with_rows(Box::new(SqliteFixtureRows::open(workspace)?), policy))
    }
    /// Same rules over an embedding host's own durable rows (e.g. a product task table).
    pub fn with_rows(rows: Box<dyn CheckpointRows>, policy: SnapshotPolicy) -> Self {
        Self { rows, policy }
    }
    pub(crate) fn load(&self, request_id: &str) -> Result<Option<Saved>> {
        let Some((revision, a, b)) = self.rows.get(request_id, limit())? else {
            return Ok(None);
        };
        if a.len() > limit() || b.len() > limit() {
            return Err(fail(
                "BUDGET_EXCEEDED",
                "Stored checkpoint exceeds public state byte budget",
            ));
        }
        let checkpoint: Value = serde_json::from_slice(&a)
            .map_err(|_| fail("INVALID_INPUT", "Stored checkpoint JSON is corrupt"))?;
        let frozen: Value = serde_json::from_slice(&b)
            .map_err(|_| fail("INVALID_INPUT", "Frozen request JSON is corrupt"))?;
        self.policy.check(&checkpoint)?;
        self.policy.check(&frozen)?;
        validate(&checkpoint)?;
        if checkpoint["requestId"] != request_id || checkpoint["revision"] != revision {
            return Err(fail(
                "INVALID_INPUT",
                "Stored checkpoint index differs from its identity",
            ));
        }
        Ok(Some(Saved { checkpoint, frozen }))
    }
    /// Public checkpoint port read; the caller supplies its trusted authorized root request.
    pub fn read(&self, authorized_request: &str, request_id: &str) -> Result<Option<Value>> {
        if authorized_request != request_id {
            return Err(fail(
                "PERMISSION_DENIED",
                "Checkpoint belongs to another request",
            ));
        }
        self.load(request_id).map(|s| s.map(|s| s.checkpoint))
    }
    pub(crate) fn create(&mut self, checkpoint: &Value, frozen: &Value) -> Result<()> {
        validate(checkpoint)?;
        if checkpoint["revision"] != "1" || checkpoint["state"] != "prepared" {
            return Err(fail(
                "INVALID_INPUT",
                "Initial checkpoint must be prepared at revision one",
            ));
        }
        let a = self.policy.encode(checkpoint)?;
        let b = self.policy.encode(frozen)?;
        if !self
            .rows
            .insert(checkpoint["requestId"].as_str().unwrap(), &a, &b)?
        {
            return Err(fail(
                "CHECKPOINT_CONFLICT",
                "Request already has a checkpoint; do not resubmit",
            ));
        }
        Ok(())
    }
    pub(crate) fn update(&mut self, old: &Value, new: &Value) -> Result<()> {
        validate(new)?;
        let old_revision = decimal_u64(old["revision"].as_str().unwrap())
            .map_err(|_| fail("INVALID_INPUT", "Checkpoint revision is invalid"))?;
        let next = old_revision
            .checked_add(1)
            .ok_or_else(|| fail("BUDGET_EXCEEDED", "Checkpoint revision is exhausted"))?;
        if new["requestId"] != old["requestId"] || new["revision"] != next.to_string() {
            return Err(fail(
                "CHECKPOINT_CONFLICT",
                "Checkpoint revision does not follow compare-and-swap",
            ));
        }
        let a = self.policy.encode(new)?;
        let previous = serde_json::to_vec(old).unwrap();
        if !self.rows.swap(
            old["requestId"].as_str().unwrap(),
            &old_revision.to_string(),
            &previous,
            &next.to_string(),
            &a,
        )? {
            return Err(fail(
                "CHECKPOINT_CONFLICT",
                "Checkpoint changed; reload the original request",
            ));
        }
        Ok(())
    }
}
pub(crate) fn validate(value: &Value) -> Result<()> {
    SchemaRegistry::validate_public("checkpoint", value)
        .map_err(|_| fail("INVALID_INPUT", "Checkpoint differs from public schema"))?;
    decimal_u64(value["revision"].as_str().unwrap())
        .map_err(|_| fail("INVALID_INPUT", "Checkpoint revision is outside u64"))?;
    let mut ids=std::collections::BTreeSet::new();
    if value["mediaIdentities"].as_array().unwrap().iter().any(|id|!ids.insert(id["id"].as_str().unwrap())){return Err(fail("INVALID_INPUT","Checkpoint media identity is duplicated"));}
    let result = &value["result"];
    let terminal = result["kind"] == "completed"
        || matches!(
            result["status"].as_str(),
            Some("succeeded" | "failed" | "cancelled" | "expired")
        );
    let valid_state = match value["state"].as_str().unwrap() {
        "prepared" | "acceptance_unknown" => {
            value["vendorTaskId"].is_null() && value["resumeToken"].is_null() && result.is_null()
        }
        "accepted" => {
            !terminal
                && value["vendorTaskId"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
                && (result["kind"] == "accepted"
                    || matches!(result["status"].as_str(), Some("queued" | "running")))
                && value["resumeToken"] == result["resumeToken"]
                && (result.get("vendorTaskId").is_none()
                    || result["vendorTaskId"] == value["vendorTaskId"])
        }
        "terminal" => {
            terminal
                && value["resumeToken"] == result["resumeToken"]
                && (result.get("vendorTaskId").is_none()
                    || result["vendorTaskId"] == value["vendorTaskId"])
        }
        _ => false,
    };
    if !valid_state {
        return Err(fail(
            "INVALID_INPUT",
            "Checkpoint state, task identity, token and result disagree",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Foreign rows (an embedding host's table) that return fixed stored bytes.
    struct Fixed(Vec<u8>);
    impl CheckpointRows for Fixed {
        fn get(&self, _: &str, max: usize) -> Result<Option<(String, Vec<u8>, Vec<u8>)>> {
            let cut = self.0.len().min(max + 1);
            Ok(Some(("1".into(), self.0[..cut].to_vec(), b"{}".to_vec())))
        }
        fn insert(&mut self, _: &str, _: &[u8], _: &[u8]) -> Result<bool> {
            Ok(false)
        }
        fn swap(&mut self, _: &str, _: &str, _: &[u8], _: &str, _: &[u8]) -> Result<bool> {
            Ok(false)
        }
    }

    #[test]
    fn host_rows_get_the_same_budget_and_schema_rules() {
        let store = |bytes: Vec<u8>| {
            CheckpointStore::with_rows(Box::new(Fixed(bytes)), SnapshotPolicy::new(vec![]).unwrap())
        };
        let big = store(vec![b' '; limit() + 10]);
        assert_eq!(big.read("r", "r").unwrap_err().code, "BUDGET_EXCEEDED");
        let corrupt = store(b"{not json".to_vec());
        assert_eq!(corrupt.read("r", "r").unwrap_err().code, "INVALID_INPUT");
        let off_schema = store(br#"{"requestId":"r","revision":"1"}"#.to_vec());
        assert_eq!(off_schema.read("r", "r").unwrap_err().code, "INVALID_INPUT");
        assert_eq!(off_schema.read("a", "r").unwrap_err().code, "PERMISSION_DENIED");
    }
}
