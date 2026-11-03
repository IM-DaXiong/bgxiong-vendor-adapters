//! Parse video-target-keyframes-v1 selection into package SAM inputs.
//! Public SDK fields only — no model-private media paths.
use alloc::{format, string::String, string::ToString, vec::Vec};
use serde_json::Value;

use crate::{parameters::Point, Failure};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    TowardStart,
    TowardEnd,
    Both,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::TowardStart => "towardStart",
            Direction::TowardEnd => "towardEnd",
            Direction::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointLabel {
    Positive,
    Negative,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelPoint {
    pub point_id: String,
    pub x: f64,
    pub y: f64,
    pub label: PointLabel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelBox {
    pub box_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    pub keyframe_id: String,
    pub time_ms: u64,
    pub points: Vec<SelPoint>,
    pub box_: Option<SelBox>,
    pub frame_index: u64,
    pub resolved_time_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameGeometry {
    pub width: u64,
    pub height: u64,
    pub rotation: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub target_id: String,
    pub revision: String,
    pub frame_geometry: FrameGeometry,
    pub keyframes: Vec<Keyframe>,
    pub selection_digest: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackingRequest {
    pub source_media_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub selection: Selection,
    pub direction: Direction,
    pub anchor_keyframe_id: String,
    pub coverage_start_ms: u64,
    pub coverage_end_ms: u64,
    pub request_digest: String,
    pub source_digest: String,
    pub width: u64,
    pub height: u64,
    pub fps: u64,
    /// Actual sample count after host-media facts. Zero means not sampled yet.
    pub sampled_frames: u64,
    /// Presentation time of each sampled frame, ms from the prepared range start.
    pub frame_times_ms: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaskArtifactRef {
    pub artifact_id: String,
    pub digest: String,
}

fn invalid(message: &'static str) -> Failure {
    Failure::new("INVALID_INPUT", message)
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value, Failure> {
    v.get(key)
        .ok_or_else(|| invalid("A required parameter is missing"))
}
fn text(v: &Value, key: &str) -> Result<String, Failure> {
    Ok(field(v, key)?
        .as_str()
        .ok_or_else(|| invalid("A text parameter is not a string"))?
        .to_string())
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
fn num(v: &Value, key: &str) -> Result<f64, Failure> {
    field(v, key)?
        .as_f64()
        .ok_or_else(|| invalid("A numeric parameter is not a number"))
}

pub fn parse_direction(v: &Value) -> Result<Direction, Failure> {
    match v.as_str() {
        Some("towardStart") => Ok(Direction::TowardStart),
        Some("towardEnd") => Ok(Direction::TowardEnd),
        Some("both") => Ok(Direction::Both),
        _ => Err(invalid("direction must be towardStart, towardEnd, or both")),
    }
}

fn parse_point(v: &Value) -> Result<SelPoint, Failure> {
    let label = match text(v, "label")?.as_str() {
        "positive" => PointLabel::Positive,
        "negative" => PointLabel::Negative,
        _ => return Err(invalid("point label must be positive or negative")),
    };
    let x = num(v, "x")?;
    let y = num(v, "y")?;
    if x < 0.0 || y < 0.0 {
        return Err(invalid("point coordinates must be non-negative"));
    }
    Ok(SelPoint {
        point_id: text(v, "pointId")?,
        x,
        y,
        label,
    })
}

fn parse_box(v: &Value) -> Result<SelBox, Failure> {
    let width = num(v, "width")?;
    let height = num(v, "height")?;
    if width <= 0.0 || height <= 0.0 {
        return Err(invalid("box width and height must be positive"));
    }
    let x = num(v, "x")?;
    let y = num(v, "y")?;
    if x < 0.0 || y < 0.0 {
        return Err(invalid("box origin must be non-negative"));
    }
    Ok(SelBox {
        box_id: text(v, "boxId")?,
        x,
        y,
        width,
        height,
    })
}

/// Map a relative time onto actual frame timestamps. The index is the last stamp at or before `time_ms`.
pub fn resolve_frame_at_stamps(time_ms: u64, stamps: &[u64]) -> Result<(u64, u64), Failure> {
    if stamps.is_empty() || stamps.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(invalid("source frame timestamps are unavailable"));
    }
    if time_ms < stamps[0] {
        return Err(invalid("keyframe timeMs is before the first source frame"));
    }
    let mut index = 0usize;
    for (i, stamp) in stamps.iter().enumerate() {
        if *stamp <= time_ms {
            index = i;
        } else {
            break;
        }
    }
    Ok((index as u64, stamps[index]))
}

pub fn resolve_frame_index(time_ms: u64, fps: u64, frame_count: u64) -> Result<(u64, u64), Failure> {
    if fps == 0 || frame_count == 0 {
        return Err(invalid("fps and frame_count must be positive"));
    }
    let idx = time_ms
        .checked_mul(fps)
        .ok_or_else(|| invalid("timeMs is too large"))?
        / 1000;
    if idx >= frame_count {
        return Err(invalid(
            "keyframe timeMs does not map inside the generation window",
        ));
    }
    let resolved = idx
        .checked_mul(1000)
        .ok_or_else(|| invalid("resolved time overflow"))?
        / fps;
    Ok((idx, resolved))
}

enum FrameTiming<'a> {
    /// Structural acceptance only. Real indexes are assigned after host-media facts.
    Structure { span_ms: u64 },
    Constant { fps: u64, span_ms: u64 },
    Stamps { stamps: &'a [u64], span_ms: u64 },
}

pub fn parse_selection(v: &Value, fps: u64, span_ms: u64) -> Result<Selection, Failure> {
    parse_selection_timed(v, FrameTiming::Constant { fps, span_ms })
}

fn parse_selection_timed(v: &Value, timing: FrameTiming<'_>) -> Result<Selection, Failure> {
    if int(v, "schemaVersion", 1, 1)? != 1 {
        return Err(invalid("selection.schemaVersion must be 1"));
    }
    let geom = field(v, "frameGeometry")?;
    let frame_geometry = FrameGeometry {
        width: int(geom, "width", 1, u64::MAX)?,
        height: int(geom, "height", 1, u64::MAX)?,
        rotation: geom.get("rotation").and_then(Value::as_u64).unwrap_or(0),
    };
    if !matches!(frame_geometry.rotation, 0 | 90 | 180 | 270) {
        return Err(invalid("frameGeometry.rotation must be 0/90/180/270"));
    }
    let span_ms = match &timing {
        FrameTiming::Structure { span_ms }
        | FrameTiming::Constant { span_ms, .. }
        | FrameTiming::Stamps { span_ms, .. } => *span_ms,
    };
    let frame_count = match &timing {
        FrameTiming::Structure { .. } => 0,
        FrameTiming::Constant { fps, span_ms } => {
            if *fps == 0 {
                return Err(invalid("fps and frame_count must be positive"));
            }
            let n = span_ms
                .checked_mul(*fps)
                .ok_or_else(|| invalid("sourceRange is too long"))?
                / 1000;
            if n == 0 {
                return Err(invalid(
                    "sourceRange is shorter than one frame at the requested fps",
                ));
            }
            n
        }
        FrameTiming::Stamps { stamps, .. } => {
            if stamps.is_empty() {
                return Err(invalid("source frame timestamps are unavailable"));
            }
            stamps.len() as u64
        }
    };
    let raw = field(v, "keyframes")?
        .as_array()
        .ok_or_else(|| invalid("keyframes must be an array"))?;
    if raw.is_empty() {
        return Err(invalid("selection must contain at least one keyframe"));
    }
    let mut keyframes = Vec::new();
    let mut seen_ids = alloc::collections::BTreeSet::new();
    let mut frame_owners: alloc::collections::BTreeMap<u64, String> =
        alloc::collections::BTreeMap::new();
    for kf in raw {
        let keyframe_id = text(kf, "keyframeId")?;
        if !seen_ids.insert(keyframe_id.clone()) {
            return Err(invalid("duplicate keyframeId in selection"));
        }
        let time_ms = int(kf, "timeMs", 0, u64::MAX)?;
        if time_ms >= span_ms {
            return Err(invalid(
                "keyframe timeMs must be inside the cropped source range",
            ));
        }
        let points_raw = field(kf, "points")?
            .as_array()
            .ok_or_else(|| invalid("keyframe.points must be an array"))?;
        let points: Vec<SelPoint> = points_raw
            .iter()
            .map(parse_point)
            .collect::<Result<_, _>>()?;
        let box_ = match kf.get("box") {
            None | Some(Value::Null) => None,
            Some(b) => Some(parse_box(b)?),
        };
        if points.is_empty() && box_.is_none() {
            return Err(invalid("each keyframe needs at least one point or a box"));
        }
        let has_positive = points.iter().any(|p| p.label == PointLabel::Positive);
        if !has_positive && box_.is_none() {
            return Err(invalid("each keyframe needs a positive point or a box"));
        }
        for p in &points {
            if p.x >= frame_geometry.width as f64 || p.y >= frame_geometry.height as f64 {
                return Err(invalid("a selection point lies outside frameGeometry"));
            }
        }
        if let Some(b) = &box_ {
            if b.x + b.width > frame_geometry.width as f64
                || b.y + b.height > frame_geometry.height as f64
            {
                return Err(invalid("a selection box lies outside frameGeometry"));
            }
        }
        let (frame_index, resolved_time_ms) = match &timing {
            FrameTiming::Structure { .. } => (keyframes.len() as u64, time_ms),
            FrameTiming::Constant { fps, .. } => resolve_frame_index(time_ms, *fps, frame_count)?,
            FrameTiming::Stamps { stamps, .. } => resolve_frame_at_stamps(time_ms, stamps)?,
        };
        if frame_owners.insert(frame_index, keyframe_id.clone()).is_some() {
            return Err(Failure::new(
                "INVALID_INPUT",
                "two keyframes map to the same sampling frame",
            ));
        }
        keyframes.push(Keyframe {
            keyframe_id,
            time_ms,
            points,
            box_,
            frame_index,
            resolved_time_ms,
        });
    }
    keyframes.sort_by_key(|k| k.frame_index);
    Ok(Selection {
        target_id: text(v, "targetId")?,
        revision: text(v, "revision")?,
        frame_geometry,
        keyframes,
        selection_digest: text(v, "selectionDigest")?,
    })
}

pub fn parse_tracking(input: &Value) -> Result<TrackingRequest, Failure> {
    for retired in ["source", "sourceRange", "width", "height", "fps"] {
        if input.get(retired).is_some() {
            return Err(invalid(
                "tracking input uses the public sourceIdentity contract",
            ));
        }
    }
    let source = field(input, "sourceIdentity")?;
    let source_digest = text(source, "contentDigest")?;
    if source_digest == "sha256:unknown" || !source_digest.starts_with("sha256:") {
        return Err(invalid("source content digest must be the prepared sha256"));
    }
    let range = field(input, "range")?;
    if field(source, "selectedRange")? != range {
        return Err(invalid("range must equal sourceIdentity.selectedRange"));
    }
    let start_ms = int(range, "startMs", 0, u64::MAX)?;
    let end_ms = int(range, "endMs", 1, u64::MAX)?;
    if end_ms <= start_ms {
        return Err(invalid("range.endMs must be greater than startMs"));
    }
    let span = end_ms - start_ms;
    let selection = parse_selection_timed(
        field(input, "selection")?,
        FrameTiming::Structure { span_ms: span },
    )?;
    let direction = parse_direction(field(input, "direction")?)?;
    let anchor_keyframe_id = text(input, "anchorKeyframeId")?;
    if !selection
        .keyframes
        .iter()
        .any(|k| k.keyframe_id == anchor_keyframe_id)
    {
        return Err(invalid("anchorKeyframeId must name a keyframe in selection"));
    }
    let cov = field(input, "requestedCoverage")?;
    let coverage_start_ms = int(cov, "startMs", 0, u64::MAX)?;
    let coverage_end_ms = int(cov, "endMs", 0, u64::MAX)?;
    if coverage_end_ms <= coverage_start_ms {
        return Err(invalid("requestedCoverage.endMs must be greater than startMs"));
    }
    if coverage_start_ms < start_ms || coverage_end_ms > end_ms {
        return Err(invalid(
            "requestedCoverage must lie inside the source range span",
        ));
    }
    for forbidden in ["samModel", "comfyPath", "maskPath", "serverPath", "uploadRef"] {
        if input.get(forbidden).is_some() {
            return Err(invalid(
                "model-private media fields are not allowed in SDK params",
            ));
        }
    }
    Ok(TrackingRequest {
        source_media_id: text(source, "mediaId")?,
        start_ms,
        end_ms,
        selection,
        direction,
        anchor_keyframe_id,
        coverage_start_ms,
        coverage_end_ms,
        request_digest: text(input, "requestDigest")?,
        source_digest,
        width: 0,
        height: 0,
        fps: 0,
        sampled_frames: 0,
        frame_times_ms: Vec::new(),
    })
}

/// Package-internal sampling. Public requests do not carry width, height, or fps.
/// Size is the probed frame rounded down to a multiple of 16 (minimum 16). Fps is the probed rate.
pub fn apply_source_sampling(
    input: &Value,
    probed_width: u64,
    probed_height: u64,
    probed_fps: u64,
) -> Result<TrackingRequest, Failure> {
    if probed_width == 0 || probed_height == 0 || probed_fps == 0 {
        return Err(invalid("source video dimensions or frame rate are unavailable"));
    }
    let width = (probed_width / 16) * 16;
    let height = (probed_height / 16) * 16;
    if width < 16 || height < 16 {
        return Err(invalid("source video is smaller than the internal sampling grid"));
    }
    let mut tr = parse_tracking(input)?;
    let span = tr.end_ms - tr.start_ms;
    tr.selection = parse_selection(field(input, "selection")?, probed_fps, span)?;
    tr.width = width;
    tr.height = height;
    tr.fps = probed_fps;
    let span = tr.end_ms - tr.start_ms;
    tr.sampled_frames = span.saturating_mul(probed_fps) / 1000;
    if tr.sampled_frames == 0 {
        return Err(invalid("sourceRange is shorter than one frame at the requested fps"));
    }
    tr.frame_times_ms = (0..tr.sampled_frames).map(|i| i * 1000 / probed_fps).collect();
    Ok(tr)
}

/// Same size grid as `apply_source_sampling`, using the broker's actual frame timestamps.
pub fn apply_source_sampling_at_frames(
    input: &Value,
    probed_width: u64,
    probed_height: u64,
    stamps: &[u64],
) -> Result<TrackingRequest, Failure> {
    if probed_width == 0 || probed_height == 0 || stamps.is_empty() {
        return Err(invalid("source video dimensions or frame timestamps are unavailable"));
    }
    let width = (probed_width / 16) * 16;
    let height = (probed_height / 16) * 16;
    if width < 16 || height < 16 {
        return Err(invalid("source video is smaller than the internal sampling grid"));
    }
    let mut tr = parse_tracking(input)?;
    let span = tr.end_ms - tr.start_ms;
    tr.selection = parse_selection_timed(
        field(input, "selection")?,
        FrameTiming::Stamps { stamps, span_ms: span },
    )?;
    tr.width = width;
    tr.height = height;
    tr.fps = 0;
    tr.sampled_frames = stamps.len() as u64;
    tr.frame_times_ms = stamps.to_vec();
    Ok(tr)
}

pub fn parse_mask_artifact(v: &Value) -> Result<MaskArtifactRef, Failure> {
    Ok(MaskArtifactRef {
        artifact_id: text(v, "artifactId")?,
        digest: text(v, "digest")?,
    })
}

pub fn encode_keyframes_json(keyframes: &[Keyframe]) -> String {
    let mut items = Vec::new();
    for kf in keyframes {
        let mut pts = Vec::new();
        for p in &kf.points {
            let label = match p.label {
                PointLabel::Positive => 1,
                PointLabel::Negative => 0,
            };
            pts.push(format!(
                "{{\"pointId\":\"{}\",\"x\":{},\"y\":{},\"label\":{}}}",
                p.point_id, p.x, p.y, label
            ));
        }
        let box_json = match &kf.box_ {
            Some(b) => format!(
                ",\"box\":{{\"boxId\":\"{}\",\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}",
                b.box_id, b.x, b.y, b.width, b.height
            ),
            None => String::new(),
        };
        items.push(format!(
            "{{\"keyframeId\":\"{}\",\"frameIndex\":{},\"timeMs\":{},\"points\":[{}]{}}}",
            kf.keyframe_id,
            kf.frame_index,
            kf.time_ms,
            pts.join(","),
            box_json
        ));
    }
    format!("[{}]", items.join(","))
}

pub fn positive_points(kf: &Keyframe) -> Vec<Point> {
    kf.points
        .iter()
        .filter(|p| p.label == PointLabel::Positive)
        .map(|p| Point { x: p.x, y: p.y })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_selection() -> Value {
        json!({
            "schemaVersion": 1,
            "targetId": "t1",
            "revision": "r1",
            "frameGeometry": {"width": 1920, "height": 1080, "rotation": 0},
            "keyframes": [{
                "keyframeId": "kf0",
                "timeMs": 2000,
                "points": [{"pointId": "p0", "x": 960.0, "y": 540.0, "label": "positive"}]
            }],
            "selectionDigest": "digest-sel"
        })
    }

    #[test]
    fn parses_tracking_and_resolves_frames() {
        let input = json!({
            "sourceIdentity": {
                "mediaId": "m-src",
                "contentDigest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "selectedRange": {"startMs": 0, "endMs": 5000}
            },
            "range": {"startMs": 0, "endMs": 5000},
            "selection": sample_selection(),
            "direction": "both",
            "anchorKeyframeId": "kf0",
            "requestedCoverage": {"startMs": 0, "endMs": 5000},
            "requestDigest": "digest-req"
        });
        let t = apply_source_sampling(&input, 640, 640, 24).unwrap();
        assert_eq!(t.direction, Direction::Both);
        assert_eq!(t.width, 640);
        assert_eq!(t.selection.keyframes[0].frame_index, 48);
        assert_eq!(t.selection.keyframes[0].resolved_time_ms, 2000);
        assert!(parse_tracking(&json!({"source":{"mediaId":"m"},"sourceRange":{"mediaId":"m","startMs":0,"endMs":1}})).is_err());
    }

    #[test]
    fn rejects_sampling_frame_collision() {
        let mut sel = sample_selection();
        sel["keyframes"] = json!([
            {"keyframeId": "kfA", "timeMs": 1000, "points": [{"pointId":"a","x":1.0,"y":1.0,"label":"positive"}]},
            {"keyframeId": "kfB", "timeMs": 1020, "points": [{"pointId":"b","x":2.0,"y":2.0,"label":"positive"}]}
        ]);
        let err = parse_selection(&sel, 24, 5000).unwrap_err();
        assert_eq!(err.message, "two keyframes map to the same sampling frame");
    }

    #[test]
    fn irregular_timestamps_map_without_a_constant_fps() {
        let mut sel = sample_selection();
        sel["keyframes"] = json!([
            {"keyframeId": "kfA", "timeMs": 40, "points": [{"pointId":"a","x":1.0,"y":1.0,"label":"positive"}]},
            {"keyframeId": "kfB", "timeMs": 180, "points": [{"pointId":"b","x":2.0,"y":2.0,"label":"positive"}]}
        ]);
        let input = json!({
            "sourceIdentity": {
                "mediaId": "m-src",
                "contentDigest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "selectedRange": {"startMs": 0, "endMs": 5000}
            },
            "range": {"startMs": 0, "endMs": 5000},
            "selection": sel,
            "direction": "towardEnd",
            "anchorKeyframeId": "kfA",
            "requestedCoverage": {"startMs": 40, "endMs": 5000},
            "requestDigest": "d"
        });
        let stamps = [0u64, 40, 100, 180, 400];
        let t = apply_source_sampling_at_frames(&input, 640, 480, &stamps).unwrap();
        assert_eq!(t.sampled_frames, 5);
        assert_eq!(t.fps, 0);
        assert_eq!(t.selection.keyframes[0].frame_index, 1);
        assert_eq!(t.selection.keyframes[0].resolved_time_ms, 40);
        assert_eq!(t.selection.keyframes[1].frame_index, 3);
        assert_eq!(t.selection.keyframes[1].resolved_time_ms, 180);
    }

    #[test]
    fn rejects_private_media_fields() {
        let input = json!({
            "sourceIdentity": {
                "mediaId": "m-src",
                "contentDigest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "selectedRange": {"startMs": 0, "endMs": 5000}
            },
            "range": {"startMs": 0, "endMs": 5000},
            "selection": sample_selection(),
            "direction": "towardEnd",
            "anchorKeyframeId": "kf0",
            "requestedCoverage": {"startMs": 2000, "endMs": 5000},
            "requestDigest": "d",
            "maskPath": "/secret"
        });
        assert!(parse_tracking(&input).is_err());
    }

    #[test]
    fn public_profile_fixture_parses_and_old_source_fields_are_rejected() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "L:/00Dev/bgxiong-ai-story/packages/vendor-adapter-contract/v2/profiles/video-target-keyframes-v1/fixtures/receipts.json"
        ))
        .unwrap();
        let input = &fixtures["valid"]["trackingInput"];
        let parsed = parse_tracking(input).expect("F04 public trackingInput must parse");
        assert_eq!(parsed.source_media_id, "media-vtk-1");
        assert_eq!(parsed.direction, Direction::Both);
        assert_eq!(parsed.anchor_keyframe_id, "kf0");
        let mut stale = input.clone();
        stale["source"] = json!({"mediaId": "media-vtk-1"});
        stale["sourceRange"] = json!({"mediaId": "media-vtk-1", "startMs": 0, "endMs": 5000});
        stale["width"] = json!(1280);
        stale["height"] = json!(720);
        stale["fps"] = json!(24);
        assert!(parse_tracking(&stale).is_err());
    }
}
