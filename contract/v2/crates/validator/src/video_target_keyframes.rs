//! Negotiated video-target-keyframes-v1. Schema + digest/geometry integrity; no I/O.
use crate::{ContractError, SchemaRegistry};
use serde_json::Value;
use std::collections::BTreeSet;

pub const FEATURE: &str = "video-target-keyframes-v1";

const DECLARATION: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/declaration.schema.json";
const SELECTION: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/selection.schema.json";
const TRACKING_INPUT: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/tracking.input.schema.json";
const PROGRESS: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/progress.schema.json";
const RECEIPT: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/tracking-receipt.schema.json";
const FRAME_PAGE: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/frame-page.schema.json";
const FRAME_INDEX: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/frame-index.schema.json";
const FREEZE: &str = "https://bgxiong.local/vendor-adapter/v2/profiles/video-target-keyframes-v1/generation-freeze.schema.json";

fn invalid(path: &str, reason: &str) -> ContractError {
    ContractError::new("INVALID_INPUT", path, reason)
}

fn public() -> Result<SchemaRegistry, ContractError> {
    SchemaRegistry::public()
}

fn range_ok(range: &Value) -> bool {
    let start = range["startMs"].as_u64().unwrap_or(u64::MAX);
    let end = range["endMs"].as_u64().unwrap_or(0);
    start <= end
}

fn validate_ranges(path: &str, ranges: &Value) -> Result<(), ContractError> {
    for (i, range) in ranges.as_array().unwrap_or(&Vec::new()).iter().enumerate() {
        if !range_ok(range) {
            return Err(invalid(&format!("{path}/{i}"), "range startMs must be <= endMs"));
        }
    }
    Ok(())
}

pub fn validate_declaration(value: &Value) -> Result<(), ContractError> {
    public()?.validate(DECLARATION, value)
}

pub fn validate_selection(value: &Value) -> Result<(), ContractError> {
    public()?.validate(SELECTION, value)?;
    let w = value["frameGeometry"]["width"].as_u64().unwrap_or(0) as f64;
    let h = value["frameGeometry"]["height"].as_u64().unwrap_or(0) as f64;
    let mut keyframe_ids = BTreeSet::new();
    for (ki, keyframe) in value["keyframes"].as_array().unwrap().iter().enumerate() {
        let kid = keyframe["keyframeId"].as_str().unwrap();
        if !keyframe_ids.insert(kid.to_owned()) {
            return Err(invalid(
                &format!("/keyframes/{ki}/keyframeId"),
                "keyframeId must be unique within selection",
            ));
        }
        let points = keyframe["points"].as_array().unwrap();
        let has_box = keyframe.get("box").is_some_and(|b| !b.is_null());
        if points.is_empty() && !has_box {
            return Err(invalid(
                &format!("/keyframes/{ki}"),
                "keyframe requires at least one point or a box",
            ));
        }
        let mut point_ids = BTreeSet::new();
        for (pi, point) in points.iter().enumerate() {
            let pid = point["pointId"].as_str().unwrap();
            if !point_ids.insert(pid.to_owned()) {
                return Err(invalid(
                    &format!("/keyframes/{ki}/points/{pi}/pointId"),
                    "pointId must be unique within keyframe",
                ));
            }
            let x = point["x"].as_f64().unwrap_or(-1.0);
            let y = point["y"].as_f64().unwrap_or(-1.0);
            if x < 0.0 || y < 0.0 || x > w + 1e-6 || y > h + 1e-6 {
                return Err(invalid(
                    &format!("/keyframes/{ki}/points/{pi}"),
                    "point outside frameGeometry",
                ));
            }
        }
        if has_box {
            let boxv = &keyframe["box"];
            let x = boxv["x"].as_f64().unwrap_or(-1.0);
            let y = boxv["y"].as_f64().unwrap_or(-1.0);
            let bw = boxv["width"].as_f64().unwrap_or(0.0);
            let bh = boxv["height"].as_f64().unwrap_or(0.0);
            if x < 0.0 || y < 0.0 || x + bw > w + 1e-6 || y + bh > h + 1e-6 {
                return Err(invalid(
                    &format!("/keyframes/{ki}/box"),
                    "box outside frameGeometry",
                ));
            }
        }
    }
    Ok(())
}

pub fn validate_tracking_input(value: &Value) -> Result<(), ContractError> {
    public()?.validate(TRACKING_INPUT, value)?;
    validate_selection(&value["selection"])?;
    if !range_ok(&value["range"]) {
        return Err(invalid("/range", "range startMs must be <= endMs"));
    }
    if !range_ok(&value["requestedCoverage"]) {
        return Err(invalid("/requestedCoverage", "coverage startMs must be <= endMs"));
    }
    let source = &value["sourceIdentity"];
    let sel_source = &value["selection"]["sourceIdentity"];
    if source != sel_source {
        return Err(invalid(
            "/sourceIdentity",
            "tracking sourceIdentity must equal selection.sourceIdentity",
        ));
    }
    if &value["range"] != &source["selectedRange"] {
        return Err(invalid(
            "/range",
            "range must equal sourceIdentity.selectedRange",
        ));
    }
    let range_start = value["range"]["startMs"].as_u64().unwrap_or(0);
    let range_end = value["range"]["endMs"].as_u64().unwrap_or(0);
    let cover_start = value["requestedCoverage"]["startMs"].as_u64().unwrap_or(u64::MAX);
    let cover_end = value["requestedCoverage"]["endMs"].as_u64().unwrap_or(0);
    if cover_start < range_start || cover_end > range_end {
        return Err(invalid(
            "/requestedCoverage",
            "requestedCoverage must stay inside range",
        ));
    }
    let display = &source["displayGeometry"];
    let frame = &value["selection"]["frameGeometry"];
    if display["widthPx"] != frame["width"] || display["heightPx"] != frame["height"] {
        return Err(invalid(
            "/sourceIdentity/displayGeometry",
            "displayGeometry size must match selection.frameGeometry",
        ));
    }
    for (i, keyframe) in value["selection"]["keyframes"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let time = keyframe["timeMs"].as_u64().unwrap_or(u64::MAX);
        if time < range_start || time >= range_end {
            return Err(invalid(
                &format!("/selection/keyframes/{i}/timeMs"),
                "keyframe timeMs must be inside range",
            ));
        }
    }
    let anchor = value["anchorKeyframeId"].as_str().unwrap();
    let known: BTreeSet<_> = value["selection"]["keyframes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|k| k["keyframeId"].as_str().map(str::to_owned))
        .collect();
    if !known.contains(anchor) {
        return Err(invalid(
            "/anchorKeyframeId",
            "anchorKeyframeId not present in selection.keyframes",
        ));
    }
    Ok(())
}

pub fn validate_progress(value: &Value) -> Result<(), ContractError> {
    public()?.validate(PROGRESS, value)?;
    validate_ranges("/coveredRanges", &value["coveredRanges"])?;
    let total = value["totalFrames"].as_u64();
    let processed = value["processedFrames"].as_u64();
    if let (Some(t), Some(p)) = (total, processed) {
        if p > t {
            return Err(invalid(
                "/processedFrames",
                "processedFrames cannot exceed totalFrames",
            ));
        }
    }
    Ok(())
}

pub fn validate_tracking_receipt(value: &Value) -> Result<(), ContractError> {
    public()?.validate(RECEIPT, value)?;
    validate_ranges("/coveredRanges", &value["coveredRanges"])?;
    validate_ranges("/lostRanges", &value["lostRanges"])?;
    validate_ranges("/uncomputedRanges", &value["uncomputedRanges"])?;
    validate_ranges("/skipRanges", &value["skipRanges"])?;
    if !range_ok(&value["range"]) {
        return Err(invalid("/range", "range startMs must be <= endMs"));
    }
    let mut resolved_ids = BTreeSet::new();
    for (i, item) in value["resolvedKeyframes"].as_array().unwrap().iter().enumerate() {
        let id = item["keyframeId"].as_str().unwrap();
        if !resolved_ids.insert(id.to_owned()) {
            return Err(invalid(
                &format!("/resolvedKeyframes/{i}/keyframeId"),
                "resolved keyframeId must be unique",
            ));
        }
    }
    let ready = value["ready"].as_bool().unwrap_or(false);
    let covered = value["coveredRanges"].as_array().unwrap();
    let mask = &value["maskArtifact"];
    if ready {
        let uncomputed = value["uncomputedRanges"].as_array();
        if uncomputed.is_some_and(|ranges| !ranges.is_empty()) {
            return Err(invalid(
                "/ready",
                "ready=true rejects nonempty uncomputedRanges",
            ));
        }
        if covered.is_empty() {
            return Err(invalid(
                "/ready",
                "ready=true requires nonempty coveredRanges",
            ));
        }
        if mask["artifactId"].as_str().unwrap_or("").is_empty()
            || mask["digest"].as_str().unwrap_or("").is_empty()
        {
            return Err(invalid(
                "/maskArtifact",
                "ready=true requires opaque maskArtifact identity and digest",
            ));
        }
        let preview = value.get("previewMediaRef");
        let has_preview = preview.is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()));
        if !has_preview && mask.get("pageIndexRef").and_then(Value::as_str).is_none() {
            // Readable mask/preview resource required when ready.
            // pageIndexRef on maskArtifact or previewMediaRef counts.
            if value.get("framePageIndexRef").and_then(Value::as_str).is_none() {
                return Err(invalid(
                    "/ready",
                    "ready=true requires previewMediaRef or frame page index reference",
                ));
            }
        }
    }
    Ok(())
}

pub fn validate_frame_page(value: &Value) -> Result<(), ContractError> {
    public()?.validate(FRAME_PAGE, value)?;
    let page = &value["page"];
    if page["complete"] == false && page["nextCursor"].is_null() {
        return Err(invalid(
            "/page/nextCursor",
            "incomplete page requires nextCursor",
        ));
    }
    let mut indexes = BTreeSet::new();
    for (i, frame) in value["frames"].as_array().unwrap().iter().enumerate() {
        let idx = frame["frameIndex"].as_u64().unwrap();
        if !indexes.insert(idx) {
            return Err(invalid(
                &format!("/frames/{i}/frameIndex"),
                "frameIndex must be unique within page",
            ));
        }
        let state = frame["state"].as_str().unwrap();
        if state == "covered" {
            let has_mask = frame
                .get("maskPageRef")
                .is_some_and(|m| m.is_object());
            if !has_mask {
                return Err(invalid(
                    &format!("/frames/{i}/maskPageRef"),
                    "covered frame requires maskPageRef",
                ));
            }
        }
    }
    Ok(())
}

pub fn validate_frame_index(value: &Value) -> Result<(), ContractError> {
    public()?.validate(FRAME_INDEX, value)?;
    let pages = value["pages"].as_array().unwrap();
    let mut cursors = BTreeSet::new();
    for (i, page) in pages.iter().enumerate() {
        let cursor = page["cursor"].as_str().unwrap();
        if !cursors.insert(cursor.to_owned()) {
            return Err(invalid(
                &format!("/pages/{i}/cursor"),
                "page cursor must be unique",
            ));
        }
        let start = page["startMs"].as_u64().unwrap_or(u64::MAX);
        let end = page["endMs"].as_u64().unwrap_or(0);
        if start >= end {
            return Err(invalid(
                &format!("/pages/{i}"),
                "page startMs must be < endMs",
            ));
        }
    }
    let mut next_of = std::collections::BTreeMap::new();
    for (i, page) in pages.iter().enumerate() {
        let cursor = page["cursor"].as_str().unwrap().to_owned();
        let next = match &page["nextCursor"] {
            Value::Null => None,
            Value::String(next) => {
                if !cursors.contains(next) {
                    return Err(invalid(
                        &format!("/pages/{i}/nextCursor"),
                        "nextCursor must name another page",
                    ));
                }
                Some(next.clone())
            }
            _ => {
                return Err(invalid(
                    &format!("/pages/{i}/nextCursor"),
                    "nextCursor must be a page cursor or null",
                ))
            }
        };
        next_of.insert(cursor, next);
    }
    for start in &cursors {
        let mut seen = BTreeSet::new();
        let mut current = Some(start.clone());
        while let Some(cursor) = current {
            if !seen.insert(cursor.clone()) {
                return Err(invalid("/pages", "page cursor graph contains a cycle"));
            }
            current = next_of.get(&cursor).and_then(|next| next.clone());
        }
    }
    Ok(())
}

/// Sorted-key JSON with no insignificant whitespace. Integer-valued numbers have no fraction.
/// Same key order as `JSON.stringify` after sorting object keys.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(&mut out, value);
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn write_canonical(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&canonical_number(number)),
        Value::String(text) => out.push_str(&serde_json::to_string(text).expect("string json")),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(*key).expect("key json"));
                out.push(':');
                write_canonical(out, &map[*key]);
            }
            out.push('}');
        }
    }
}

fn canonical_number(number: &serde_json::Number) -> String {
    if let Some(value) = number.as_u64() {
        return value.to_string();
    }
    if let Some(value) = number.as_i64() {
        return value.to_string();
    }
    if let Some(value) = number.as_f64() {
        const MAX_SAFE: f64 = 9007199254740991.0;
        if value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE {
            return (value as i64).to_string();
        }
    }
    number.to_string()
}

pub fn validate_generation_freeze(
    value: &Value,
    receipt: Option<&Value>,
) -> Result<(), ContractError> {
    public()?.validate(FREEZE, value)?;
    if !range_ok(&value["range"]) {
        return Err(invalid("/range", "range startMs must be <= endMs"));
    }
    if let Some(receipt) = receipt {
        if value["selectionDigest"] != receipt["selectionDigest"] {
            return Err(invalid(
                "/selectionDigest",
                "freeze selectionDigest must match tracking receipt",
            ));
        }
        if value["sourceDigest"] != receipt["sourceDigest"] {
            return Err(invalid(
                "/sourceDigest",
                "freeze sourceDigest must match tracking receipt",
            ));
        }
        if value["maskArtifact"]["artifactId"] != receipt["maskArtifact"]["artifactId"]
            || value["maskArtifact"]["digest"] != receipt["maskArtifact"]["digest"]
        {
            return Err(invalid(
                "/maskArtifact",
                "freeze maskArtifact must bind the same opaque receipt mask",
            ));
        }
        if receipt["ready"] != true {
            return Err(invalid(
                "/trackingReceiptDigest",
                "generation freeze requires ready tracking receipt",
            ));
        }
    }
    Ok(())
}

const FIXTURES: &str = include_str!("../../../profiles/video-target-keyframes-v1/fixtures/receipts.json");

/// The public tracking input that every package declaring this profile must accept unchanged.
pub fn canonical_tracking_input() -> Result<Value, ContractError> {
    let fixtures: Value = serde_json::from_str(FIXTURES)
        .map_err(|_| ContractError::new("INVALID_PACKAGE", "/fixtures", "profile fixtures are not JSON"))?;
    let input = fixtures["valid"]["trackingInput"].clone();
    validate_tracking_input(&input)?;
    Ok(input)
}

/// A trackingPreview operation is submittable only when its registered input schema and its media
/// slots accept the canonical public input. Hosts run this when projecting capabilities, so an
/// installed package with a stale input contract never offers tracking.
pub fn check_tracking_operation(input_schema_json: &str, media_slots: &Value) -> Result<(), ContractError> {
    let input = canonical_tracking_input()?;
    let id = String::from("tracking-input");
    let registry = SchemaRegistry::register(vec![(
        id.clone(),
        input_schema_json.to_owned(),
        crate::hash_bytes(input_schema_json.as_bytes()),
    )])?;
    registry.validate(&id, &input)?;
    let source_id = input["sourceIdentity"]["mediaId"].as_str().unwrap_or_default();
    let source = serde_json::json!({"kind": "video"});
    let mut media = std::collections::BTreeMap::new();
    media.insert(source_id, &source);
    crate::media::validate_slot_references(&input, media_slots, &media)
}

/// Host publishes this feature only after consuming declaration/selection/tracking/freeze.
pub fn feature_id() -> &'static str {
    FEATURE
}
