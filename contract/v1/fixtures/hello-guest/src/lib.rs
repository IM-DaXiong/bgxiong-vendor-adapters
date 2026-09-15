//! Minimal WIT world guest used as the L2 rust smoke (H-10).
//! Imports Host interfaces; probe has no vendor side effects.

wit_bindgen::generate!({
    path: "../../wit/vendor-adapter.wit",
    world: "vendor-adapter",
});

struct Hello;

impl Guest for Hello {
    fn invoke(request: Invocation) -> Response {
        bgxiong::vendor_adapter::host_log::log("info", "hello-probe", "wit invoke");
        match request.operation {
            Operation::Probe => Response {
                protocol_version: 1,
                operation: Operation::Probe,
                data_json: Some(r#"{"ok":true}"#.into()),
                error: None,
            },
            other => Response {
                protocol_version: 1,
                operation: other,
                data_json: None,
                error: Some(AdapterError {
                    code: "adapterCapabilityDenied".into(),
                    message: "hello fixture only implements probe".into(),
                    retryable: false,
                    vendor_code: None,
                    vendor_request_id: None,
                }),
            },
        }
    }
}

export!(Hello);
