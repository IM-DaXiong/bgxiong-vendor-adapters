//! Strict parsing of the submitted request. Every tunable value comes from the request;
//! nothing is filled from template constants. Only values illegal for the nodes are rejected.
use alloc::{string::String, string::ToString};
use serde_json::Value;

use crate::Failure;

pub const MAX_SAFE_SEED: u64 = (1 << 53) - 1;
pub const SAMPLERS: &[&str] = &["lcm", "euler", "euler_ancestral", "dpmpp_2m", "dpmpp_sde", "uni_pc"];
pub const SCHEDULERS: &[&str] = &["simple", "normal", "karras", "exponential", "sgm_uniform", "beta"];
pub const CACHE_DEVICES: &[&str] = &["gpu", "cpu"];
pub const CACHE_DTYPES: &[&str] = &["int8", "default", "int4"];

#[derive(Debug, Clone, PartialEq)]
pub struct Advanced {
    /// Pose-branch text (`positive_pose`). Empty = the node's own default (the main positive).
    pub motion_prompt: String,
    pub segment_frames: u64,
    pub seed: u64,
    pub steps: u64,
    pub cfg: f64,
    pub sampler: String,
    pub scheduler: String,
    pub sampling_shift: f64,
    pub pose_strength: f64,
    pub pose_start_percent: f64,
    pub pose_end_percent: f64,
    pub reference_image_strength: f64,
    pub context_windows: bool,
    pub cache_device: String,
    pub cache_dtype: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Driving (pose) video, already trimmed to the range and converted to `fps` CFR by the host.
    pub source_media_id: String,
    /// Character image (appearance source).
    pub replacement_media_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Character appearance + background description (main positive conditioning).
    pub prompt: String,
    pub negative_prompt: String,
    pub width: u64,
    pub height: u64,
    pub fps: u64,
    pub keep_audio: bool,
    pub advanced: Advanced,
}

fn invalid(message: &'static str) -> Failure {
    Failure::new("INVALID_INPUT", message)
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value, Failure> {
    v.get(key).ok_or_else(|| invalid("A required parameter is missing"))
}
fn int(v: &Value, key: &str, min: u64, max: u64) -> Result<u64, Failure> {
    let n = field(v, key)?
        .as_u64()
        .ok_or_else(|| invalid("An integer parameter is not a non-negative integer"))?;
    if n < min || n > max {
        return Err(invalid("An integer parameter is outside its declared range"));
    }
    Ok(n)
}
fn num(v: &Value, key: &str, min: f64, max: f64) -> Result<f64, Failure> {
    let n = field(v, key)?.as_f64().ok_or_else(|| invalid("A numeric parameter is not a number"))?;
    if !(min..=max).contains(&n) {
        return Err(invalid("A numeric parameter is outside its declared range"));
    }
    Ok(n)
}
fn text(v: &Value, key: &str) -> Result<String, Failure> {
    Ok(field(v, key)?.as_str().ok_or_else(|| invalid("A text parameter is not a string"))?.to_string())
}
fn flag(v: &Value, key: &str) -> Result<bool, Failure> {
    field(v, key)?.as_bool().ok_or_else(|| invalid("A boolean parameter is not a boolean"))
}
fn choice(v: &Value, key: &str, set: &[&str]) -> Result<String, Failure> {
    let s = text(v, key)?;
    if !set.contains(&s.as_str()) {
        return Err(invalid("A selection parameter is not in its declared set"));
    }
    Ok(s)
}
fn media_id(v: &Value, key: &str) -> Result<String, Failure> {
    let id = text(field(v, key)?, "mediaId")?;
    if id.is_empty() {
        return Err(invalid("A media reference is empty"));
    }
    Ok(id)
}

pub fn parse(input: &Value) -> Result<Request, Failure> {
    let range = field(input, "sourceRange")?;
    if range.get("mediaId") != field(input, "source")?.get("mediaId") {
        return Err(invalid("sourceRange.mediaId must name the source media"));
    }
    let start_ms = int(range, "startMs", 0, u64::MAX)?;
    let end_ms = int(range, "endMs", 1, u64::MAX)?;
    if end_ms <= start_ms {
        return Err(invalid("sourceRange.endMs must be greater than startMs"));
    }
    // WanAnimate2ToVideo width/height step is 16.
    let width = int(input, "width", 16, u64::MAX)?;
    let height = int(input, "height", 16, u64::MAX)?;
    if width % 16 != 0 || height % 16 != 0 {
        return Err(invalid("width and height must be multiples of 16"));
    }
    let prompt = text(input, "prompt")?;
    if prompt.is_empty() {
        return Err(invalid("prompt must not be empty"));
    }
    let keep_audio = match text(input, "audio")?.as_str() {
        "source" => true,
        "none" => false,
        _ => return Err(invalid("audio must be source or none")),
    };
    let a = field(input, "advanced")?;
    // WanAnimate2ToVideo `length` step 4 from 1: 4n+1 pixel frames.
    let segment_frames = int(a, "segmentFrames", 5, u64::MAX)?;
    if segment_frames % 4 != 1 {
        return Err(invalid("segmentFrames must be 4n+1"));
    }
    let pose_start_percent = num(a, "poseStartPercent", 0.0, 1.0)?;
    let pose_end_percent = num(a, "poseEndPercent", 0.0, 1.0)?;
    if pose_end_percent < pose_start_percent {
        return Err(invalid("poseEndPercent must not be before poseStartPercent"));
    }
    Ok(Request {
        source_media_id: media_id(input, "source")?,
        replacement_media_id: media_id(input, "replacement")?,
        start_ms,
        end_ms,
        prompt,
        negative_prompt: text(input, "negativePrompt")?,
        width,
        height,
        fps: int(input, "fps", 1, 120)?,
        keep_audio,
        advanced: Advanced {
            motion_prompt: text(a, "motionPrompt")?,
            segment_frames,
            seed: int(a, "seed", 0, MAX_SAFE_SEED)?,
            steps: int(a, "steps", 1, 100)?,
            cfg: num(a, "cfg", 0.0, 30.0)?,
            sampler: choice(a, "sampler", SAMPLERS)?,
            scheduler: choice(a, "scheduler", SCHEDULERS)?,
            sampling_shift: num(a, "samplingShift", 0.0, 100.0)?,
            pose_strength: num(a, "poseStrength", 0.0, 10.0)?,
            pose_start_percent,
            pose_end_percent,
            reference_image_strength: num(a, "referenceImageStrength", 0.0, 10.0)?,
            context_windows: flag(a, "contextWindows")?,
            cache_device: choice(a, "cacheDevice", CACHE_DEVICES)?,
            cache_dtype: choice(a, "cacheDtype", CACHE_DTYPES)?,
        },
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub fn sample() -> Value {
        json!({
            "source": {"mediaId": "m-src"}, "sourceRange": {"mediaId": "m-src", "startMs": 0, "endMs": 2000},
            "replacement": {"mediaId": "m-ref"},
            "prompt": "a toy figure dancing", "negativePrompt": "blurry", "width": 480, "height": 832,
            "fps": 30, "audio": "source",
            "advanced": {"motionPrompt": "street dance", "segmentFrames": 81, "seed": 7, "steps": 10, "cfg": 1,
                "sampler": "lcm", "scheduler": "simple", "samplingShift": 5, "poseStrength": 1,
                "poseStartPercent": 0, "poseEndPercent": 1, "referenceImageStrength": 1,
                "contextWindows": false, "cacheDevice": "gpu", "cacheDtype": "int8"}
        })
    }

    #[test]
    fn parses_complete_request() {
        let r = parse(&sample()).ok().unwrap();
        assert_eq!((r.width, r.height, r.fps, r.advanced.segment_frames, r.keep_audio), (480, 832, 30, 81, true));
        assert_eq!(r.advanced.motion_prompt, "street dance");
    }

    #[test]
    fn rejects_illegal_values_without_adjusting() {
        for (ptr, bad) in [
            ("/width", json!(482)),
            ("/advanced/segmentFrames", json!(80)),
            ("/advanced/poseEndPercent", json!(1.5)),
            ("/sourceRange/endMs", json!(0)),
            ("/advanced/sampler", json!("made_up")),
            ("/advanced/contextWindows", json!("yes")),
            ("/prompt", json!("")),
        ] {
            let mut v = sample();
            *v.pointer_mut(ptr).unwrap() = bad;
            assert!(parse(&v).is_err(), "{ptr} must be rejected");
        }
        let mut v = sample();
        v["advanced"].as_object_mut().unwrap().remove("seed");
        assert!(parse(&v).is_err(), "missing values are never defaulted");
    }
}