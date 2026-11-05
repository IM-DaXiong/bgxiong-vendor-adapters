//! RunningHub OpenAPI V2 AI App mapper. Isolated H3 example - not a client vendor.

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde_json::{json, Value};

use crate::config::{
    submit_ai_app_url, KindWidgetTable, WeightPin, APP_ID_1SLOT, APP_ID_2SLOT, APP_ID_3SLOT,
    APP_ID_4SLOT, APP_ID_FL2VA, APP_ID_I2V_TURBO, APP_ID_SOUNDTRACK, ASPECT_FIELD_NAME,
    AUDIO_FIELD_NAME, AUDIO_NODE_ID_SOUNDTRACK, FL2VA_WEIGHT_PINS, FL2VA_WIDGETS, IMAGE_FIELD_NAME,
    MODEL_ID_1SLOT, MODEL_ID_2SLOT, MODEL_ID_3SLOT, MODEL_ID_4SLOT, MODEL_ID_FL2VA,
    MODEL_ID_I2V_TURBO, MODEL_ID_SOUNDTRACK, REF1_WEIGHT_PINS, REF1_WIDGETS, REF_IMAGE_NODE_IDS_1,
    REF_IMAGE_NODE_IDS_1REF, REF_IMAGE_NODE_IDS_2, REF_IMAGE_NODE_IDS_3, REF_IMAGE_NODE_IDS_4,
    REF_IMAGE_NODE_IDS_FL2VA, REF_IMAGE_NODE_IDS_SOUNDTRACK, RESOLUTION_FIELD_NAME,
    SLOT1_WEIGHT_PINS, SLOT1_WIDGETS, SLOT2_WIDGETS, SLOT3_WIDGETS, SLOT4_WIDGETS,
    SLOT_REF2VA_WEIGHT_PINS, SOUNDTRACK_WEIGHT_PINS, SOUNDTRACK_WIDGETS, SUBMIT_MODE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Ref1,
    Slot2,
    Slot3,
    Slot4,
    I2vTurbo,
    Fl2va,
    Soundtrack,
}

impl ModelKind {
    pub fn app_id(self) -> &'static str {
        match self {
            ModelKind::Ref1 => APP_ID_1SLOT,
            ModelKind::Slot2 => APP_ID_2SLOT,
            ModelKind::Slot3 => APP_ID_3SLOT,
            ModelKind::Slot4 => APP_ID_4SLOT,
            ModelKind::I2vTurbo => APP_ID_I2V_TURBO,
            ModelKind::Fl2va => APP_ID_FL2VA,
            ModelKind::Soundtrack => APP_ID_SOUNDTRACK,
        }
    }

    pub fn max_refs(self) -> usize {
        match self {
            ModelKind::Ref1 | ModelKind::I2vTurbo | ModelKind::Fl2va | ModelKind::Soundtrack => 1,
            ModelKind::Slot2 => 2,
            ModelKind::Slot3 => 3,
            ModelKind::Slot4 => 4,
        }
    }

    pub fn requires_reference_audio(self) -> bool {
        matches!(self, ModelKind::Soundtrack)
    }

    pub fn ref_node_ids(self) -> &'static [&'static str] {
        match self {
            ModelKind::Ref1 => &REF_IMAGE_NODE_IDS_1REF,
            ModelKind::Slot2 => &REF_IMAGE_NODE_IDS_2,
            ModelKind::Slot3 => &REF_IMAGE_NODE_IDS_3,
            ModelKind::Slot4 => &REF_IMAGE_NODE_IDS_4,
            ModelKind::I2vTurbo => &REF_IMAGE_NODE_IDS_1,
            ModelKind::Fl2va => &REF_IMAGE_NODE_IDS_FL2VA,
            ModelKind::Soundtrack => &REF_IMAGE_NODE_IDS_SOUNDTRACK,
        }
    }

    pub fn widgets(self) -> KindWidgetTable {
        match self {
            ModelKind::Ref1 => REF1_WIDGETS,
            ModelKind::Slot2 => SLOT2_WIDGETS,
            ModelKind::Slot3 => SLOT3_WIDGETS,
            ModelKind::Slot4 => SLOT4_WIDGETS,
            ModelKind::I2vTurbo => SLOT1_WIDGETS,
            ModelKind::Fl2va => FL2VA_WIDGETS,
            ModelKind::Soundtrack => SOUNDTRACK_WIDGETS,
        }
    }

    pub fn weight_pins(self) -> &'static [WeightPin] {
        match self {
            ModelKind::Ref1 => REF1_WEIGHT_PINS,
            ModelKind::Slot2 | ModelKind::Slot3 | ModelKind::Slot4 => SLOT_REF2VA_WEIGHT_PINS,
            ModelKind::I2vTurbo => SLOT1_WEIGHT_PINS,
            ModelKind::Fl2va => FL2VA_WEIGHT_PINS,
            ModelKind::Soundtrack => SOUNDTRACK_WEIGHT_PINS,
        }
    }

    pub fn submit_url(self) -> String {
        submit_ai_app_url(self.app_id())
    }

    pub fn model_id(self) -> &'static str {
        match self {
            ModelKind::Ref1 => MODEL_ID_1SLOT,
            ModelKind::Slot2 => MODEL_ID_2SLOT,
            ModelKind::Slot3 => MODEL_ID_3SLOT,
            ModelKind::Slot4 => MODEL_ID_4SLOT,
            ModelKind::I2vTurbo => MODEL_ID_I2V_TURBO,
            ModelKind::Fl2va => MODEL_ID_FL2VA,
            ModelKind::Soundtrack => MODEL_ID_SOUNDTRACK,
        }
    }
}

pub fn parse_model_kind(model: Option<&str>) -> Result<ModelKind, String> {
    match model.map(str::trim).filter(|s| !s.is_empty()) {
        Some(m) if m == MODEL_ID_1SLOT => Ok(ModelKind::Ref1),
        Some(m) if m == MODEL_ID_2SLOT => Ok(ModelKind::Slot2),
        Some(m) if m == MODEL_ID_3SLOT => Ok(ModelKind::Slot3),
        Some(m) if m == MODEL_ID_4SLOT => Ok(ModelKind::Slot4),
        Some(m) if m == MODEL_ID_I2V_TURBO => Ok(ModelKind::I2vTurbo),
        Some(m) if m == MODEL_ID_FL2VA => Ok(ModelKind::Fl2va),
        Some(m) if m == MODEL_ID_SOUNDTRACK => Ok(ModelKind::Soundtrack),
        Some(m) => Err(format!(
            "unknown model {m}; expected {MODEL_ID_1SLOT}, {MODEL_ID_2SLOT}, {MODEL_ID_3SLOT}, {MODEL_ID_4SLOT}, {MODEL_ID_I2V_TURBO}, {MODEL_ID_FL2VA} or {MODEL_ID_SOUNDTRACK}"
        )),
        None => Err(format!(
            "model is required; expected {MODEL_ID_1SLOT}, {MODEL_ID_2SLOT}, {MODEL_ID_3SLOT}, {MODEL_ID_4SLOT}, {MODEL_ID_I2V_TURBO}, {MODEL_ID_FL2VA} or {MODEL_ID_SOUNDTRACK}"
        )),
    }
}

fn push_node_str(list: &mut Vec<Value>, node_id: &str, field_name: &str, field_value: &str) {
    if node_id.trim().is_empty() {
        return;
    }
    list.push(json!({
        "nodeId": node_id,
        "fieldName": field_name,
        "fieldValue": field_value
    }));
}

fn format_num_string(n: f64) -> String {
    // no_std wasm has no f64::trunc.
    let whole = n as i64;
    if (n - (whole as f64)).abs() < 1e-12 {
        format!("{whole}")
    } else {
        format!("{n}")
    }
}

fn h3_length_expression(fps: f64) -> Result<String, String> {
    // no_std wasm has no f64::round; only the workbench enum values 24 / 25 / 30 are legal.
    let n = if fps == 24.0 {
        24
    } else if fps == 25.0 {
        25
    } else if fps == 30.0 {
        30
    } else {
        return Err("closed_set_fps:24,25,30".into());
    };
    Ok(format!(
        "max(5, round(a * {n})) + (5 - (max(5, round(a * {n})) % 17)) % 17"
    ))
}

pub fn resolution_combo(resolution: &str) -> Result<String, String> {
    match resolution.trim() {
        "1:1" | "1:1 (Square)" => Ok("1:1 (Square)".into()),
        "16:9" | "16:9 (Widescreen)" => Ok("16:9 (Widescreen)".into()),
        "9:16" | "9:16 (Portrait Widescreen)" => Ok("9:16 (Portrait Widescreen)".into()),
        "4:3" | "4:3 (Standard)" => Ok("4:3 (Standard)".into()),
        other => Err(format!("unsupported aspect {other}")),
    }
}

/// Closed H3 megapixel tiers. Echo `mp:0.4` so RCD-12 matches the workbench id.
pub fn megapixels_from_resolution(resolution: &str) -> Result<(String, f64), String> {
    let t = resolution.trim();
    let n = if let Some(rest) = t.strip_prefix("mp:") {
        rest.parse::<f64>()
            .map_err(|_| format!("unsupported resolution {t}"))?
    } else {
        t.parse::<f64>()
            .map_err(|_| format!("unsupported resolution {t}"))?
    };
    let (id, mp) = if (n - 0.25).abs() < 1e-9 {
        ("mp:0.25", 0.25)
    } else if (n - 0.4).abs() < 1e-9 {
        ("mp:0.4", 0.4)
    } else if (n - 0.6).abs() < 1e-9 {
        ("mp:0.6", 0.6)
    } else if (n - 1.0).abs() < 1e-9 {
        ("mp:1", 1.0)
    } else {
        return Err(format!("unsupported resolution {t}"));
    };
    Ok((id.into(), mp))
}

/// Workbench widgets plus the per-slot CLIP/UNet/VAE/scheduler pin table.
/// OpenAPI merge does not keep webpage defaults for omitted weight nodes.
pub fn mapped_node_info(
    kind: ModelKind,
    prompt: &str,
    duration: Option<f64>,
    fps: Option<f64>,
    resolution: Option<&str>,
    aspect: Option<&str>,
    uploaded: &[&str],
    uploaded_audio: Option<&str>,
) -> Result<Value, String> {
    if SUBMIT_MODE != "mapped_run" {
        return Ok(json!([]));
    }
    if prompt.is_empty() {
        return Err("submit requires prompt".into());
    }
    let w = kind.widgets();
    let mut list = Vec::new();
    push_node_str(&mut list, w.prompt_node, w.prompt_field, prompt);
    {
        let Some(d) = duration else {
            return Err("duration node is configured; durationSeconds is required".into());
        };
        if d < 1.0 || d > 15.0 {
            return Err("durationSeconds must be 1 to 15".into());
        }
        push_node_str(
            &mut list,
            w.duration_node,
            w.duration_field,
            &format_num_string(d),
        );
    }
    if w.fps_node.trim().is_empty() {
        // Graphs without an fps widget (fl2va node 131 is noise_seed).
    } else if let Some(f) = fps {
        let expr = h3_length_expression(f)?;
        push_node_str(&mut list, w.fps_node, w.fps_field, &format_num_string(f));
        if !w.math_node.trim().is_empty() {
            push_node_str(&mut list, w.math_node, w.math_field, &expr);
        }
    }
    {
        let Some(a) = aspect.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("aspect node is configured; aspect is required".into());
        };
        let combo = resolution_combo(a)?;
        push_node_str(&mut list, w.aspect_node, ASPECT_FIELD_NAME, &combo);
    }
    {
        let Some(r) = resolution.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("resolution node is configured; resolution is required".into());
        };
        let (_, mp) = megapixels_from_resolution(r)?;
        push_node_str(
            &mut list,
            w.resolution_node,
            RESOLUTION_FIELD_NAME,
            &format_num_string(mp),
        );
    }
    let names: Vec<&str> = uploaded
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        return Err("r2v requires at least one reference image".into());
    }
    if names.len() > kind.max_refs() {
        return Err(format!(
            "r2v model accepts at most {} reference images",
            kind.max_refs()
        ));
    }
    let nodes = kind.ref_node_ids();
    for (i, n) in names.iter().enumerate() {
        let node_id = nodes[i];
        if node_id == "141" {
            return Err(format!(
                "{} graph must not write image onto TurboLoRA node 141",
                kind.model_id()
            ));
        }
        if node_id == AUDIO_NODE_ID_SOUNDTRACK && kind == ModelKind::Soundtrack {
            return Err(format!(
                "{} graph must not write image onto Load Audio node {AUDIO_NODE_ID_SOUNDTRACK}",
                kind.model_id()
            ));
        }
        if kind == ModelKind::Soundtrack && n.to_ascii_lowercase().ends_with(".wav") {
            return Err("wav must not be written as a reference image".into());
        }
        push_node_str(&mut list, node_id, IMAGE_FIELD_NAME, n);
    }
    if kind.requires_reference_audio() {
        let Some(audio) = uploaded_audio.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("soundtrack requires one referenceAudios item".into());
        };
        let lower = audio.to_ascii_lowercase();
        if lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".webp")
            || lower.ends_with(".gif")
        {
            return Err("image file must not be written as soundtrack audio".into());
        }
        push_node_str(&mut list, AUDIO_NODE_ID_SOUNDTRACK, AUDIO_FIELD_NAME, audio);
    }
    if kind == ModelKind::I2vTurbo && list.iter().any(|n| n["nodeId"] == "138") {
        return Err("i2v turbo nodeInfoList must not contain node 138".into());
    }
    if kind == ModelKind::Ref1
        && list
            .iter()
            .any(|n| n["nodeId"] == "114" || n["nodeId"] == "139" || n["nodeId"] == "141")
    {
        return Err("1ref nodeInfoList must not write image nodes 114, 139, or 141".into());
    }
    for (node_id, field_name, field_value) in kind.weight_pins() {
        push_node_str(&mut list, node_id, field_name, field_value);
    }
    Ok(Value::Array(list))
}
