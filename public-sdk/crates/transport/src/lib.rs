//! Explicit SDK worker transport. Hosts own policy and asynchronous port effects;
//! this crate never links Wasmtime, discovers production workers, or reads secrets.
pub mod client;
pub mod framing;
pub mod process;
pub use bgx_adapter_v2_ports::wire;
pub use client::{Control, ManagedHost, PortFuture, Receipt, WorkerClient};
pub use process::{Fault, FaultKind, Options};
