#![cfg_attr(target_arch = "wasm32", no_std)]
//! DeepSeek HTTP semantics live only in this independent author package.
extern crate alloc;
use alloc::{format, string::{String, ToString}, vec::Vec};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PLUGIN: &str = "local.bgx.deepseek-text";
const OPERATION: &str = "local.bgx.deepseek-text/complete";
const MODELS: &[&str] = &["deepseek-flash", "deepseek-v4-pro"];
const INPUT: &str = include_str!("../schemas/input.schema.json");
const CONFIG: &str = include_str!("../schemas/config.schema.json");
const OUTPUT: &str = include_str!("../schemas/output.schema.json");

#[derive(Debug)]
struct Failure { code: &'static str, message: String, vendor: Option<String>, request: Option<String> }
impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), vendor: None, request: None }
    }
}
fn invalid(message: &str) -> Failure { Failure::new("INVALID_INPUT", message) }
fn malformed() -> Failure { Failure::new("UPSTREAM_ERROR", "DeepSeek returned an invalid structured response; no retry attempted") }
fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, Failure> {
    value[field].as_str().ok_or_else(malformed)
}
fn schema(id: &str, source: &str) -> Value {
    let digest: String = Sha256::digest(source.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    json!({"id":id,"digest":digest,"schemaJson":source})
}
pub fn capabilities() -> Value {
    let input = schema("input", INPUT);
    let config = schema("config", CONFIG);
    let output = schema("output", OUTPUT);
    json!({"schemaVersion":2,"pluginId":PLUGIN,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[{
        "schemaVersion":2,"localId":"chat","capabilityKey":"text.generate","transportSlot":"text","modelIds":MODELS,
        "configurationSchema":{"id":"config","digest":config["digest"]},
        "operations":[{"id":OPERATION,"version":1,"label":"DeepSeek 文本补全",
            "inputSchema":{"id":"input","digest":input["digest"]},"outputSchema":{"id":"output","digest":output["digest"]},"resumeSchema":null,
            "uiSchema":{"controls":[{"fieldPointer":"/messages","widget":"repeat","label":"对话消息","description":"按原顺序保留完整多轮文本","group":"输入"},
                {"fieldPointer":"/parameters/maxTokens","widget":"integer","label":"输出上限","description":"max_tokens","group":"参数"},
                {"fieldPointer":"/parameters/thinking","widget":"toggle","label":"思考","description":"enabled / disabled","group":"参数"}]},
            "mediaSlots":[],"outputSlots":[{"role":"answer","kind":"text","minCount":1,"maxCount":1}],
            "execution":{"mode":"synchronous","streaming":false,"cancellation":"unsupported","resumable":false,"remoteIdempotency":false}}],
        "schemas":[input,config,output]
    }]})
}

/// The SDK validates the public schema; these checks also make direct native use explicit.
fn request_body(payload: &Value) -> Result<Value, Failure> {
    if !payload["modelId"].as_str().is_some_and(|m| MODELS.contains(&m)) || payload["operationId"] != OPERATION { return Err(invalid("Unknown selected model or operation")); }
    let input = payload["input"].as_object().ok_or_else(|| invalid("input must be an object"))?;
    if input.keys().any(|k| k != "messages" && k != "parameters") { return Err(invalid("Unsupported text input field")); }
    let messages = input.get("messages").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(|| invalid("messages must not be empty"))?;
    let mut projected = Vec::new();
    for message in messages {
        let fields = message.as_object().ok_or_else(|| invalid("message must be an object"))?;
        if fields.keys().any(|k| !["role","content","reasoningContent"].contains(&k.as_str())) { return Err(invalid("Unsupported message field")); }
        let role = text(message, "role")?;
        if !["system","user","assistant"].contains(&role) { return Err(invalid("Unsupported message role")); }
        let mut value = json!({"role":role,"content":text(message,"content")?});
        if let Some(reasoning) = message.get("reasoningContent") {
            if role != "assistant" || !reasoning.is_string() { return Err(invalid("Reasoning history belongs to assistant messages")); }
            value["reasoning_content"] = reasoning.clone();
        }
        projected.push(value);
    }
    let parameters = input.get("parameters").and_then(Value::as_object).ok_or_else(|| invalid("parameters must be an object"))?;
    if parameters.keys().any(|k| !["maxTokens","thinking","temperature"].contains(&k.as_str())) { return Err(invalid("Unsupported parameter; nothing was discarded")); }
    let mut body = json!({"model":payload["modelId"],"messages":projected,"stream":false});
    if let Some(max) = parameters.get("maxTokens") {
        if !max.as_u64().is_some_and(|v| (1..=393216).contains(&v)) { return Err(invalid("maxTokens is outside the documented range")); }
        body["max_tokens"] = max.clone();
    }
    if let Some(thinking) = parameters.get("thinking") {
        let enabled = thinking.as_bool().ok_or_else(|| invalid("thinking must be boolean"))?;
        body["thinking"] = json!({"type":if enabled {"enabled"} else {"disabled"}});
    }
    if let Some(temperature) = parameters.get("temperature") {
        if !temperature.as_f64().is_some_and(|v| (0.0..=2.0).contains(&v)) { return Err(invalid("DeepSeek temperature must be between 0 and 2")); }
        if parameters.get("thinking") != Some(&Value::Bool(false)) { return Err(invalid("DeepSeek temperature has no effect in thinking mode; explicitly disable thinking before setting temperature")); }
        body["temperature"] = temperature.clone();
    }
    Ok(body)
}

fn usage(response: &Value) -> Result<Value, Failure> {
    let Some(usage) = response.get("usage").filter(|v| !v.is_null()) else { return Ok(Value::Null); };
    if !usage.is_object() { return Err(malformed()); }
    let mut measurements = Vec::new();
    for (pointer, metric, unit) in [("/prompt_tokens","input_tokens","tokens"),("/completion_tokens","output_tokens","tokens"),("/total_tokens","total_tokens","tokens"),
        ("/prompt_cache_hit_tokens","cache_hit_input_tokens","tokens"),("/prompt_cache_miss_tokens","cache_miss_input_tokens","tokens"),
        ("/prompt_tokens_details/cached_tokens","cached_input_tokens","tokens"),("/completion_tokens_details/reasoning_tokens","reasoning_tokens","tokens")] {
        if let Some(value) = usage.pointer(pointer) {
            let amount = value.as_u64().ok_or_else(malformed)?;
            let metric = match metric {"input_tokens"|"output_tokens"|"total_tokens" => metric.to_string(), _ => format!("{PLUGIN}/{metric}")};
            measurements.push(json!({"metric":metric,"unit":unit,"amount":amount.to_string(),"source":"reported"}));
        }
    }
    if measurements.is_empty() { Ok(Value::Null) } else { Ok(json!({"measurements":measurements})) }
}

fn complete(response: &Value, model: &str) -> Result<Value, Failure> {
    if response["model"] != model { return Err(Failure::new("UPSTREAM_ERROR", "Returned model differs from the explicit selection")); }
    let choices = response["choices"].as_array().filter(|a| a.len()==1).ok_or_else(malformed)?;
    let choice = &choices[0];
    let reason = text(choice,"finish_reason")?;
    if !["stop","length","content_filter"].contains(&reason) { return Err(Failure::new("UNSUPPORTED_FEATURE", "This operation does not execute tool-call responses")); }
    let message = &choice["message"];
    if message["role"] != "assistant" || message.get("tool_calls").is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty())) { return Err(malformed()); }
    let content = text(message,"content")?;
    let reasoning = message.get("reasoning_content").cloned().unwrap_or(Value::Null);
    if !reasoning.is_null() && !reasoning.is_string() { return Err(malformed()); }
    let id = response.get("id").cloned().unwrap_or(Value::Null);
    if !id.is_null() && !id.is_string() { return Err(malformed()); }
    Ok(json!({"kind":"completed","vendorTaskId":null,"appliedParams":null,
        "outputs":[{"kind":"text","role":"answer","text":content}],
        "data":{"model":model,"providerRequestId":id,"finishReason":reason,"reasoningContent":reasoning,"assistantMessageJson":serde_json::to_string(message).map_err(|_|malformed())?},"usage":usage(response)?}))
}

fn submit(h: &mut dyn Host, request: &Invocation, payload: &Value) -> Result<Value, Failure> {
    let body = request_body(payload)?;
    let config = &payload["configuration"];
    let base = text(config,"endpointBaseUrl").map_err(|_| invalid("Explicit endpointBaseUrl is required"))?;
    let credential = text(config,"credentialRef").map_err(|_| invalid("Explicit credentialRef is required"))?;
    if credential.is_empty() { return Err(invalid("credentialRef must not be empty")); }
    let policy: Value = serde_json::from_str(include_str!("../../../../contract/v2/protocol/schema-policy.json")).expect("published SDK policy");
    let args = json!({"plan":{"method":"POST","url":format!("{}/chat/completions",base.trim_end_matches('/')),
        "headers":[{"name":"Content-Type","value":"application/json"}],
        "generatedHeaders":[{"kind":"bearer","name":"Authorization","targetField":"Authorization"}],
        "body":{"kind":"bytes","bytes":serde_json::to_vec(&body).map_err(|_|invalid("Request is not serializable"))?},
        "sink":{"kind":"buffer","maxBytes":policy["limits"]["maxBufferedHttpBytes"]},"timeoutMs":request.r#budget_ms},"credential":credential});
    let wire: Value = host::call(h,"host-http.execute",&args).map_err(|error| {
        let value = serde_json::to_value(&error).expect("host error");
        let denied = value["code"]=="PERMISSION_DENIED";
        let message = if denied {format!("Managed HTTP denied: {}; no retry attempted",host::diagnostic(&error))}
            else {"Managed HTTP did not return a complete response; no retry attempted".into()};
        Failure::new(if denied {"PERMISSION_DENIED"} else {"ACCEPTANCE_UNKNOWN"}, message)
    })?;
    let status = wire["status"].as_u64().ok_or_else(malformed)?;
    let bytes: Vec<u8> = serde_json::from_value(wire["payload"]["bytes"].clone()).map_err(|_| malformed())?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    if !(200..300).contains(&status) {
        let value = parsed.unwrap_or(Value::Null);
        let mut failure = Failure::new(match status {401|403=>"UNAUTHORIZED",429=>"RATE_LIMITED",_=>"UPSTREAM_ERROR"}, format!("DeepSeek HTTP {status}; no retry attempted"));
        failure.vendor = value["error"]["code"].as_str().map(str::to_string).or_else(|| value["error"]["code"].as_i64().map(|v|v.to_string()));
        failure.request = value["id"].as_str().map(str::to_string);
        return Err(failure);
    }
    let value = parsed.map_err(|_| malformed())?;
    if let Some(error) = value.get("error").filter(|e|!e.is_null()) {
        let mut failure=Failure::new("UPSTREAM_ERROR","DeepSeek reported a business failure; no retry attempted");
        failure.vendor=error["code"].as_str().map(str::to_string).or_else(||error["code"].as_i64().map(|v|v.to_string()));
        failure.request=value["id"].as_str().map(str::to_string);return Err(failure);
    }
    complete(&value, payload["modelId"].as_str().ok_or_else(||invalid("modelId is required"))?)
}

pub struct DeepseekText;
impl Adapter for DeepseekText {
    fn invoke(request: Invocation, h: &mut dyn Host) -> Response {
        let result = (|| {
            let payload: Value=serde_json::from_str(&request.r#payload_json).map_err(|_|invalid("Payload is not JSON"))?;
            let operation = serde_json::to_value(&request.r#operation).expect("typed operation");
            match operation.as_str() {
                Some("capabilities") => Ok(capabilities()),
                Some("probe") if payload["kind"]=="package_validation" => Ok(json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]})),
                Some("submit") => submit(h,&request,&payload),
                _ => Err(Failure::new("UNSUPPORTED_FEATURE","This text operation has no asynchronous, upload or tool execution lifecycle")),
            }
        })();
        match result {
            Ok(value)=>guest::completed_response(&request,&value).expect("serializable public response"),
            Err(error)=>serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,"error":{"code":error.code,"message":error.message,"retryable":false,"vendorCode":error.vendor,"vendorRequestId":error.request}})).expect("public error"),
        }
    }
}
bgx_adapter_v2_sdk::export_adapter!(DeepseekText);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_models_and_effective_parameters_are_explicit() {
        for model in MODELS {
            let mut payload=json!({"modelId":model,"operationId":OPERATION,"input":{"messages":[{"role":"user","content":"原文\n\n🐻"}],"parameters":{"thinking":false,"temperature":0.7,"maxTokens":128}}});
            let wire=request_body(&payload).unwrap();assert_eq!(wire["model"],*model);assert_eq!(wire["messages"],payload["input"]["messages"]);
            assert_eq!(wire["thinking"]["type"],"disabled");assert_eq!(wire["max_tokens"],128);
            payload["input"]["parameters"]["thinking"]=json!(true);assert!(request_body(&payload).is_err());
            payload["input"]["parameters"].as_object_mut().unwrap().remove("temperature");
            assert_eq!(request_body(&payload).unwrap()["thinking"]["type"],"enabled");
            payload["input"]["parameters"]["topP"]=json!(0.8);assert!(request_body(&payload).is_err());
        }
    }
    #[test]
    fn reported_zero_cache_usage_and_optional_reasoning_follow_the_public_result() {
        let response=json!({"id":"deepseek-fixture","model":"deepseek-flash","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"独立包测试"}}],
            "usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":5,"prompt_tokens_details":{"cached_tokens":0}}});
        let result=complete(&response,"deepseek-flash").unwrap();assert_eq!(result["outputs"][0]["text"],"独立包测试");assert!(result["data"]["reasoningContent"].is_null());
        assert_eq!(result["usage"]["measurements"][3],json!({"metric":"local.bgx.deepseek-text/cache_hit_input_tokens","unit":"tokens","amount":"0","source":"reported"}));
        assert!(complete(&response,"deepseek-v4-pro").is_err());
        let mut absent=response;absent.as_object_mut().unwrap().remove("usage");assert!(complete(&absent,"deepseek-flash").unwrap()["usage"].is_null());
    }
}

