use crate::{Cancellation, Runtime, RuntimeFault};
use bgx_adapter_v2_ports::{
    decimal_u64, read_frame, resource_policy,
    wire::{self, Host, HostPortCallV2, Invocation, ManifestV2, Response, WorkerEnvelopeV2},
    write_frame,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

fn packet(value: Value) -> anyhow::Result<WorkerEnvelopeV2> {
    Ok(serde_json::from_value(value)?)
}
fn bytes(path: &Path) -> anyhow::Result<Vec<u8>> {
    let mut data = Vec::new();
    let max = resource_policy()["limits"]["maxModuleBytes"]
        .as_u64()
        .unwrap();
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut data)?;
    anyhow::ensure!(
        data.len() as u64 <= max,
        "Package metadata exceeds resource budget"
    );
    Ok(data)
}
fn sha(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}
fn protocol() -> Value {
    serde_json::from_str(include_str!(
        "../../../protocol/vendor-adapter-protocol.json"
    ))
    .expect("protocol")
}
fn finish(
    writer: &Arc<Mutex<io::Stdout>>,
    request: &Invocation,
    response: Response,
) -> anyhow::Result<()> {
    let frame = packet(
        json!({"schemaVersion":2,"kind":"response","requestId":request.r#request_id,"response":response}),
    )?;
    match write_frame(&mut *writer.lock().expect("stdout mutex"), &frame) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            let response = crate::engine::error_response(
                request,
                &RuntimeFault::new(
                    "RESOURCE_UNAVAILABLE",
                    "Response exceeds the transport resource budget",
                ),
            );
            let frame = packet(
                json!({"schemaVersion":2,"kind":"response","requestId":request.r#request_id,"response":response}),
            )?;
            write_frame(&mut *writer.lock().expect("stdout mutex"), &frame)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub struct PackageDescriptor {
    pub root: PathBuf,
    pub module: PathBuf,
    pub module_hash: String,
    pub identity: Value,
    pub manifest: ManifestV2,
}
impl PackageDescriptor {
    pub fn load(root: &Path, expected_identity: &Value) -> anyhow::Result<Self> {
        let root = root.canonicalize()?;
        let manifest_bytes = bytes(&root.join("manifest.json"))?;
        let manifest: ManifestV2 = serde_json::from_slice(&manifest_bytes)?;
        let value = serde_json::to_value(&manifest)?;
        anyhow::ensure!(
            value["language"] == "rust",
            "This worker executes Rust WASM packages only"
        );
        let protocol = protocol();
        let inventory = bytes(&root.join(protocol["signature"]["hashManifest"].as_str().unwrap()))?;
        let identity = json!({"pluginId":value["pluginId"],"version":value["version"],"digest":sha(&inventory)});
        anyhow::ensure!(
            &identity == expected_identity,
            "Package identity differs from frozen invocation"
        );
        let files = wire::guest::parse_hash_inventory(std::str::from_utf8(&inventory)?)
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            files.get("manifest.json") == Some(&sha(&manifest_bytes)),
            "Manifest content differs from signed inventory"
        );
        let entry = value["entry"].as_str().unwrap();
        anyhow::ensure!(
            wire::guest::valid_package_path(entry),
            "Unsafe package entry"
        );
        let module = root.join(entry).canonicalize()?;
        anyhow::ensure!(module.starts_with(&root), "Module escapes package root");
        let module_hash = value["artifactSha256"].as_str().unwrap().to_owned();
        anyhow::ensure!(
            files.get(entry) == Some(&module_hash),
            "Entry digest differs from inventory"
        );
        Ok(Self {
            root,
            module,
            module_hash,
            identity,
            manifest,
        })
    }
}

struct Proxy {
    request_id: String,
    next: u64,
    writer: Arc<Mutex<io::Stdout>>,
    incoming: mpsc::Receiver<anyhow::Result<WorkerEnvelopeV2>>,
    cancel: Cancellation,
    fault: Arc<Mutex<Option<String>>>,
}
impl Proxy {
    fn fail(&self, reason: &str) -> wire::host::HostError {
        *self.fault.lock().expect("fault mutex") = Some(reason.into());
        self.cancel.cancel();
        wire::host::invalid_wire(reason)
    }
}
impl Host for Proxy {
    fn call(&mut self, call: HostPortCallV2) -> Result<Value, wire::host::HostError> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| self.fail("Worker call identity exhausted"))?;
        let outgoing=packet(json!({"schemaVersion":2,"kind":"port_call","requestId":self.request_id,"callId":self.next.to_string(),"call":call})).map_err(|_|self.fail("Cannot encode port frame"))?;
        write_frame(&mut *self.writer.lock().expect("stdout mutex"), &outgoing)
            .map_err(|_| self.fail("Parent transport is unavailable"))?;
        loop {
            if self.cancel.is_cancelled() {
                return Err(wire::host::invalid_wire("Local execution was interrupted"));
            }
            match self.incoming.recv_timeout(Duration::from_millis(5)) {
                Ok(Ok(frame)) => {
                    let frame = serde_json::to_value(frame).expect("typed worker frame");
                    if frame["kind"] != "port_result"
                        || frame["requestId"] != self.request_id
                        || decimal_u64(frame["callId"].as_str().unwrap()).ok() != Some(self.next)
                    {
                        return Err(self.fail("Port result belongs to a different request or call"));
                    }
                    if let Some(value) = frame["result"].get("ok") {
                        return Ok(value.clone());
                    }
                    return Err(serde_json::from_value(frame["result"]["error"].clone())
                        .map_err(|_| self.fail("Malformed host error"))?);
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if self.cancel.is_cancelled() {
                        return Err(wire::host::invalid_wire("Local execution was interrupted"));
                    }
                    return Err(self.fail("Parent ended before returning the managed port result"))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }
    }
}

/// One process owns one invocation. Concurrent invocations use independent workers.
/// Stdin stays open until the final response; there are no global interrupts.
pub fn run_stdio(package_root: &Path) -> anyhow::Result<()> {
    let mut reader = io::stdin();
    let hello = read_frame(&mut reader)?.ok_or_else(|| anyhow::anyhow!("Missing handshake"))?;
    let value = serde_json::to_value(&hello)?;
    anyhow::ensure!(value["kind"] == "hello", "First frame must be handshake");
    anyhow::ensure!(
        value["policyDigest"] == sha(include_bytes!("../../../protocol/schema-policy.json")),
        "Worker policy digest differs"
    );
    let writer = Arc::new(Mutex::new(io::stdout()));
    write_frame(&mut *writer.lock().expect("stdout mutex"), &hello)?;
    let invoke = read_frame(&mut reader)?.ok_or_else(|| anyhow::anyhow!("Missing invocation"))?;
    let value = serde_json::to_value(invoke)?;
    anyhow::ensure!(value["kind"] == "invoke", "Second frame must be invocation");
    let request: Invocation = serde_json::from_value(value["invocation"].clone())?;
    anyhow::ensure!(
        value["requestId"] == request.r#request_id,
        "Request correlation differs"
    );
    let descriptor = match PackageDescriptor::load(package_root, &value["package"]) {
        Ok(value) => value,
        Err(error) => {
            let response = crate::engine::error_response(
                &request,
                &RuntimeFault::new("INVALID_PACKAGE", error.to_string()),
            );
            return finish(&writer, &request, response);
        }
    };
    let cancel = Cancellation::default();
    let reader_cancel = cancel.clone();
    let request_id = request.r#request_id.clone();
    let expected_id = request_id.clone();
    let fault = Arc::new(Mutex::new(None::<String>));
    let reader_fault = fault.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("sdk-v2-parent-frames".into())
        .spawn(move || loop {
            match read_frame(&mut reader) {
                Ok(Some(frame)) => {
                    let value = serde_json::to_value(&frame).expect("typed worker frame");
                    if value["requestId"] != expected_id {
                        *reader_fault.lock().expect("fault mutex") =
                            Some("Cross-request worker frame".into());
                        reader_cancel.cancel();
                        break;
                    }
                    if value["kind"] == "cancel" {
                        reader_cancel.cancel();
                        break;
                    }
                    if value["kind"] != "port_result" {
                        *reader_fault.lock().expect("fault mutex") =
                            Some("Unexpected frame during invocation".into());
                        reader_cancel.cancel();
                        break;
                    }
                    if sender.send(Ok(frame)).is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    *reader_fault.lock().expect("fault mutex") =
                        Some("Parent closed before final response".into());
                    reader_cancel.cancel();
                    break;
                }
                Err(error) => {
                    *reader_fault.lock().expect("fault mutex") =
                        Some("Invalid or truncated parent frame".into());
                    reader_cancel.cancel();
                    let _ = sender.send(Err(error.into()));
                    break;
                }
            }
        })?;
    let proxy = Proxy {
        request_id: request_id.clone(),
        next: 0,
        writer: writer.clone(),
        incoming: receiver,
        cancel: cancel.clone(),
        fault: fault.clone(),
    };
    let result = match Runtime::new() {
        Ok(mut runtime) => runtime.execute_file(
            &descriptor.module,
            &descriptor.module_hash,
            request.clone(),
            Box::new(proxy),
            descriptor.manifest.r#limits,
            cancel,
        ),
        Err(error) => Err(error),
    };
    let response: Response = if let Some(reason) = fault.lock().expect("fault mutex").clone() {
        crate::engine::error_response(&request, &RuntimeFault::new("INVALID_INPUT", reason))
    } else {
        match result {
            Ok(response) => response,
            Err(error) => crate::engine::error_response(&request, &error),
        }
    };
    finish(&writer, &request, response)
}
