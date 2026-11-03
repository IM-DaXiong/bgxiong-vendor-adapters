//! Negotiated media-target-selection-v1. Schema + receipt integrity; no I/O.
use crate::{ContractError, SchemaRegistry};
use serde_json::Value;
use std::collections::BTreeSet;

pub const FEATURE: &str = "media-target-selection-v1";

const SOURCE: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/media-target-selection-v1/source-identity.schema.json";
const ANALYSIS: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/media-target-selection-v1/analysis-receipt.schema.json";
const SELECTION: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/media-target-selection-v1/selection.schema.json";
const TRACKING: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/media-target-selection-v1/tracking-receipt.schema.json";
const FREEZE: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/media-target-selection-v1/freeze.schema.json";
const REFINE_OUT: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/media-target-selection-v1/refine.output.schema.json";

fn invalid(path: &str, reason: &str) -> ContractError {
    ContractError::new("INVALID_INPUT", path, reason)
}

fn public() -> Result<SchemaRegistry, ContractError> {
    SchemaRegistry::public()
}

pub fn validate_source(value: &Value) -> Result<(), ContractError> {
    public()?.validate(SOURCE, value)
}

pub fn validate_analysis(value: &Value) -> Result<(), ContractError> {
    public()?.validate(ANALYSIS, value)?;
    let page = &value["page"];
    if page["complete"] == false && page["nextCursor"].is_null() {
        return Err(invalid("/page/nextCursor", "incomplete page requires nextCursor"));
    }
    let w = value["source"]["displayGeometry"]["widthPx"].as_u64().unwrap_or(0);
    let h = value["source"]["displayGeometry"]["heightPx"].as_u64().unwrap_or(0);
    for (si, shot) in value["shots"].as_array().unwrap().iter().enumerate() {
        for (ci, cand) in shot["candidates"].as_array().unwrap().iter().enumerate() {
            let boxv = &cand["box"];
            let x = boxv["x"].as_f64().unwrap_or(-1.0);
            let y = boxv["y"].as_f64().unwrap_or(-1.0);
            let bw = boxv["width"].as_f64().unwrap_or(0.0);
            let bh = boxv["height"].as_f64().unwrap_or(0.0);
            if x < 0.0 || y < 0.0 || x + bw > w as f64 + 1e-6 || y + bh > h as f64 + 1e-6 {
                return Err(invalid(
                    &format!("/shots/{si}/candidates/{ci}/box"),
                    "candidate box outside display geometry",
                ));
            }
        }
    }
    Ok(())
}

pub fn validate_selection(value: &Value, analysis: Option<&Value>) -> Result<(), ContractError> {
    public()?.validate(SELECTION, value)?;
    if let Some(analysis) = analysis {
        if value["analysisId"] != analysis["analysisId"]
            || value["analysisRevision"] != analysis["revision"]
            || value["analysisDigest"] != analysis["digest"]
        {
            return Err(invalid("/selection", "selection must bind current analysis revision"));
        }
        let known: BTreeSet<(String, String)> = analysis["shots"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| {
                let shot = s["shotId"].as_str().unwrap().to_owned();
                s["candidates"].as_array().unwrap().iter().map(move |c| {
                    (shot.clone(), c["candidateId"].as_str().unwrap().to_owned())
                })
            })
            .collect();
        for (i, shot) in value["shots"].as_array().unwrap().iter().enumerate() {
            let choice = &shot["choice"];
            if choice["kind"] == "target" {
                let key = (
                    shot["shotId"].as_str().unwrap().to_owned(),
                    choice["candidateId"].as_str().unwrap().to_owned(),
                );
                if !known.contains(&key) {
                    return Err(invalid(
                        &format!("/shots/{i}/choice/candidateId"),
                        "candidateId not in analysis revision/shot",
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn validate_tracking(value: &Value) -> Result<(), ContractError> {
    public()?.validate(TRACKING, value)
}

pub fn validate_freeze(value: &Value) -> Result<(), ContractError> {
    public()?.validate(FREEZE, value)
}

pub fn validate_refine_output(value: &Value) -> Result<(), ContractError> {
    public()?.validate(REFINE_OUT, value)
}

/// Host publishes this feature only after consuming analysis/selection/tracking/freeze.
pub fn feature_id() -> &'static str {
    FEATURE
}
