//! Public before-send channel. S0 advertises only before-send, never media-stream.
use bgx_adapter_v2_reference_host::before_send::{BeforeSendSink, PersistFuture};
use bgx_adapter_v2_validator::SchemaRegistry;
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};

fn error(code: &str, message: &str) -> bgx_adapter_v2_ports::wire::host::HostError {
    serde_json::from_value(json!({"code":code,"message":message})).expect("public host error")
}

pub struct AuditChannel {
    file: Arc<Mutex<File>>,
    external: bool,
}
impl AuditChannel {
    pub async fn open(directory: &Path, external: bool) -> crate::Result<Self> {
        if external {
            let (send, receive) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let result = crate::host_rows::exchange(
                    json!({"schemaVersion":1,"type":"channel_hello","features":["before-send"]}),
                );
                let _ = send.send(result);
            });
            let reply = tokio::time::timeout(std::time::Duration::from_secs(30), receive)
                .await
                .map_err(|_| {
                    crate::workspace::invalid("host-channel", "Host channel handshake timed out")
                })?
                .map_err(|_| crate::workspace::invalid("host-channel", "Host channel closed"))?
                .map_err(|_| {
                    crate::workspace::invalid("host-channel", "Host channel handshake failed")
                })?;
            if reply["schemaVersion"] != 1 {
                return Err(crate::error(
                    "UNSUPPORTED_VERSION",
                    "host-channel",
                    "Unsupported host channel version",
                    "Use host-channel schemaVersion 1",
                ));
            }
            SchemaRegistry::validate_public("host-channel-v1", &reply).map_err(|_| {
                crate::workspace::invalid("host-channel", "Invalid public channel acceptance")
            })?;
            if reply["type"] != "channel_accept"
                || reply["enabledFeatures"] != json!(["before-send"])
            {
                return Err(crate::error(
                    "UNSUPPORTED_FEATURE",
                    "host-channel",
                    "S0 requires before-send and does not implement media-stream",
                    "Accept only the advertised before-send feature",
                ));
            }
        }
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join("before-send.jsonl"))
            .map_err(|_| {
                crate::workspace::invalid("before-send", "Cannot create the request audit log")
            })?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            external,
        })
    }
}
impl BeforeSendSink for AuditChannel {
    fn persist<'a>(&'a self, record: &'a Value) -> PersistFuture<'a> {
        let file = self.file.clone();
        let record = record.clone();
        let external = self.external;
        Box::pin(async move {
            SchemaRegistry::validate_public("host-channel-v1", &record).map_err(|_| {
                error(
                    "INVALID_INPUT",
                    "Outbound audit record violates the public contract",
                )
            })?;
            let (send, receive) = tokio::sync::oneshot::channel();
            // A detached stdio reader cannot keep Tokio shutdown waiting after a request deadline.
            std::thread::spawn(move || {
                let result = (|| {
                    {
                        let mut file = file
                            .lock()
                            .map_err(|_| error("INTERNAL_ERROR", "Audit log lock failed"))?;
                        serde_json::to_writer(&mut *file, &record)
                            .map_err(|_| error("RESOURCE_UNAVAILABLE", "Audit log write failed"))?;
                        file.write_all(b"\n")
                            .and_then(|_| file.flush())
                            .and_then(|_| file.sync_data())
                            .map_err(|_| {
                                error("RESOURCE_UNAVAILABLE", "Audit log persistence failed")
                            })?;
                    }
                    if external {
                        let ack = crate::host_rows::exchange(record.clone()).map_err(|_| {
                            error(
                                "RESOURCE_UNAVAILABLE",
                                "Host audit acknowledgement unavailable",
                            )
                        })?;
                        SchemaRegistry::validate_public("host-channel-v1", &ack).map_err(|_| {
                            error("INVALID_INPUT", "Malformed host audit acknowledgement")
                        })?;
                        if ack["type"] != "audit_ack" || ack["exchangeId"] != record["exchangeId"] {
                            return Err(error(
                                "INVALID_INPUT",
                                "Host audit acknowledgement identity mismatch",
                            ));
                        }
                        if ack["result"]["persisted"] != true {
                            return Err(error(
                                "RESOURCE_UNAVAILABLE",
                                "Host refused outbound persistence; no HTTP request was sent",
                            ));
                        }
                    }
                    Ok(())
                })();
                let _ = send.send(result);
            });
            receive.await.map_err(|_| {
                error(
                    "RESOURCE_UNAVAILABLE",
                    "Outbound persistence channel closed",
                )
            })?
        })
    }
}
