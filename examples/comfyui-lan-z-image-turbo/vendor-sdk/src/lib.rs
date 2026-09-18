//! Envelope helpers for BigBear vendor adapter plugins (protocol v1 / WIT 1.1.0).
//!
//! Public closed sets come from the crate-local `vendor-adapter-protocol.json`.
//! This crate must not read host policy JSON, `src-tauri`, or UI trees.

#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

const CONTRACT_JSON: &str = include_str!("vendor-adapter-protocol.json");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContractFile {
    protocol_host_major: u32,
    operations: Vec<String>,
    capability_slots: Vec<String>,
    error_codes: Vec<String>,
    task_statuses: Vec<String>,
    output_sources: Vec<String>,
    output_kinds: Vec<String>,
    optional_features: Vec<String>,
}

fn contract() -> ContractFile {
    serde_json::from_str(CONTRACT_JSON).expect("vendor-adapter-protocol.json is part of this crate")
}

pub fn protocol_host_major() -> u32 {
    contract().protocol_host_major
}

pub fn operations() -> Vec<String> {
    contract().operations
}

pub fn capability_slots() -> Vec<String> {
    contract().capability_slots
}

pub fn error_codes() -> Vec<String> {
    contract().error_codes
}

pub fn task_statuses() -> Vec<String> {
    contract().task_statuses
}

pub fn output_sources() -> Vec<String> {
    contract().output_sources
}

pub fn optional_features() -> Vec<String> {
    contract().optional_features
}

/// Host -> guest envelope. `budgetMs` is remaining execution budget, not a wall-clock deadline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invocation {
    pub protocol_version: u32,
    pub operation: String,
    pub profile_id: String,
    pub capability_slot: String,
    pub request_id: String,
    pub budget_ms: u64,
    pub payload_json: String,
    #[serde(default)]
    pub binding_id: String,
    #[serde(default)]
    pub negotiated_version: Option<u32>,
    #[serde(default)]
    pub enabled_features: Vec<String>,
}

impl Invocation {
    pub fn payload<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.payload_json)
    }

    pub fn feature_enabled(&self, feature: &str) -> bool {
        self.enabled_features.iter().any(|f| f == feature)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.protocol_version != protocol_host_major() {
            return Err(format!(
                "protocolVersion {} is not host major {}",
                self.protocol_version,
                protocol_host_major()
            ));
        }
        if !operations().iter().any(|o| o == &self.operation) {
            return Err(format!("unknown operation: {}", self.operation));
        }
        if !capability_slots()
            .iter()
            .any(|s| s == &self.capability_slot)
        {
            return Err(format!("unknown capability slot: {}", self.capability_slot));
        }
        if self.budget_ms == 0 {
            return Err("budgetMs must be > 0".into());
        }
        for feat in &self.enabled_features {
            if !optional_features().iter().any(|f| f == feat) {
                return Err(format!("unknown required feature: {feat}"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default)]
    pub vendor_code: Option<String>,
    #[serde(default)]
    pub vendor_request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub protocol_version: u32,
    pub operation: String,
    #[serde(default)]
    pub data_json: Option<String>,
    #[serde(default)]
    pub error: Option<AdapterError>,
}

impl Response {
    pub fn ok(operation: &str, data: &serde_json::Value) -> Self {
        Self {
            protocol_version: protocol_host_major(),
            operation: operation.to_string(),
            data_json: Some(data.to_string()),
            error: None,
        }
    }

    pub fn err(operation: &str, code: &str, message: &str, retryable: bool) -> Self {
        Self {
            protocol_version: protocol_host_major(),
            operation: operation.to_string(),
            data_json: None,
            error: Some(AdapterError {
                code: code.to_string(),
                message: message.to_string(),
                retryable,
                vendor_code: None,
                vendor_request_id: None,
            }),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.protocol_version != protocol_host_major() {
            return Err(format!(
                "protocolVersion {} is not host major {}",
                self.protocol_version,
                protocol_host_major()
            ));
        }
        if !operations().iter().any(|o| o == &self.operation) {
            return Err(format!("unknown operation: {}", self.operation));
        }
        match (self.data_json.is_some(), self.error.is_some()) {
            (true, false) => {}
            (false, true) => {}
            _ => return Err("response must be either data or error".to_string()),
        }
        if let Some(err) = &self.error {
            if !error_codes().iter().any(|c| c == &err.code) {
                return Err(format!("unknown error code: {}", err.code));
            }
        }
        if let Some(data) = &self.data_json {
            validate_operation_data(&self.operation, data)?;
        }
        Ok(())
    }
}

fn validate_operation_data(operation: &str, data: &str) -> Result<(), String> {
    let v: serde_json::Value =
        serde_json::from_str(data).map_err(|e| format!("dataJson: {e}"))?;
    match operation {
        "submit" => {
            let parsed: SubmitResult =
                serde_json::from_value(v).map_err(|e| format!("submit data: {e}"))?;
            parsed.validate()
        }
        "query" => {
            let parsed: QueryResult =
                serde_json::from_value(v).map_err(|e| format!("query data: {e}"))?;
            parsed.validate()
        }
        "cancel" => {
            let status = v
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if !matches!(status, "unsupported" | "requested" | "confirmed") {
                return Err("cancel data.status must be unsupported|requested|confirmed".into());
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamBinding(String);

impl ParamBinding {
    pub fn new(id: &str) -> Result<Self, String> {
        let t = id.trim();
        if t.is_empty() {
            return Err("ParamBinding refuses empty id".into());
        }
        Ok(Self(t.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RcdParam {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
}

impl RcdParam {
    pub fn fixed(value: serde_json::Value) -> Self {
        Self {
            mode: "fixed".into(),
            value: Some(value),
            min: None,
            max: None,
            step: None,
            default: None,
            options: None,
            binding_id: None,
        }
    }

    pub fn range(min: f64, max: f64, step: f64, default: f64, binding: ParamBinding) -> Self {
        Self {
            mode: "range".into(),
            value: None,
            min: Some(min),
            max: Some(max),
            step: Some(step),
            default: Some(serde_json::Value::from(default)),
            options: None,
            binding_id: Some(binding.as_str().to_string()),
        }
    }

    pub fn enum_of(
        options: Vec<serde_json::Value>,
        default: serde_json::Value,
        binding: ParamBinding,
    ) -> Self {
        Self {
            mode: "enum".into(),
            value: None,
            min: None,
            max: None,
            step: None,
            default: Some(default),
            options: Some(options),
            binding_id: Some(binding.as_str().to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RcdEnumOption {
    pub id: String,
    pub label: String,
    pub value: serde_json::Value,
    pub aspect_ratio: Option<String>,
}

impl RcdEnumOption {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        value: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value,
            aspect_ratio: None,
        }
    }

    pub fn with_aspect(mut self, aspect_ratio: impl Into<String>) -> Self {
        self.aspect_ratio = Some(aspect_ratio.into());
        self
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        m.insert("id".into(), serde_json::Value::String(self.id.clone()));
        m.insert("label".into(), serde_json::Value::String(self.label.clone()));
        m.insert("value".into(), self.value.clone());
        if let Some(ar) = &self.aspect_ratio {
            m.insert("aspectRatio".into(), serde_json::Value::String(ar.clone()));
        }
        serde_json::Value::Object(m)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppliedParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect: Option<serde_json::Value>,
}

impl AppliedParams {
    pub fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

fn ok_serialized(op: &str, value: &impl Serialize) -> Response {
    match serde_json::to_value(value) {
        Ok(v) => Response::ok(op, &v),
        Err(e) => Response::err(
            op,
            "adapterBadOutput",
            &format!("{op} serialize: {e}"),
            false,
        ),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", rename_all_fields = "camelCase")]
pub enum SubmitResult {
    Accepted {
        vendor_task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        applied_params: Option<serde_json::Value>,
    },
    Completed {
        outputs: Vec<Output>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        applied_params: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vendor_task_id: Option<String>,
    },
}

impl SubmitResult {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            SubmitResult::Accepted { vendor_task_id, .. } => {
                if vendor_task_id.trim().is_empty() {
                    return Err("accepted.vendorTaskId must be a real vendor id".into());
                }
                Ok(())
            }
            SubmitResult::Completed { outputs, .. } => {
                if outputs.is_empty() {
                    return Err("completed.outputs must not be empty".into());
                }
                for o in outputs {
                    o.validate()?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub status: String,
    #[serde(default)]
    pub outputs: Vec<Output>,
    #[serde(default)]
    pub progress_text: Option<String>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
}

impl QueryResult {
    pub fn validate(&self) -> Result<(), String> {
        if !task_statuses().iter().any(|s| s == &self.status) {
            return Err(format!("unknown task status: {}", self.status));
        }
        for o in &self.outputs {
            o.validate()?;
        }
        if self.status == "succeeded" && self.outputs.is_empty() {
            return Err("succeeded query must include outputs".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Output {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
    Media {
        media_kind: String,
        source: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
}

impl Output {
    pub fn text(text: impl Into<String>) -> Self {
        Output::Text {
            text: text.into(),
            mime: Some("text/plain".into()),
        }
    }

    pub fn media(media_kind: impl Into<String>, source: impl Into<String>, value: impl Into<String>) -> Self {
        Output::Media {
            media_kind: media_kind.into(),
            source: source.into(),
            value: value.into(),
            mime: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Output::Text { text, .. } => {
                if text.is_empty() {
                    return Err("text output must not be empty".into());
                }
                Ok(())
            }
            Output::Media {
                media_kind,
                source,
                value,
                ..
            } => {
                if value.trim().is_empty() {
                    return Err("media output value must not be empty".into());
                }
                if source == "host-media" && (value.contains('/') || value.contains('\\')) {
                    return Err("host-media value must be a handle, not a filesystem path".into());
                }
                if !output_sources().iter().any(|s| s == source) {
                    return Err(format!("unknown output source: {source}"));
                }
                if media_kind.trim().is_empty() {
                    return Err("mediaKind required".into());
                }
                let _ = contract().output_kinds;
                Ok(())
            }
        }
    }
}

pub fn submit_accepted(vendor_task_id: impl Into<String>) -> Response {
    submit_accepted_with_applied(vendor_task_id, None)
}

pub fn submit_accepted_with_applied(
    vendor_task_id: impl Into<String>,
    applied: Option<AppliedParams>,
) -> Response {
    let applied_params = match applied {
        None => None,
        Some(a) => match a.to_json() {
            Ok(v) => Some(v),
            Err(e) => {
                return Response::err(
                    "submit",
                    "adapterBadOutput",
                    &format!("appliedParams serialize: {e}"),
                    false,
                )
            }
        },
    };
    let result = SubmitResult::Accepted {
        vendor_task_id: vendor_task_id.into(),
        applied_params,
    };
    ok_serialized("submit", &result)
}

pub fn submit_completed(outputs: Vec<Output>) -> Response {
    submit_completed_with_applied(outputs, None)
}

pub fn submit_completed_with_applied(
    outputs: Vec<Output>,
    applied: Option<AppliedParams>,
) -> Response {
    let applied_params = match applied {
        None => None,
        Some(a) => match a.to_json() {
            Ok(v) => Some(v),
            Err(e) => {
                return Response::err(
                    "submit",
                    "adapterBadOutput",
                    &format!("appliedParams serialize: {e}"),
                    false,
                )
            }
        },
    };
    let result = SubmitResult::Completed {
        outputs,
        applied_params,
        vendor_task_id: None,
    };
    ok_serialized("submit", &result)
}

pub fn query_result(result: &QueryResult) -> Response {
    ok_serialized("query", result)
}

pub fn cancel_status(status: &str) -> Response {
    ok_serialized("cancel", &serde_json::json!({ "status": status }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_is_readable_and_closed_sets_are_present() {
        assert_eq!(protocol_host_major(), 1);
        assert!(operations().iter().any(|o| o == "submit"));
        assert!(capability_slots().iter().any(|s| s == "imageToImage"));
        assert!(error_codes().iter().any(|c| c == "adapterInvalidRequest"));
    }

    #[test]
    fn invocation_parses_host_camel_case_wire_json() {
        let raw = r#"{
            "protocolVersion": 1,
            "operation": "submit",
            "profileId": "p1",
            "capabilitySlot": "image",
            "requestId": "task-1:0",
            "budgetMs": 15000,
            "payloadJson": "{\"prompt\":\"hi\"}",
            "bindingId": "bind-1",
            "negotiatedVersion": 1,
            "enabledFeatures": ["partial-outputs"]
        }"#;
        let inv: Invocation = serde_json::from_str(raw).expect("wire json");
        inv.validate().expect("valid envelope");
        let payload: serde_json::Value = inv.payload().expect("payload");
        assert_eq!(payload["prompt"], "hi");
        assert!(inv.feature_enabled("partial-outputs"));
        assert_eq!(inv.binding_id, "bind-1");
        assert_eq!(inv.negotiated_version, Some(1));
    }

    #[test]
    fn invocation_rejects_unknown_operation() {
        let mut inv: Invocation = serde_json::from_str(
            r#"{"protocolVersion":1,"operation":"submit","profileId":"p","capabilitySlot":"image","requestId":"r","budgetMs":1,"payloadJson":"{}"}"#,
        )
        .expect("wire json");
        inv.operation = "teleport".into();
        assert!(inv.validate().is_err());
    }

    #[test]
    fn submit_accepted_emits_vendor_task_id() {
        let resp = submit_accepted("t-42");
        resp.validate().expect("submit accepted");
        let data: serde_json::Value =
            serde_json::from_str(resp.data_json.as_deref().expect("data")).expect("data json");
        assert_eq!(data["kind"], "accepted");
        assert_eq!(data["vendorTaskId"], "t-42");
    }

    #[test]
    fn submit_completed_text_is_consumable() {
        let resp = submit_completed(vec![Output::text("hello")]);
        resp.validate().expect("text completed");
        let data: serde_json::Value =
            serde_json::from_str(resp.data_json.as_deref().expect("data")).expect("data json");
        assert_eq!(data["kind"], "completed");
        assert_eq!(data["outputs"][0]["kind"], "text");
        assert_eq!(data["outputs"][0]["text"], "hello");
        assert!(data["outputs"][0].get("value").is_none());
    }

    #[test]
    fn response_rejects_unknown_operation_success() {
        let resp = Response::ok("not-an-operation", &serde_json::json!({"kind":"completed","outputs":[]}));
        assert!(resp.validate().is_err());
    }

    #[test]
    fn param_binding_rejects_empty() {
        assert!(ParamBinding::new("  ").is_err());
        assert!(ParamBinding::new("node-6").is_ok());
        let p = RcdParam::range(1.0, 15.0, 1.0, 5.0, ParamBinding::new("dur").unwrap());
        assert_eq!(p.binding_id.as_deref(), Some("dur"));
    }

    #[test]
    fn enum_option_with_aspect_is_camel_case() {
        let v = RcdEnumOption::new("2k-16-9", "2K 16:9", serde_json::json!("1920x1088"))
            .with_aspect("21:9")
            .to_json();
        assert_eq!(v["id"], "2k-16-9");
        assert_eq!(v["aspectRatio"], "21:9");
        assert!(v.get("aspect_ratio").is_none());
    }

    #[test]
    fn submit_accepted_with_applied_is_camel_case() {
        let mut applied = AppliedParams::default();
        applied.duration = Some(serde_json::json!(5));
        let resp = submit_accepted_with_applied("t-1", Some(applied));
        let data: serde_json::Value =
            serde_json::from_str(resp.data_json.as_deref().expect("data")).expect("data json");
        assert_eq!(data["appliedParams"]["duration"], 5);
    }

    #[test]
    fn error_response_has_no_data() {
        let resp = Response::err("submit", "adapterVendorHttpError", "upstream 500", true);
        resp.validate().expect("mutually exclusive");
        assert!(resp.data_json.is_none());
    }

    #[test]
    fn protocol_json_has_no_host_policy_keys() {
        assert!(!CONTRACT_JSON.contains("trustPullPath"));
        assert!(!CONTRACT_JSON.contains("providerKinds"));
        assert!(!CONTRACT_JSON.contains("languageTiers"));
    }
}
