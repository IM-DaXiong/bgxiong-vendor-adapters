//! Anonymous starter guest. Import L3 probe is offline. Replace submit/query
//! with host-http to your vendor; do not call host-http from empty probe.

#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
extern crate alloc;

#[cfg(target_arch = "wasm32")]
use alloc::format;
#[cfg(target_arch = "wasm32")]
use alloc::string::ToString;
#[cfg(target_arch = "wasm32")]
use alloc::vec;
#[cfg(target_arch = "wasm32")]
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(any(target_arch = "wasm32", test))]
mod wasm_mem;

wit_bindgen::generate!({
    path: "wit/vendor-adapter.wit",
    world: "vendor-adapter",
});

use bgx_vendor_adapter_sdk::{query_result, submit_accepted, Output, QueryResult};
use bgxiong::vendor_adapter::types::{AdapterError, Operation};

struct Starter;

impl Guest for Starter {
    fn invoke(request: Invocation) -> Response {
        bgxiong::vendor_adapter::host_log::log("info", "starter", "invoke");
        match request.operation {
            Operation::Capabilities => ok_json(
                Operation::Capabilities,
                serde_json::json!({"slots":["image"],"models":["echo"]}),
            ),
            Operation::Probe => ok_json(Operation::Probe, serde_json::json!({"ok":true})),
            Operation::Submit => submit(&request),
            Operation::Query => query(&request),
            Operation::Cancel => err(
                request.operation,
                "adapterCapabilityDenied",
                "starter does not implement cancel",
            ),
            Operation::Upload => err(
                request.operation,
                "adapterCapabilityDenied",
                "starter does not implement upload",
            ),
        }
    }
}

fn submit(request: &Invocation) -> Response {
    // Live vendor I/O goes through host-http on submit/query only.
    // Import L3 probe must not call host-http (empty payload, no credential).
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(e) => {
            return err(
                Operation::Submit,
                "adapterBadOutput",
                &format!("payload: {e}"),
            )
        }
    };
    let _ = payload.get("prompt");
    let sdk = submit_accepted("starter-task-not-live");
    sdk_to_wit(Operation::Submit, sdk)
}

fn query(request: &Invocation) -> Response {
    let payload: serde_json::Value = match serde_json::from_str(&request.payload_json) {
        Ok(v) => v,
        Err(e) => {
            return err(
                Operation::Query,
                "adapterBadOutput",
                &format!("payload: {e}"),
            )
        }
    };
    let _ = payload.get("vendorTaskId");
    let sdk = query_result(&QueryResult {
        status: "failed".into(),
        outputs: Vec::<Output>::new(),
        progress_text: Some("starter query is not connected to a vendor".into()),
        retry_after_ms: None,
    });
    sdk_to_wit(Operation::Query, sdk)
}

fn ok_json(op: Operation, data: serde_json::Value) -> Response {
    Response {
        protocol_version: 1,
        operation: op,
        data_json: Some(data.to_string()),
        error: None,
    }
}

fn err(op: Operation, code: &str, message: &str) -> Response {
    Response {
        protocol_version: 1,
        operation: op,
        data_json: None,
        error: Some(AdapterError {
            code: code.into(),
            message: message.into(),
            retryable: false,
            vendor_code: None,
            vendor_request_id: None,
        }),
    }
}

fn sdk_to_wit(op: Operation, sdk: bgx_vendor_adapter_sdk::Response) -> Response {
    Response {
        protocol_version: sdk.protocol_version,
        operation: op,
        data_json: sdk.data_json,
        error: sdk.error.map(|e| AdapterError {
            code: e.code,
            message: e.message,
            retryable: e.retryable,
            vendor_code: e.vendor_code,
            vendor_request_id: e.vendor_request_id,
        }),
    }
}

export!(Starter);
