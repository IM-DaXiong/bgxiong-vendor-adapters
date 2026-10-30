#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use bgx_adapter_v2_sdk::{guest, Adapter, Host, Invocation, OperationSubmitV2, Response};
use serde_json::{json, Value};

// Replace this LOCAL publisher identity in both source and manifest when authoring.
fn capabilities() -> Value {
    serde_json::from_str(include_str!("capabilities.json")).expect("generated speech profile")
}
fn failure(request: &Invocation, code: &str, message: &str) -> Response {
    serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,
        "error":{"code":code,"message":message,"retryable":false,"vendorCode":null,"vendorRequestId":null}})).expect("closed public error")
}
pub struct SpeechTemplate;
impl Adapter for SpeechTemplate {
    fn invoke(request: Invocation, _host: &mut dyn Host) -> Response {
        let operation = serde_json::to_value(&request.r#operation).expect("closed operation");
        let result = match operation.as_str().expect("operation string") {
            "capabilities" => capabilities(),
            "probe" => {
                json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]})
            }
            "submit" => {
                let input: OperationSubmitV2 = match serde_json::from_str(&request.r#payload_json) {
                    Ok(input) => input,
                    Err(_) => {
                        return failure(
                            &request,
                            "INVALID_INPUT",
                            "Submit does not match the public request contract",
                        )
                    }
                };
                if input.r#operation_id != "std/speech.synthesize" {
                    return failure(
                        &request,
                        "UNSUPPORTED_FEATURE",
                        "This template declares only ordinary synthesis",
                    );
                }
                // Implement the package-owned runtime mapping here using the managed
                // host HTTP port. Do not return fixture audio or claim inference works.
                return failure(
                    &request,
                    "RESOURCE_UNAVAILABLE",
                    "Author template: model runtime mapping has not been implemented",
                );
            }
            _ => {
                return failure(
                    &request,
                    "UNSUPPORTED_FEATURE",
                    "This synchronous example does not implement the selected operation",
                )
            }
        };
        guest::completed_response(&request, &result).expect("serializable public result")
    }
}
bgx_adapter_v2_sdk::export_adapter!(SpeechTemplate);
