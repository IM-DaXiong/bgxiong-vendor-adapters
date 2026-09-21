//! Envelope helpers for BigBear vendor adapter plugins (protocol v1 / contract 1.2.0 / WIT 1.1.0).
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
    #[serde(default)]
    terminal_statuses: Vec<String>,
    #[serde(default)]
    terminal_failure_categories: Vec<String>,
    #[serde(default)]
    terminal_failure_message_max_chars: u64,
    #[serde(default)]
    terminal_failure_details_max_bytes: u64,
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

pub fn terminal_statuses() -> Vec<String> {
    let c = contract();
    if c.terminal_statuses.is_empty() {
        return vec!["failed".into(), "cancelled".into(), "expired".into()];
    }
    c.terminal_statuses
}

pub fn terminal_failure_categories() -> Vec<String> {
    let c = contract();
    if c.terminal_failure_categories.is_empty() {
        return vec![
            "vendorTaskFailed".into(),
            "vendorTaskCancelled".into(),
            "vendorTaskExpired".into(),
        ];
    }
    c.terminal_failure_categories
}

pub fn terminal_failure_message_max_chars() -> usize {
    let n = contract().terminal_failure_message_max_chars;
    if n == 0 {
        2000
    } else {
        n as usize
    }
}

pub fn terminal_failure_details_max_bytes() -> usize {
    let n = contract().terminal_failure_details_max_bytes;
    if n == 0 {
        4096
    } else {
        n as usize
    }
}

pub const ADAPTER_QUERY_TERMINAL_FAILURE_MISSING: &str = "ADAPTER_QUERY_TERMINAL_FAILURE_MISSING";
pub const ADAPTER_QUERY_TERMINAL_FAILURE_FORBIDDEN: &str =
    "ADAPTER_QUERY_TERMINAL_FAILURE_FORBIDDEN";
pub const ADAPTER_QUERY_TERMINAL_FAILURE_DETAILS_TOO_LARGE: &str =
    "ADAPTER_QUERY_TERMINAL_FAILURE_DETAILS_TOO_LARGE";
pub const ADAPTER_QUERY_PROGRESS_ON_TERMINAL: &str = "ADAPTER_QUERY_PROGRESS_ON_TERMINAL";

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
    let v: serde_json::Value = serde_json::from_str(data).map_err(|e| format!("dataJson: {e}"))?;
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
            let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("");
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
    pub fn new(id: impl Into<String>, label: impl Into<String>, value: serde_json::Value) -> Self {
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
        m.insert(
            "label".into(),
            serde_json::Value::String(self.label.clone()),
        );
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
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalFailure {
    pub category: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl TerminalFailure {
    pub fn vendor_task_failed(
        vendor_code: Option<String>,
        message: impl Into<String>,
        details: Option<serde_json::Value>,
    ) -> Result<Self, String> {
        Self::new("vendorTaskFailed", vendor_code, message, details, false)
    }

    pub fn new(
        category: impl Into<String>,
        vendor_code: Option<String>,
        message: impl Into<String>,
        details: Option<serde_json::Value>,
        retryable: bool,
    ) -> Result<Self, String> {
        let failure = Self {
            category: category.into(),
            message: message.into(),
            retryable,
            vendor_code,
            vendor_request_id: None,
            http_status: None,
            details: details.map(sanitize_terminal_details),
        };
        failure.validate()?;
        Ok(failure)
    }

    pub fn from_vendor_reason(
        category: &str,
        vendor_code: Option<String>,
        reason: &serde_json::Value,
    ) -> Result<Self, String> {
        let (message, details) = message_and_details_from_reason(reason);
        Self::new(category, vendor_code, message, details, false)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !terminal_failure_categories()
            .iter()
            .any(|c| c == &self.category)
        {
            return Err(format!("unknown terminalFailure.category: {}", self.category));
        }
        let message = self.message.trim();
        if message.is_empty() {
            return Err(format!("{ADAPTER_QUERY_TERMINAL_FAILURE_MISSING}: message"));
        }
        if message.chars().count() > terminal_failure_message_max_chars() {
            return Err("ADAPTER_QUERY_TERMINAL_FAILURE_MESSAGE_TOO_LARGE".into());
        }
        if let Some(details) = &self.details {
            let bytes = serde_json::to_vec(details)
                .map_err(|e| format!("terminalFailure.details: {e}"))?;
            if bytes.len() > terminal_failure_details_max_bytes() {
                return Err(ADAPTER_QUERY_TERMINAL_FAILURE_DETAILS_TOO_LARGE.into());
            }
        }
        Ok(())
    }
}

fn message_and_details_from_reason(reason: &serde_json::Value) -> (String, Option<serde_json::Value>) {
    match reason {
        serde_json::Value::String(s) => (s.trim().to_string(), None),
        serde_json::Value::Object(map) => {
            let message = map
                .get("exception_message")
                .or_else(|| map.get("exceptionMessage"))
                .or_else(|| map.get("msg"))
                .or_else(|| map.get("message"))
                .or_else(|| map.get("error"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "vendor task failed".into());
            (message, Some(sanitize_terminal_details(reason.clone())))
        }
        other => (
            "vendor task failed".into(),
            Some(sanitize_terminal_details(other.clone())),
        ),
    }
}

fn sanitize_terminal_details(value: serde_json::Value) -> serde_json::Value {
    sanitize_value(value, 0)
}

fn is_sensitive_detail_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("apikey")
        || lower.contains("api_key")
        || lower.contains("authorization")
        || lower == "token"
        || lower.ends_with("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower == "prompt"
        || lower.contains("cookie")
        || lower.contains("signature")
        || lower.contains("accesskey")
        || lower.contains("access_key")
}

fn sanitize_value(value: serde_json::Value, depth: usize) -> serde_json::Value {
    if depth > 6 {
        return serde_json::Value::String("[truncated]".into());
    }
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if is_sensitive_detail_key(&k) {
                    continue;
                }
                out.insert(k, sanitize_value(v, depth + 1));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .take(32)
                .map(|v| sanitize_value(v, depth + 1))
                .collect(),
        ),
        serde_json::Value::String(s) => {
            if s.chars().count() > 500 {
                let head: String = s.chars().take(500).collect();
                serde_json::Value::String(head)
            } else {
                serde_json::Value::String(s)
            }
        }
        other => other,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_failure: Option<TerminalFailure>,
}

impl QueryResult {
    pub fn in_progress(status: impl Into<String>, progress_text: Option<String>) -> Self {
        Self {
            status: status.into(),
            outputs: Vec::new(),
            progress_text,
            retry_after_ms: None,
            terminal_failure: None,
        }
    }

    pub fn succeeded(outputs: Vec<Output>) -> Self {
        Self {
            status: "succeeded".into(),
            outputs,
            progress_text: None,
            retry_after_ms: None,
            terminal_failure: None,
        }
    }

    pub fn terminal(status: impl Into<String>, failure: TerminalFailure) -> Self {
        Self {
            status: status.into(),
            outputs: Vec::new(),
            progress_text: None,
            retry_after_ms: None,
            terminal_failure: Some(failure),
        }
    }

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
        let terminal = terminal_statuses().iter().any(|s| s == &self.status);
        match (terminal, self.terminal_failure.as_ref()) {
            (true, None) => {
                return Err(format!(
                    "{ADAPTER_QUERY_TERMINAL_FAILURE_MISSING}: status={}",
                    self.status
                ));
            }
            (true, Some(failure)) => failure.validate()?,
            (false, Some(_)) => {
                return Err(format!(
                    "{ADAPTER_QUERY_TERMINAL_FAILURE_FORBIDDEN}: status={}",
                    self.status
                ));
            }
            (false, None) => {}
        }
        if terminal
            && self
                .progress_text
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some()
        {
            return Err(format!(
                "{ADAPTER_QUERY_PROGRESS_ON_TERMINAL}: progressText is running-only"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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

    pub fn media(
        media_kind: impl Into<String>,
        source: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
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
    if let Err(e) = result.validate() {
        return Response::err("query", "adapterBadOutput", &e, false);
    }
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
        let resp = Response::ok(
            "not-an-operation",
            &serde_json::json!({"kind":"completed","outputs":[]}),
        );
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

    #[test]
    fn query_terminal_fixtures_are_deterministic() {
        let cancelled: QueryResult = serde_json::from_value(serde_json::json!({
            "status": "cancelled",
            "outputs": [],
            "terminalFailure": {
                "category": "vendorTaskCancelled",
                "message": "remote task cancelled",
                "retryable": false,
                "vendorCode": "CANCELLED"
            }
        }))
        .expect("cancelled");
        cancelled.validate().expect("cancelled ok");
        assert_eq!(
            cancelled.terminal_failure.as_ref().unwrap().category,
            "vendorTaskCancelled"
        );

        let expired: QueryResult = serde_json::from_value(serde_json::json!({
            "status": "expired",
            "outputs": [],
            "terminalFailure": {
                "category": "vendorTaskExpired",
                "message": "remote task expired",
                "retryable": false,
                "vendorCode": "EXPIRED"
            }
        }))
        .expect("expired");
        expired.validate().expect("expired ok");

        let http200: QueryResult = serde_json::from_value(serde_json::json!({
            "status": "failed",
            "outputs": [],
            "terminalFailure": {
                "category": "vendorTaskFailed",
                "message": "business failed after HTTP 200",
                "retryable": false,
                "vendorCode": "805",
                "httpStatus": 200,
                "details": { "code": 805 }
            }
        }))
        .expect("http200");
        http200.validate().expect("http200 ok");
        assert_eq!(http200.terminal_failure.as_ref().unwrap().http_status, Some(200));
        assert_eq!(
            http200.terminal_failure.as_ref().unwrap().vendor_code.as_deref(),
            Some("805")
        );

        let missing: QueryResult = serde_json::from_value(serde_json::json!({
            "status": "failed",
            "outputs": [],
            "progressText": "failed"
        }))
        .expect("missing");
        let err = missing.validate().expect_err("bare failed is illegal");
        assert!(err.contains(ADAPTER_QUERY_TERMINAL_FAILURE_MISSING));

        let resp = query_result(&missing);
        assert!(resp.data_json.is_none());
        assert_eq!(
            resp.error.as_ref().map(|e| e.code.as_str()),
            Some("adapterBadOutput")
        );

        let huge = "x".repeat(5000);
        let oversized = TerminalFailure {
            category: "vendorTaskFailed".into(),
            message: "overflow".into(),
            retryable: false,
            vendor_code: None,
            vendor_request_id: None,
            http_status: None,
            details: Some(serde_json::json!({ "blob": huge })),
        };
        assert!(oversized
            .validate()
            .unwrap_err()
            .contains(ADAPTER_QUERY_TERMINAL_FAILURE_DETAILS_TOO_LARGE));
    }

    #[test]
    fn running_query_rejects_terminal_failure() {
        let q = QueryResult {
            status: "running".into(),
            outputs: Vec::new(),
            progress_text: Some("30%".into()),
            retry_after_ms: None,
            terminal_failure: Some(
                TerminalFailure::vendor_task_failed(None, "no", None).unwrap(),
            ),
        };
        assert!(q
            .validate()
            .unwrap_err()
            .contains(ADAPTER_QUERY_TERMINAL_FAILURE_FORBIDDEN));
    }

    #[test]
    fn terminal_failure_strips_sensitive_keys() {
        let tf = TerminalFailure::from_vendor_reason(
            "vendorTaskFailed",
            Some("FAILED".into()),
            &serde_json::json!({
                "exception_message": "node rejected",
                "apiKey": "secret-key",
                "prompt": "user prompt"
            }),
        )
        .unwrap();
        assert_eq!(tf.message, "node rejected");
        let details = tf.details.unwrap();
        assert!(details.get("apiKey").is_none());
        assert!(details.get("prompt").is_none());
        assert_eq!(details["exception_message"], "node rejected");
    }
}

/// Feature id for handle-based video/keyframe reference media.
pub const FEATURE_VIDEO_REFERENCE_MEDIA_V1: &str = "video-reference-media-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaInputV1 {
    pub id: String,
    pub kind: String,
    pub handle: String,
    pub mime: String,
    pub file_name: String,
    pub byte_length: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub association_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoMediaSubmitV1 {
    pub model: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_frame: Option<MediaInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_frame: Option<MediaInputV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_images: Option<Vec<MediaInputV1>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_videos: Option<Vec<MediaInputV1>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_audios: Option<Vec<MediaInputV1>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_parameters: Option<serde_json::Value>,
}

fn validate_media_input(item: &MediaInputV1, expected_kind: Option<&str>) -> Result<(), String> {
    if item.id.is_empty() {
        return Err("MediaInputV1.id must be non-empty".into());
    }
    if item.handle.is_empty() {
        return Err(format!("MediaInputV1.handle empty for id {}", item.id));
    }
    if item.mime.is_empty() || item.file_name.is_empty() {
        return Err(format!(
            "MediaInputV1.mime/fileName required for id {}",
            item.id
        ));
    }
    if item.byte_length.is_empty() || !item.byte_length.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "MediaInputV1.byteLength must be decimal u64 string for id {}",
            item.id
        ));
    }
    if item.byte_length.len() > 1 && item.byte_length.starts_with('0') {
        return Err(format!(
            "MediaInputV1.byteLength must not contain a leading zero for id {}",
            item.id
        ));
    }
    if item.byte_length.parse::<u64>().is_err() {
        return Err(format!(
            "MediaInputV1.byteLength exceeds u64 for id {}",
            item.id
        ));
    }
    if item.file_name == "."
        || item.file_name == ".."
        || item.file_name.contains('/')
        || item.file_name.contains('\\')
        || item.file_name.contains("..")
    {
        return Err(format!(
            "MediaInputV1.fileName must be a safe basename for id {}",
            item.id
        ));
    }
    match item.kind.as_str() {
        "image" | "video" | "audio" => {}
        other => return Err(format!("MediaInputV1.kind invalid: {other}")),
    }
    if let Some(exp) = expected_kind {
        if item.kind != exp {
            return Err(format!(
                "MediaInputV1.kind {} does not match array {} for id {}",
                item.kind, exp, item.id
            ));
        }
    }
    Ok(())
}

/// Strict decode for feature video-reference-media-v1 submit payloads.
/// Rejects null elements, empty handles, kind mismatches, duplicate ids, and invalid byteLength.
pub fn decode_video_media_submit_v1(json: &str) -> Result<VideoMediaSubmitV1, String> {
    let v: VideoMediaSubmitV1 =
        serde_json::from_str(json).map_err(|e| format!("adapterInvalidRequest: {e}"))?;
    if v.model.is_empty() {
        return Err("adapterInvalidRequest: model required".into());
    }
    let mut ids = alloc::collections::BTreeSet::new();
    {
        let mut push = |item: &MediaInputV1, kind: &str| -> Result<(), String> {
            validate_media_input(item, Some(kind))?;
            if !ids.insert(item.id.clone()) {
                return Err(format!(
                    "adapterInvalidRequest: duplicate media id {}",
                    item.id
                ));
            }
            Ok(())
        };
        if let Some(ref sf) = v.start_frame {
            push(sf, "image")?;
        }
        if let Some(ref ef) = v.end_frame {
            push(ef, "image")?;
        }
        if let Some(ref items) = v.reference_images {
            for item in items {
                push(item, "image")?;
            }
        }
        if let Some(ref items) = v.reference_videos {
            for item in items {
                push(item, "video")?;
            }
        }
        if let Some(ref items) = v.reference_audios {
            for item in items {
                push(item, "audio")?;
            }
        }
    }
    if let Some(ref pp) = v.plugin_parameters {
        if !pp.is_object() {
            return Err("adapterInvalidRequest: pluginParameters must be object".into());
        }
        for k in [
            "prompt",
            "model",
            "startFrame",
            "endFrame",
            "referenceImages",
            "referenceVideos",
            "referenceAudios",
        ] {
            if pp.get(k).is_some() {
                return Err(format!(
                    "adapterInvalidRequest: pluginParameters reserved key {k}"
                ));
            }
        }
    }
    Ok(v)
}

#[cfg(test)]
mod video_media_tests {
    use super::*;

    #[test]
    fn decodes_mixed_fixture_shape() {
        let json = r#"{
          "model":"m","prompt":"p",
          "referenceVideos":[{"id":"v1","kind":"video","handle":"h","mime":"video/mp4","fileName":"a.mp4","byteLength":"12"}],
          "referenceAudios":[{"id":"a1","kind":"audio","handle":"h2","mime":"audio/wav","fileName":"a.wav","byteLength":"4"}]
        }"#;
        let v = decode_video_media_submit_v1(json).expect("ok");
        assert_eq!(v.reference_videos.as_ref().unwrap()[0].id, "v1");
    }

    #[test]
    fn rejects_kind_mismatch() {
        let json = r#"{"model":"m","prompt":"p","referenceVideos":[{"id":"v1","kind":"audio","handle":"h","mime":"audio/wav","fileName":"a.wav","byteLength":"1"}]}"#;
        assert!(decode_video_media_submit_v1(json).is_err());
    }

    #[test]
    fn rejects_duplicate_id() {
        let json = r#"{"model":"m","prompt":"p","referenceImages":[
          {"id":"dup","kind":"image","handle":"h1","mime":"image/png","fileName":"a.png","byteLength":"1"},
          {"id":"dup","kind":"image","handle":"h2","mime":"image/png","fileName":"b.png","byteLength":"1"}
        ]}"#;
        assert!(decode_video_media_submit_v1(json).is_err());
    }

    #[test]
    fn rejects_contract_boundary_values() {
        for (file_name, byte_length) in [
            ("a.mp4", "01"),
            ("a.mp4", "18446744073709551616"),
            (".", "1"),
            ("..", "1"),
        ] {
            let json = format!(
                r#"{{"model":"m","prompt":"p","referenceVideos":[{{"id":"v1","kind":"video","handle":"h","mime":"video/mp4","fileName":"{file_name}","byteLength":"{byte_length}"}}]}}"#
            );
            assert!(
                decode_video_media_submit_v1(&json).is_err(),
                "boundary input must fail: fileName={file_name}, byteLength={byte_length}"
            );
        }
    }

    #[test]
    fn shared_contract_fixtures_match_sdk_decode() {
        let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .map(|dir| dir.join("fixtures/video-reference-media-v1"))
            .find(|dir| dir.join("INDEX.json").is_file())
            .expect("shared video-reference-media-v1 fixtures");
        let index: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(fixture_dir.join("INDEX.json")).expect("fixture INDEX"),
        )
        .expect("fixture INDEX json");
        for (name, meta) in index["files"].as_object().expect("fixture files") {
            let json = std::fs::read_to_string(fixture_dir.join(name))
                .unwrap_or_else(|error| panic!("read fixture {name}: {error}"));
            let decoded = decode_video_media_submit_v1(&json);
            match meta["expect"].as_str().expect("fixture expect") {
                "pass" => assert!(decoded.is_ok(), "{name} should pass: {decoded:?}"),
                "fail" => assert!(decoded.is_err(), "{name} should fail"),
                other => panic!("unknown fixture expectation {other} for {name}"),
            }
        }
    }
}
