#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{format, string::String};
use bgx_adapter_v2_sdk::{guest, Adapter, Host, Invocation, OperationSubmitV2, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

// Replace this LOCAL publisher identity in both source and manifest when authoring.
const PLUGIN_ID: &str = "local.sdk.echo";
const TEXT_SCHEMA: &str = r#"{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}"#;
const CONFIG_SCHEMA: &str = r#"{"type":"object","properties":{},"required":[],"additionalProperties":false}"#;
fn schema(id: &str, source: &str) -> Value {
    let digest: String = Sha256::digest(source.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    json!({"id":id,"digest":digest,"schemaJson":source})
}
fn capabilities() -> Value {
    let input = schema("input", TEXT_SCHEMA);
    let output = schema("output", TEXT_SCHEMA);
    let config = schema("configuration", CONFIG_SCHEMA);
    json!({"schemaVersion":2,"pluginId":PLUGIN_ID,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[{
        "schemaVersion":2,"localId":"echo","capabilityKey":format!("{PLUGIN_ID}/echo"),"transportSlot":"generic","modelIds":["example-model"],
        "configurationSchema":{"id":"configuration","digest":config["digest"]},
        "operations":[{"id":format!("{PLUGIN_ID}/echo"),"version":1,"label":"Echo text",
            "inputSchema":{"id":"input","digest":input["digest"]},"outputSchema":{"id":"output","digest":output["digest"]},"resumeSchema":null,
            "uiSchema":{"controls":[{"fieldPointer":"/text","widget":"multiline","label":"Text","description":"","group":"Input"}]},"mediaSlots":[],
            "execution":{"mode":"synchronous","streaming":false,"cancellation":"unsupported","resumable":false,"remoteIdempotency":false},
            "outputSlots":[{"role":"answer","kind":"text","minCount":1,"maxCount":null}]}],
        "schemas":[input,output,config]
    }]})
}
fn failure(request: &Invocation, code: &str, message: &str) -> Response {
    serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,
        "error":{"code":code,"message":message,"retryable":false,"vendorCode":null,"vendorRequestId":null}})).expect("closed public error")
}
pub struct Echo;
impl Adapter for Echo {
    fn invoke(request: Invocation, _host: &mut dyn Host) -> Response {
        let operation = serde_json::to_value(&request.r#operation).expect("closed operation");
        let result = match operation.as_str().expect("operation string") {
            "capabilities" => capabilities(),
            "probe" => json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]}),
            "submit" => {
                let input: OperationSubmitV2 = match serde_json::from_str(&request.r#payload_json) {
                    Ok(input) => input,
                    Err(_) => return failure(&request, "INVALID_INPUT", "Submit does not match the public request contract"),
                };
                let Some(text) = input.r#input.get("text").and_then(Value::as_str) else {
                    return failure(&request, "INVALID_INPUT", "Echo requires its declared text input");
                };
                json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,
                    "outputs":[{"kind":"text","role":"answer","text":text}],"data":{"text":text},"usage":null})
            }
            _ => return failure(&request, "UNSUPPORTED_FEATURE", "This synchronous example does not implement the selected operation"),
        };
        guest::completed_response(&request, &result).expect("serializable public result")
    }
}
bgx_adapter_v2_sdk::export_adapter!(Echo);

#[cfg(test)]
mod tests {
    use super::*;
    struct NoEffects;
    impl Host for NoEffects {
        fn call(&mut self, _: bgx_adapter_v2_sdk::HostPortCallV2) -> Result<Value,bgx_adapter_v2_sdk::host::HostError> {
            panic!("The local echo must not invoke filesystem/network/credential ports")
        }
    }
    fn request(operation: &str, payload: Value) -> Invocation {
        serde_json::from_value(json!({"protocolVersion":2,"operation":operation,"profileId":"binding","bindingId":"binding","capabilitySlot":"generic",
            "requestId":"native-test","budgetMs":1000,"negotiatedVersion":2,"enabledFeatures":[],"payloadJson":payload.to_string()})).unwrap()
    }
    #[test]
    fn native_echo_uses_the_same_public_request_and_keeps_unicode() {
        let caps=capabilities();let digest=&caps["capabilityContracts"][0]["operations"][0]["inputSchema"]["digest"];
        let reply=Echo::invoke(request("submit",json!({"schemaVersion":2,"modelId":"example-model","capabilityKey":format!("{PLUGIN_ID}/echo"),"operationId":format!("{PLUGIN_ID}/echo"),
            "operationVersion":1,"inputSchemaDigest":digest,"input":{"text":"Hello，世界"},"media":[],"uploads":[],"configuration":{}})),&mut NoEffects);
        let response=serde_json::to_value(reply).unwrap();let data:Value=serde_json::from_str(response["dataJson"].as_str().unwrap()).unwrap();assert_eq!(data["data"]["text"],"Hello，世界");
    }
}
