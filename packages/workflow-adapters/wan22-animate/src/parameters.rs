//! Strict parsing of the submitted request. Every tunable value comes from the
//! request; nothing is filled from template constants. Only values illegal for
//! the model/nodes are rejected; there are no product caps on size, fps or length.
use alloc::{string::String, string::ToString, vec::Vec};
use serde_json::Value;

use crate::Failure;

pub const MAX_SAFE_SEED: u64 = (1 << 53) - 1;
pub const SAMPLERS: &[&str] = &[
    "euler",
    "euler_ancestral",
    "dpmpp_2m",
    "dpmpp_sde",
    "uni_pc",
    "lcm",
];
pub const SCHEDULERS: &[&str] = &[
    "simple",
    "normal",
    "karras",
    "exponential",
    "sgm_uniform",
    "beta",
];

/// Operation selected by the submitted operation id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Character replace: segment the target and keep the source background.
    Mix,
    /// Drive performance: animate the replacement image with the source motion only.
    Move,
    /// Mask-only tracking preview (video-target-keyframes-v1).
    Tracking,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Advanced {
    pub window_frames: u64,
    pub overlap_frames: u64,
    pub seed: u64,
    pub steps: u64,
    pub cfg: f64,
    pub sampler: String,
    pub scheduler: String,
    pub sampling_shift: f64,
    pub distill_lora_strength: f64,
    pub relight_lora_strength: f64,
}

/// Character-replace-only inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct Replace {
    pub positive_points: Vec<Point>,
    /// Milliseconds relative to the cropped source clip.
    pub frame_time_ms: u64,
    pub mask_grow_pixels: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub source_media_id: String,
    pub replacement_media_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Present exactly for [`Mode::Mix`] when using legacy single-frame points.
    pub replace: Option<Replace>,
    /// Confirmed tracking mask; when set, Mix must not re-run SAM.
    pub mask_artifact: Option<crate::target_keyframes::MaskArtifactRef>,
    /// Mix-only mask grow; set whenever Mix runs (legacy or maskArtifact).
    pub mask_grow_pixels: Option<u64>,
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
    v.get(key)
        .ok_or_else(|| invalid("A required parameter is missing"))
}
fn int(v: &Value, key: &str, min: u64, max: u64) -> Result<u64, Failure> {
    let n = field(v, key)?
        .as_u64()
        .ok_or_else(|| invalid("An integer parameter is not a non-negative integer"))?;
    if n < min || n > max {
        return Err(invalid(
            "An integer parameter is outside its declared range",
        ));
    }
    Ok(n)
}
fn num(v: &Value, key: &str, min: f64, max: f64) -> Result<f64, Failure> {
    let n = field(v, key)?
        .as_f64()
        .ok_or_else(|| invalid("A numeric parameter is not a number"))?;
    if !(min..=max).contains(&n) {
        return Err(invalid("A numeric parameter is outside its declared range"));
    }
    Ok(n)
}
fn text(v: &Value, key: &str) -> Result<String, Failure> {
    Ok(field(v, key)?
        .as_str()
        .ok_or_else(|| invalid("A text parameter is not a string"))?
        .to_string())
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
/// Wan latent frames are 4n+1 pixel frames; window and overlap must align.
fn aligned(n: u64) -> bool {
    n % 4 == 1
}
/// Rejects a key that the selected operation does not declare.
fn absent(v: &Value, key: &str) -> Result<(), Failure> {
    if v.get(key).is_some() {
        return Err(invalid(
            "A parameter is not declared by the selected operation",
        ));
    }
    Ok(())
}

fn replace(input: &Value, advanced: &Value, start_ms: u64, end_ms: u64, fps: u64) -> Result<Replace, Failure> {
    let target = field(input, "targetPoints")?;
    let points = field(target, "positive")?
        .as_array()
        .ok_or_else(|| invalid("targetPoints.positive must be an array"))?;
    if points.is_empty() || points.len() > 16 {
        return Err(invalid("targetPoints.positive needs 1 to 16 points"));
    }
    let positive_points = points
        .iter()
        .map(|p| {
            Ok(Point {
                x: num(p, "x", 0.0, f64::MAX)?,
                y: num(p, "y", 0.0, f64::MAX)?,
            })
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    let span = end_ms
        .checked_sub(start_ms)
        .ok_or_else(|| invalid("sourceRange is empty"))?;
    // frameTimeMs is relative to the cropped clip; must land on a real target frame.
    let frame_time_ms = int(target, "frameTimeMs", 0, u64::MAX)?;
    if frame_time_ms >= span {
        return Err(invalid(
            "targetPoints.frameTimeMs must be inside the cropped source range",
        ));
    }
    let n = span
        .checked_mul(fps)
        .ok_or_else(|| invalid("sourceRange is too long"))?
        / 1000;
    if n == 0 {
        return Err(invalid(
            "sourceRange is shorter than one frame at the requested fps",
        ));
    }
    let anchor = frame_time_ms
        .checked_mul(fps)
        .ok_or_else(|| invalid("targetPoints.frameTimeMs is too large"))?
        / 1000;
    if anchor >= n {
        return Err(invalid(
            "targetPoints.frameTimeMs does not map to a frame inside the generation window",
        ));
    }
    Ok(Replace {
        positive_points,
        frame_time_ms,
        mask_grow_pixels: int(advanced, "maskGrowPixels", 0, 128)?,
    })
}

pub fn parse(input: &Value, mode: Mode) -> Result<Request, Failure> {
    let range = field(input, "sourceRange")?;
    if range.get("mediaId") != field(input, "source")?.get("mediaId") {
        return Err(invalid("sourceRange.mediaId must name the source media"));
    }
    let start_ms = int(range, "startMs", 0, u64::MAX)?;
    let end_ms = int(range, "endMs", 1, u64::MAX)?;
    if end_ms <= start_ms {
        return Err(invalid("sourceRange.endMs must be greater than startMs"));
    }
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
    let window_frames = int(a, "windowFrames", 5, u64::MAX)?;
    let overlap_frames = int(a, "overlapFrames", 1, u64::MAX)?;
    if !aligned(window_frames) || !aligned(overlap_frames) {
        return Err(invalid("windowFrames and overlapFrames must be 4n+1"));
    }
    if overlap_frames >= window_frames {
        return Err(invalid("overlapFrames must be smaller than windowFrames"));
    }
    let fps = int(input, "fps", 1, u64::MAX)?;
    let (replace, mask_artifact, mask_grow_pixels) = match mode {
        Mode::Mix => {
            let artifact = match input.get("maskArtifact") {
                None | Some(Value::Null) => None,
                Some(v) => Some(crate::target_keyframes::parse_mask_artifact(v)?),
            };
            let replace = if artifact.is_some() {
                match input.get("targetPoints") {
                    None | Some(Value::Null) => None,
                    Some(_) => Some(replace(input, a, start_ms, end_ms, fps)?),
                }
            } else {
                Some(replace(input, a, start_ms, end_ms, fps)?)
            };
            if artifact.is_none() && replace.is_none() {
                return Err(invalid("Mix requires targetPoints or maskArtifact"));
            }
            let grow = int(a, "maskGrowPixels", 0, 128)?;
            (replace, artifact, Some(grow))
        }
        Mode::Move => {
            absent(input, "targetPoints")?;
            absent(input, "maskArtifact")?;
            absent(a, "maskGrowPixels")?;
            (None, None, None)
        }
        Mode::Tracking => {
            return Err(invalid(
                "Tracking requests must use parse_tracking, not generation parse",
            ));
        }
    };
    Ok(Request {
        source_media_id: media_id(input, "source")?,
        replacement_media_id: media_id(input, "replacement")?,
        start_ms,
        end_ms,
        replace,
        mask_artifact,
        mask_grow_pixels,
        prompt,
        negative_prompt: text(input, "negativePrompt")?,
        width,
        height,
        fps,
        keep_audio,
        advanced: Advanced {
            window_frames,
            overlap_frames,
            seed: int(a, "seed", 0, MAX_SAFE_SEED)?,
            steps: int(a, "steps", 1, 100)?,
            cfg: num(a, "cfg", 0.0, 30.0)?,
            sampler: choice(a, "sampler", SAMPLERS)?,
            scheduler: choice(a, "scheduler", SCHEDULERS)?,
            sampling_shift: num(a, "samplingShift", 0.0, 100.0)?,
            distill_lora_strength: num(a, "distillLoraStrength", -4.0, 4.0)?,
            relight_lora_strength: num(a, "relightLoraStrength", -4.0, 4.0)?,
        },
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub fn sample() -> Value {
        json!({
            "source": {"mediaId": "m-src"}, "sourceRange": {"mediaId": "m-src", "startMs": 0, "endMs": 3000},
            "replacement": {"mediaId": "m-ref"}, "targetPoints": {"positive": [{"x": 320, "y": 180}], "frameTimeMs": 0},
            "prompt": "a person dancing", "negativePrompt": "", "width": 640, "height": 640,
            "fps": 24, "audio": "source",
            "advanced": {"windowFrames": 77, "overlapFrames": 5, "seed": 42, "steps": 6, "cfg": 1,
                "sampler": "euler", "scheduler": "simple", "samplingShift": 8,
                "distillLoraStrength": 1, "relightLoraStrength": 1, "maskGrowPixels": 10}
        })
    }
    pub fn sample_move() -> Value {
        let mut v = sample();
        v.as_object_mut().unwrap().remove("targetPoints");
        v["advanced"]
            .as_object_mut()
            .unwrap()
            .remove("maskGrowPixels");
        v
    }

    #[test]
    fn parses_complete_request() {
        let r = parse(&sample(), Mode::Mix).ok().unwrap();
        assert_eq!(
            (r.width, r.fps, r.advanced.window_frames, r.keep_audio),
            (640, 24, 77, true)
        );
        assert!(parse(&sample_move(), Mode::Move)
            .ok()
            .unwrap()
            .replace
            .is_none());
    }

    #[test]
    fn no_product_caps_on_size_fps_or_window() {
        let mut v = sample();
        v["width"] = 4096.into();
        v["height"] = 2304.into();
        v["fps"] = 120.into();
        v["advanced"]["windowFrames"] = 241.into();
        assert!(parse(&v, Mode::Mix).is_ok());
    }

    #[test]
    fn rejects_illegal_values_without_adjusting() {
        for (ptr, bad) in [
            ("/width", json!(1080)),
            ("/advanced/windowFrames", json!(76)),
            ("/advanced/overlapFrames", json!(77)),
            ("/sourceRange/endMs", json!(0)),
            ("/advanced/sampler", json!("made_up")),
        ] {
            let mut v = sample();
            *v.pointer_mut(ptr).unwrap() = bad;
            assert!(parse(&v, Mode::Mix).is_err(), "{ptr} must be rejected");
        }
        let mut v = sample();
        v["advanced"].as_object_mut().unwrap().remove("seed");
        assert!(
            parse(&v, Mode::Mix).is_err(),
            "missing values are never defaulted"
        );
        assert!(
            parse(&sample(), Mode::Move).is_err(),
            "Move does not accept replace-only inputs"
        );
        assert!(
            parse(&sample_move(), Mode::Mix).is_err(),
            "Mix requires target points"
        );
    }

    #[test]
    fn frame_time_ms_required_and_bounded() {
        let mut missing = sample();
        missing["targetPoints"].as_object_mut().unwrap().remove("frameTimeMs");
        assert!(parse(&missing, Mode::Mix).is_err(), "frameTimeMs is required");

        let mut ok_zero = sample();
        ok_zero["targetPoints"]["frameTimeMs"] = 0.into();
        let r = parse(&ok_zero, Mode::Mix).ok().unwrap();
        assert_eq!(r.replace.as_ref().unwrap().frame_time_ms, 0);

        let mut mid = sample();
        mid["sourceRange"]["endMs"] = 5000.into();
        mid["targetPoints"]["frameTimeMs"] = 1500.into();
        mid["fps"] = 24.into();
        let r = parse(&mid, Mode::Mix).ok().unwrap();
        assert_eq!(r.replace.as_ref().unwrap().frame_time_ms, 1500);

        let mut oob = sample();
        oob["sourceRange"]["endMs"] = 3000.into();
        oob["targetPoints"]["frameTimeMs"] = 3000.into();
        assert!(parse(&oob, Mode::Mix).is_err(), "frameTimeMs == span is out of range");

        let mut past = sample();
        past["sourceRange"]["endMs"] = 3000.into();
        past["targetPoints"]["frameTimeMs"] = 5000.into();
        assert!(parse(&past, Mode::Mix).is_err());
    }

    #[test]
    fn move_still_rejects_target_points() {
        let mut v = sample_move();
        v["targetPoints"] = json!({"positive": [{"x": 1, "y": 2}], "frameTimeMs": 0});
        assert!(parse(&v, Mode::Move).is_err());
        assert!(parse(&sample_move(), Mode::Move).is_ok());
    }

    #[test]
    fn mask_artifact_allows_mix_without_points() {
        let mut v = sample();
        v.as_object_mut().unwrap().remove("targetPoints");
        v["maskArtifact"] = json!({"artifactId": "art-1", "digest": "d1"});
        let r = parse(&v, Mode::Mix).ok().unwrap();
        assert!(r.replace.is_none());
        assert_eq!(r.mask_artifact.as_ref().unwrap().artifact_id, "art-1");
        assert!(parse(&v, Mode::Tracking).is_err());
    }
}
