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
    let (sw, sh, tw, th) = (src_w as f64, src_h as f64, w as f64, h as f64);
    let (cw, ch) = if sw * th > sh * tw {
        (sh * tw / th, sh)
    } else {
        (sw, sw * th / tw)
    };
    let (x0, y0) = ((sw - cw) / 2.0, (sh - ch) / 2.0);
    let scale = tw / cw;
    points
        .iter()
        .map(|p| {
            if p.x >= sw || p.y >= sh {
                return Err(Failure::new(
                    "INVALID_INPUT",
                    "A target point lies outside the source frame",
                ));
            }
            let (x, y) = ((p.x - x0) * scale, (p.y - y0) * scale);
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
fn crop_scale(src_w: u64, src_h: u64, w: u64, h: u64) -> Result<(f64, f64, f64, f64, f64), Failure> {
    if src_w == 0 || src_h == 0 {
        return Err(Failure::new(
            "INVALID_INPUT",
            "Source video dimensions are unknown",
        ));
    }
    let (sw, sh, tw, th) = (src_w as f64, src_h as f64, w as f64, h as f64);
    let (cw, ch) = if sw * th > sh * tw {
        (sh * tw / th, sh)
    } else {
        (sw, sw * th / tw)
    };
    let (x0, y0) = ((sw - cw) / 2.0, (sh - ch) / 2.0);
    let scale = tw / cw;
    Ok((x0, y0, scale, tw, th))
}

fn map_xy(x: f64, y: f64, x0: f64, y0: f64, scale: f64, tw: f64, th: f64, sw: f64, sh: f64) -> Result<(f64, f64), Failure> {
    if x >= sw || y >= sh || x < 0.0 || y < 0.0 {
        return Err(Failure::new(
            "INVALID_INPUT",
            "A target point lies outside the source frame",
        ));
    }
    let (mx, my) = ((x - x0) * scale, (y - y0) * scale);
    if mx < 0.0 || my < 0.0 || mx >= tw || my >= th {
        return Err(Failure::new(
            "INVALID_INPUT",
            "A target point is cropped away by the output aspect ratio",
        ));
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
    let (x0, y0, scale, tw, th) = crop_scale(src_w, src_h, w, h)?;
    let sw = src_w as f64;
    let sh = src_h as f64;
    let mut out = Vec::new();
    for kf in keyframes {
        let mut points = Vec::new();
        for p in &kf.points {
            let (x, y) = map_xy(p.x, p.y, x0, y0, scale, tw, th, sw, sh)?;
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
                let (x1, y1) = map_xy(b.x, b.y, x0, y0, scale, tw, th, sw, sh)?;
                let (x2, y2) = map_xy(
                    b.x + b.width,
                    b.y + b.height,
                    x0,
                    y0,
                    scale,
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
        let mapped = scale_keyframes(&[kf], 1920, 1080, 640, 640).unwrap();
        assert_eq!((mapped[0].points[0].x, mapped[0].points[0].y), (320.0, 320.0));
        let b = mapped[0].box_.as_ref().unwrap();
        assert!(b.width > 0.0 && b.height > 0.0);
        assert!(b.x >= 0.0 && b.y >= 0.0 && b.x + b.width <= 640.0 && b.y + b.height <= 640.0);
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
