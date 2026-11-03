//! Template identity and package-owned encodings (models, point coordinates).
use alloc::{format, string::String, vec::Vec};

use crate::{parameters::Point, Failure};

/// Certified author canvas this runtime graph corresponds to
/// (`ComfyUI-API/wan2.2/video_wan2_2_14B_animate.json`).
pub const TEMPLATE_ID: &str = "wan2.2/video_wan2_2_14B_animate";
pub const TEMPLATE_SHA256: &str =
    "80b11e45d14419579126ff9861f523120d9519412a6dae73f34982c6dd78adf5";

/// Resources are fixed by the template version; changing them is a new template/package version.
pub struct Models {
    pub unet: &'static str,
    pub distill_lora: &'static str,
    pub relight_lora: &'static str,
    pub text_encoder: &'static str,
    pub vae: &'static str,
    pub clip_vision: &'static str,
    pub sam2: &'static str,
    pub bbox_detector: &'static str,
    pub pose_estimator: &'static str,
}
pub const MODELS: Models = Models {
    unet: "Wan2_2-Animate-14B_fp8_e4m3fn_scaled_KJ.safetensors",
    distill_lora: "lightx2v_I2V_14B_480p_cfg_step_distill_rank64_bf16.safetensors",
    relight_lora: "WanAnimate_relight_lora_fp16.safetensors",
    text_encoder: "umt5_xxl_fp8_e4m3fn_scaled.safetensors",
    vae: "wan_2.1_vae.safetensors",
    clip_vision: "clip_vision_h.safetensors",
    sam2: "sam2_hiera_base_plus.safetensors",
    bbox_detector: "yolox_l.onnx",
    pose_estimator: "dw-ll_ucoco_384_bs5.torchscript.pt",
};

/// Maps source-frame pixel points into the `ImageScale(crop=center)` frame the
/// segmentation sees: crop the source to the target aspect, then scale.
pub fn scale_points(
    points: &[Point],
    src_w: u64,
    src_h: u64,
    w: u64,
    h: u64,
) -> Result<Vec<Point>, Failure> {
    if src_w == 0 || src_h == 0 {
        return Err(Failure::new(
            "INVALID_INPUT",
            "Source video dimensions are unknown",
        ));
    }
    let (x0, y0, sx, sy, tw, th) = crop_scale(src_w, src_h, w, h)?;
    let (sw, sh) = (src_w as f64, src_h as f64);
    points
        .iter()
        .map(|p| {
            if p.x >= sw || p.y >= sh {
                return Err(Failure::new(
                    "INVALID_INPUT",
                    "A target point lies outside the source frame",
                ));
            }
            let (x, y) = ((p.x - x0) * sx, (p.y - y0) * sy);
            if x < 0.0 || y < 0.0 || x >= tw || y >= th {
                return Err(Failure::new(
                    "INVALID_INPUT",
                    "A target point is cropped away by the output aspect ratio",
                ));
            }
            Ok(Point { x, y })
        })
        .collect()
}

/// Sam2Segmentation `coordinates_positive` string encoding.
pub fn encode_points(points: &[Point]) -> String {
    let items: Vec<String> = points
        .iter()
        .map(|p| format!("{{\"x\":{},\"y\":{}}}", p.x, p.y))
        .collect();
    format!("[{}]", items.join(","))
}



use crate::target_keyframes::{Keyframe, SelBox, SelPoint};

/// Center-crop + scale transform shared by points and boxes (source → gen frame).
// Python round on nonnegative margins: ties to even, usable with no_std.
fn round_margin(value: f64) -> f64 {
    let lower = value as u64;
    let fraction = value - lower as f64;
    (lower + u64::from(fraction > 0.5 || (fraction == 0.5 && lower % 2 == 1))) as f64
}
fn crop_scale(src_w: u64, src_h: u64, w: u64, h: u64) -> Result<(f64, f64, f64, f64, f64, f64), Failure> {
    if src_w == 0 || src_h == 0 || w == 0 || h == 0 {
        return Err(Failure::new("INVALID_INPUT", "Video dimensions must be positive"));
    }
    let (sw, sh, tw, th) = (src_w as f64, src_h as f64, w as f64, h as f64);
    let (old_aspect, new_aspect) = (sw / sh, tw / th);
    let (x0, y0) = if old_aspect > new_aspect {
        (round_margin((sw - sw * (new_aspect / old_aspect)) / 2.0), 0.0)
    } else if old_aspect < new_aspect {
        (0.0, round_margin((sh - sh * (old_aspect / new_aspect)) / 2.0))
    } else { (0.0, 0.0) };
    let (cw, ch) = (sw - 2.0 * x0, sh - 2.0 * y0);
    if cw <= 0.0 || ch <= 0.0 {
        return Err(Failure::new("INVALID_INPUT", "Center crop has no pixels"));
    }
    Ok((x0, y0, tw / cw, th / ch, tw, th))
}

fn map_xy(x: f64, y: f64, x0: f64, y0: f64, sx: f64, sy: f64, tw: f64, th: f64, sw: f64, sh: f64) -> Result<(f64, f64), Failure> {
    if x >= sw || y >= sh || x < 0.0 || y < 0.0 {
        return Err(Failure::new(
            "INVALID_INPUT",
            "A target point lies outside the source frame",
        ));
    }
    let (mx, my) = ((x - x0) * sx, (y - y0) * sy);
    if mx < 0.0 || my < 0.0 || mx >= tw || my >= th {
        return Err(Failure::new(
            "INVALID_INPUT",
            "A target point is cropped away by the output aspect ratio",
        ));
    }
    Ok((mx, my))
}

// A rectangle's far corner is an edge, not a point sample: equality is legal.
fn map_box_end(x: f64, y: f64, x0: f64, y0: f64, sx: f64, sy: f64, tw: f64, th: f64, sw: f64, sh: f64) -> Result<(f64, f64), Failure> {
    if x > sw || y > sh || x < 0.0 || y < 0.0 {
        return Err(Failure::new("INVALID_INPUT", "A selection box lies outside the source frame"));
    }
    let (mx, my) = ((x - x0) * sx, (y - y0) * sy);
    if mx < 0.0 || my < 0.0 || mx > tw || my > th {
        return Err(Failure::new("INVALID_INPUT", "A selection box is cropped away by the output aspect ratio"));
    }
    Ok((mx, my))
}

/// Map selection keyframes' points/boxes from source pixels into the generation frame.
/// Reference-image padding is intentionally not part of this map.
pub fn scale_keyframes(
    keyframes: &[Keyframe],
    src_w: u64,
    src_h: u64,
    w: u64,
    h: u64,
) -> Result<Vec<Keyframe>, Failure> {
    let (x0, y0, sx, sy, tw, th) = crop_scale(src_w, src_h, w, h)?;
    let sw = src_w as f64;
    let sh = src_h as f64;
    let mut out = Vec::new();
    for kf in keyframes {
        let mut points = Vec::new();
        for p in &kf.points {
            let (x, y) = map_xy(p.x, p.y, x0, y0, sx, sy, tw, th, sw, sh)?;
            points.push(SelPoint {
                point_id: p.point_id.clone(),
                x,
                y,
                label: p.label,
            });
        }
        let box_ = match &kf.box_ {
            None => None,
            Some(b) => {
                let (x1, y1) = map_xy(b.x, b.y, x0, y0, sx, sy, tw, th, sw, sh)?;
                let (x2, y2) = map_box_end(
                    b.x + b.width,
                    b.y + b.height,
                    x0,
                    y0,
                    sx,
                    sy,
                    tw,
                    th,
                    sw,
                    sh,
                )?;
                let width = x2 - x1;
                let height = y2 - y1;
                if width <= 0.0 || height <= 0.0 {
                    return Err(Failure::new(
                        "INVALID_INPUT",
                        "A selection box collapses after mapping",
                    ));
                }
                if x1 < 0.0 || y1 < 0.0 || x2 > tw || y2 > th {
                    return Err(Failure::new(
                        "INVALID_INPUT",
                        "A selection box is cropped away by the output aspect ratio",
                    ));
                }
                Some(SelBox {
                    box_id: b.box_id.clone(),
                    x: x1,
                    y: y1,
                    width,
                    height,
                })
            }
        };
        out.push(Keyframe {
            keyframe_id: kf.keyframe_id.clone(),
            time_ms: kf.time_ms,
            points,
            box_,
            frame_index: kf.frame_index,
            resolved_time_ms: kf.resolved_time_ms,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_crop_mapping() {
        // Actual common_upscale CPU observation: crop 481x257 at (0,7).
        let odd = scale_points(&[Point { x: 100.0, y: 100.0 }], 481, 271, 480, 256).unwrap();
        assert!((odd[0].y - 92.63813229571984).abs() < 1e-10);
        assert!((odd[0].x - 99.79209979209979).abs() < 1e-10);
        assert_eq!(round_margin(6.5), 6.0);
        assert_eq!(round_margin(7.5), 8.0);

        // 1920x1080 -> 640x640: crop 1080x1080 from x0=420, scale 640/1080.
        let p = scale_points(&[Point { x: 960.0, y: 540.0 }], 1920, 1080, 640, 640)
            .ok()
            .unwrap();
        assert_eq!((p[0].x, p[0].y), (320.0, 320.0));
        assert!(scale_points(&[Point { x: 100.0, y: 540.0 }], 1920, 1080, 640, 640).is_err());
        assert!(scale_points(&[Point { x: 1920.0, y: 0.0 }], 1920, 1080, 1920, 1088).is_err());
        assert_eq!(encode_points(&p), "[{\"x\":320,\"y\":320}]");
    }

    #[test]
    fn scale_keyframes_maps_box_and_points() {
        use crate::target_keyframes::{Keyframe, PointLabel, SelBox, SelPoint};
        let kf = Keyframe {
            keyframe_id: "kf0".into(),
            time_ms: 0,
            points: alloc::vec![SelPoint {
                point_id: "p0".into(),
                x: 960.0,
                y: 540.0,
                label: PointLabel::Positive,
            }],
            box_: Some(SelBox {
                box_id: "b0".into(),
                x: 860.0,
                y: 440.0,
                width: 200.0,
                height: 200.0,
            }),
            frame_index: 0,
            resolved_time_ms: 0,
        };
        let mapped = scale_keyframes(&[kf.clone()], 1920, 1080, 640, 640).unwrap();
        assert_eq!((mapped[0].points[0].x, mapped[0].points[0].y), (320.0, 320.0));
        let b = mapped[0].box_.as_ref().unwrap();
        assert!(b.width > 0.0 && b.height > 0.0);
        assert!(b.x >= 0.0 && b.y >= 0.0 && b.x + b.width <= 640.0 && b.y + b.height <= 640.0);
        let mut odd = kf;
        odd.points[0].x = 100.0; odd.points[0].y = 100.0;
        odd.points.push(SelPoint { point_id: "neg".into(), x: 100.0, y: 100.0, label: PointLabel::Negative });
        odd.box_ = Some(SelBox { box_id: "odd".into(), x: 100.0, y: 100.0, width: 50.0, height: 50.0 });
        let transformed = scale_keyframes(&[odd], 481, 271, 480, 256).unwrap();
        assert_eq!(transformed[0].points[1].label, PointLabel::Negative);
        for p in &transformed[0].points {
            assert!((p.y - 92.63813229571984).abs() < 1e-10);
        }
        let odd_box = transformed[0].box_.as_ref().unwrap();
        assert!((odd_box.y - 92.63813229571984).abs() < 1e-10);
        assert!((odd_box.height - 50.0 * 256.0 / 257.0).abs() < 1e-10);
        // Rectangles have closed right/bottom boundaries; point coordinates remain half-open.
        let mut boundary = transformed[0].clone();
        boundary.points.clear();
        boundary.box_ = Some(SelBox { box_id: "edge".into(), x: 0.0, y: 0.0, width: 480.0, height: 256.0 });
        let full = scale_keyframes(&[boundary.clone()], 480, 256, 480, 256).unwrap();
        assert_eq!(full[0].box_.as_ref().unwrap().width, 480.0);
        assert_eq!(full[0].box_.as_ref().unwrap().height, 256.0);
        boundary.box_.as_mut().unwrap().width = 480.1;
        assert!(scale_keyframes(&[boundary.clone()], 480, 256, 480, 256).is_err());
        boundary.box_.as_mut().unwrap().width = 480.0;
        boundary.points.push(SelPoint { point_id: "edge-point".into(), x: 480.0, y: 100.0, label: PointLabel::Positive });
        assert!(scale_keyframes(&[boundary.clone()], 480, 256, 480, 256).is_err());
        // A box ending exactly at the right/bottom crop wall remains valid.
        boundary.points.clear();
        boundary.box_ = Some(SelBox { box_id: "crop-edge".into(), x: 420.0, y: 0.0, width: 1080.0, height: 1080.0 });
        assert!(scale_keyframes(&[boundary], 1920, 1080, 640, 640).is_ok());
        // Cropped-away box rejected (far left outside center crop).
        let bad = Keyframe {
            keyframe_id: "kf1".into(),
            time_ms: 0,
            points: alloc::vec![],
            box_: Some(SelBox {
                box_id: "b1".into(),
                x: 0.0,
                y: 500.0,
                width: 50.0,
                height: 50.0,
            }),
            frame_index: 0,
            resolved_time_ms: 0,
        };
        assert!(scale_keyframes(&[bad], 1920, 1080, 640, 640).is_err());
    }
}
