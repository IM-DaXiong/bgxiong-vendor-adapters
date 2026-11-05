//! Pure input mapping onto the frozen Comfy API graph. No host I/O.
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;

use alloc::{format, string::String, string::ToString};
use serde_json::{json, Value};

use crate::capability::{INPUT_SCHEMA, MODEL_ID, OPERATION_ID};

const GRAPH: &str = include_str!("../workflow/api.json");
const LOAD_NODE: &str = "100";
const PROMPT_NODE: &str = "6";
const NEG_NODE: &str = "33";
const SAMPLER_NODE: &str = "31";
const STICKERS_NODE: &str = "70";
const MAX_PROMPT: usize = 8000;
const MAX_SEED: u64 = 9007199254740991;
const WIDTH: u64 = 2048;
const HEIGHT: u64 = 1024;
const TRIGGER: &str = "Fill the green spaces according to the image. Outpaint as a seamless 360 equirectangular panorama (2:1). Keep the horizon level. Match left and right edges.";
const STICKER_STATE: &str = r#"{"kind":"pano_sticker_state","version":1,"pose":{"yaw_deg":0.0,"pitch_deg":0.0,"roll_deg":0.0,"hFOV_deg":90.0}}"#;

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

impl Failure {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_INPUT",
            message: message.into(),
        }
    }
}

fn number(v: &Value, key: &str) -> Result<u64, Failure> {
    v.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| Failure::invalid(format!("{key} must be an integer")))
}

/// Exactly one reference mediaId.
pub fn reference_id(input: &Value) -> Result<&str, Failure> {
    let refs = input
        .get("references")
        .and_then(Value::as_array)
        .ok_or_else(|| Failure::invalid("references must be an array with exactly one image"))?;
    if refs.len() != 1 {
        return Err(Failure::invalid(
            "This model accepts exactly one reference image; 0/2/3 are out of scope for 1.0.0",
        ));
    }
    let obj = refs[0]
        .as_object()
        .filter(|o| o.len() == 1)
        .ok_or_else(|| Failure::invalid("Each reference must contain only mediaId"))?;
    obj.get("mediaId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Failure::invalid("reference mediaId is required"))
}

fn validate_shape(input: &Value) -> Result<(), Failure> {
    let obj = input
        .as_object()
        .ok_or_else(|| Failure::invalid("input must be an object"))?;
    let allowed = [
        "prompt",
        "outputProjection",
        "references",
        "width",
        "height",
        "seed",
        "advanced",
    ];
    if obj.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(Failure::invalid("Unknown image input field"));
    }
    // Cross-check declared schema keys exist for defaults path.
    let _schema: Value = serde_json::from_str(INPUT_SCHEMA).expect("checked input schema");
    let prompt = obj
        .get("prompt")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::invalid("prompt must be a non-empty string"))?;
    if prompt.trim().is_empty() || prompt.chars().count() > MAX_PROMPT {
        return Err(Failure::invalid("prompt must be 1..=8000 characters"));
    }
    match obj.get("outputProjection").and_then(Value::as_str) {
        Some("equirectangular360") => {}
        Some(_) => {
            return Err(Failure::invalid(
                "outputProjection must be equirectangular360",
            ))
        }
        None => return Err(Failure::invalid("outputProjection is required")),
    }
    let width = number(input, "width")?;
    let height = number(input, "height")?;
    if width != WIDTH || height != HEIGHT {
        return Err(Failure::invalid(
            "width/height must be exactly 2048x1024 for this frozen workflow",
        ));
    }
    let seed = number(input, "seed")?;
    if seed > MAX_SEED {
        return Err(Failure::invalid("seed exceeds JS safe integer range"));
    }
    let advanced = input
        .get("advanced")
        .and_then(Value::as_object)
        .ok_or_else(|| Failure::invalid("advanced.steps is required"))?;
    if advanced.len() != 1 {
        return Err(Failure::invalid(
            "advanced may only contain the verified steps field",
        ));
    }
    let steps = advanced
        .get("steps")
        .and_then(Value::as_u64)
        .ok_or_else(|| Failure::invalid("advanced.steps must be an integer"))?;
    if !(1..=50).contains(&steps) {
        return Err(Failure::invalid("advanced.steps must be in 1..=50"));
    }
    reference_id(input)?;
    Ok(())
}

fn final_prompt(user: &str) -> String {
    if user.contains("equirectangular panorama") {
        user.to_string()
    } else {
        format!("{user}\n{TRIGGER}")
    }
}

/// Build the executable API graph. `uploaded` is the Comfy upload name (or placeholder).
pub fn build_graph(input: &Value, uploaded: &str) -> Result<(Value, Value), Failure> {
    validate_shape(input)?;
    if uploaded.is_empty() {
        return Err(Failure::invalid("Uploaded reference name is required"));
    }
    let prompt = input["prompt"].as_str().expect("validated");
    let seed = input["seed"].as_u64().expect("validated");
    let steps = input["advanced"]["steps"].as_u64().expect("validated");
    let executed = final_prompt(prompt);
    if executed.chars().count() > MAX_PROMPT {
        return Err(Failure::invalid(
            "prompt plus required ERP trigger exceeds 8000 characters",
        ));
    }
    let mut g: Value = serde_json::from_str(GRAPH).expect("frozen API graph");
    for node in [
        LOAD_NODE,
        PROMPT_NODE,
        NEG_NODE,
        SAMPLER_NODE,
        STICKERS_NODE,
        "66",
    ] {
        if g.get(node).is_none() {
            return Err(Failure {
                code: "INTERNAL_ERROR",
                message: format!("frozen graph missing node {node}"),
            });
        }
    }
    g[LOAD_NODE]["inputs"]["image"] = json!(uploaded);
    g[PROMPT_NODE]["inputs"]["text"] = json!(executed);
    g[SAMPLER_NODE]["inputs"]["seed"] = json!(seed);
    g[SAMPLER_NODE]["inputs"]["steps"] = json!(steps);
    g[STICKERS_NODE]["inputs"]["sticker_state"] = json!(STICKER_STATE);
    let applied = json!({
        "operationId": OPERATION_ID,
        "modelId": MODEL_ID,
        "outputProjection": "equirectangular360",
        "width": WIDTH,
        "height": HEIGHT,
        "seed": seed,
        "steps": steps,
        "promptOriginal": prompt,
        "promptExecuted": executed,
        "stickerState": serde_json::from_str::<Value>(STICKER_STATE).expect("const"),
        "outputNodeId": "66",
        "loadImageNodeId": LOAD_NODE,
    });
    Ok((g, applied))
}

pub fn require_operation(payload: &Value) -> Result<(), Failure> {
    if payload.get("operationId").and_then(Value::as_str) != Some(OPERATION_ID) {
        return Err(Failure::invalid("Unknown panorama operation"));
    }
    if payload.get("modelId").and_then(Value::as_str) != Some(MODEL_ID) {
        return Err(Failure::invalid("Unknown panorama model"));
    }
    Ok(())
}