//! Dynamic API graph: shared load/preprocess once, K generation groups chained by
//! the node-native continuation outputs, one final output trimmed to N.
use alloc::string::{String, ToString};
use serde_json::{json, Map, Value};

use crate::{
    mapping::{encode_points, MODELS},
    parameters::{Point, Request},
    planner::{segment_node, Plan},
    target_keyframes,
};

pub const OUTPUT_NODE: &str = "out.save";

pub struct Inputs<'a> {
    /// Server file reference returned by the upload of the prepared source video.
    pub source_file: &'a str,
    /// Server file reference returned by the upload of the replacement image.
    pub replacement_file: &'a str,
    /// Target points already mapped into the generation frame (character replace only).
    pub points: &'a [Point],
}

fn link(node: &str, slot: u64) -> Value {
    json!([node, slot])
}

pub fn build(r: &Request, plan: &Plan, inputs: &Inputs) -> Value {
    let a = &r.advanced;
    let mut g = Map::new();
    let mut put = |id: &str, class: &str, inputs: Value| {
        g.insert(
            id.to_string(),
            json!({"class_type": class, "inputs": inputs}),
        );
    };
    put(
        "unet",
        "UNETLoader",
        json!({"unet_name": MODELS.unet, "weight_dtype": "default"}),
    );
    put(
        "lora.distill",
        "LoraLoaderModelOnly",
        json!({"model": link("unet", 0), "lora_name": MODELS.distill_lora, "strength_model": a.distill_lora_strength}),
    );
    put(
        "lora.relight",
        "LoraLoaderModelOnly",
        json!({"model": link("lora.distill", 0), "lora_name": MODELS.relight_lora, "strength_model": a.relight_lora_strength}),
    );
    put(
        "shift",
        "ModelSamplingSD3",
        json!({"model": link("lora.relight", 0), "shift": a.sampling_shift}),
    );
    put(
        "clip",
        "CLIPLoader",
        json!({"clip_name": MODELS.text_encoder, "type": "wan", "device": "default"}),
    );
    put("vae", "VAELoader", json!({"vae_name": MODELS.vae}));
    put(
        "clipvision",
        "CLIPVisionLoader",
        json!({"clip_name": MODELS.clip_vision}),
    );
    put(
        "ref",
        "LoadImage",
        json!({"image": inputs.replacement_file}),
    );
    put(
        "ref.encode",
        "CLIPVisionEncode",
        json!({"clip_vision": link("clipvision", 0), "image": link("ref", 0), "crop": "none"}),
    );
    put(
        "prompt.pos",
        "CLIPTextEncode",
        json!({"clip": link("clip", 0), "text": r.prompt}),
    );
    put(
        "prompt.neg",
        "CLIPTextEncode",
        json!({"clip": link("clip", 0), "text": r.negative_prompt}),
    );
    put("src", "LoadVideo", json!({"file": inputs.source_file}));
    put(
        "src.parts",
        "GetVideoComponents",
        json!({"video": link("src", 0)}),
    );
    put(
        "src.scaled",
        "ImageScale",
        json!({"image": link("src.parts", 0), "upscale_method": "lanczos", "width": r.width, "height": r.height, "crop": "center"}),
    );
    // Character replace only: Move must leave these inputs absent (bypass is not enough),
    // so the whole segmentation/background branch is not emitted at all.
    let replace = r.replace.as_ref();
    // Mix remaps the scaled source onto the generation timeline once; every per-frame
    // preprocess (pose/face/SAM/background) consumes that unique sequence. Move keeps
    // the original scaled frames (no SAM2 / no sequence).
    let frame_images = if replace.is_some() || r.mask_artifact.is_some() {
        put(
            "src.seq",
            "BGXAnimateFrameSequence",
            json!({
                "images": link("src.scaled", 0),
                // GetVideoComponents fps output is FLOAT (verified from Comfy core node).
                "source_fps": link("src.parts", 2),
                "target_fps": r.fps,
                "frame_count": plan.frames
            }),
        );
        "src.seq"
    } else {
        "src.scaled"
    };
    put(
        "pose.res",
        "PixelPerfectResolution",
        json!({"original_image": link("src.parts", 0), "image_gen_width": r.width, "image_gen_height": r.height, "resize_mode": "Just Resize"}),
    );
    for (id, hand_body, face) in [("face", "disable", "enable"), ("pose", "enable", "disable")] {
        put(
            id,
            "DWPreprocessor",
            json!({"image": link(frame_images, 0), "resolution": link("pose.res", 0), "detect_hand": hand_body, "detect_body": hand_body, "detect_face": face,
            "bbox_detector": MODELS.bbox_detector, "pose_estimator": MODELS.pose_estimator, "scale_stick_for_xinsr_cn": "disable"}),
        );
    }
    if let Some(art) = r.mask_artifact.as_ref() {
        put(
            "sam",
            "BGXAnimateMaskArtifactLoad",
            json!({
                "artifact_id": art.artifact_id,
                "digest": art.digest,
                "expected_frame_count": plan.frames,
                "target_fps": r.fps,
                "out_width": r.width,
                "out_height": r.height
            }),
        );
        let grow = r.mask_grow_pixels.unwrap_or(0);
        put(
            "mask.grow",
            "GrowMask",
            json!({"mask": link("sam", 0), "expand": grow, "tapered_corners": true}),
        );
        put(
            "mask.block",
            "BlockifyMask",
            json!({"masks": link("mask.grow", 0), "block_size": 32, "device": "cpu"}),
        );
        put(
            "background",
            "DrawMaskOnImage",
            json!({"image": link(frame_images, 0), "mask": link("mask.block", 0), "color": "0, 0, 0", "device": "gpu"}),
        );
    } else if let Some(rep) = replace {
        put(
            "sam.model",
            "DownloadAndLoadSAM2Model",
            json!({"model": MODELS.sam2, "segmentor": "video", "device": "cuda", "precision": "fp16"}),
        );
        put(
            "sam",
            "BGXAnimateBidirectionalMask",
            json!({
                "sam2_model": link("sam.model", 0),
                "image": link(frame_images, 0),
                "coordinates_positive": encode_points(inputs.points),
                "frame_time_ms": rep.frame_time_ms,
                "target_fps": r.fps,
                "frame_count": plan.frames,
                "keep_model_loaded": true
            }),
        );
        put(
            "mask.grow",
            "GrowMask",
            json!({"mask": link("sam", 0), "expand": rep.mask_grow_pixels, "tapered_corners": true}),
        );
        put(
            "mask.block",
            "BlockifyMask",
            json!({"masks": link("mask.grow", 0), "block_size": 32, "device": "cpu"}),
        );
        put(
            "background",
            "DrawMaskOnImage",
            json!({"image": link(frame_images, 0), "mask": link("mask.block", 0), "color": "0, 0, 0", "device": "gpu"}),
        );
    }

    let mut frames = String::new();
    for s in &plan.segments {
        let k = s.index;
        let (anim, sampler, trim, decode) = (
            segment_node(k, "animate"),
            segment_node(k, "sampler"),
            segment_node(k, "trim"),
            segment_node(k, "decode"),
        );
        let mut animate = json!({"positive": link("prompt.pos", 0), "negative": link("prompt.neg", 0), "vae": link("vae", 0),
            "clip_vision_output": link("ref.encode", 0), "reference_image": link("ref", 0), "face_video": link("face", 0), "pose_video": link("pose", 0),
            "width": r.width, "height": r.height,
            "length": a.window_frames, "batch_size": 1, "continue_motion_max_frames": a.overlap_frames, "video_frame_offset": 0});
        if replace.is_some() || r.mask_artifact.is_some() {
            animate["background_video"] = link("background", 0);
            animate["character_mask"] = link("mask.block", 0);
        }
        if k > 0 {
            animate["continue_motion"] = link(&frames, 0);
            animate["video_frame_offset"] = link(&segment_node(k - 1, "animate"), 5);
        }
        put(&anim, "WanAnimateToVideo", animate);
        put(
            &sampler,
            "KSampler",
            json!({"model": link("shift", 0), "positive": link(&anim, 0), "negative": link(&anim, 1), "latent_image": link(&anim, 2),
            "seed": s.seed, "steps": a.steps, "cfg": a.cfg, "sampler_name": a.sampler, "scheduler": a.scheduler, "denoise": 1}),
        );
        put(
            &trim,
            "TrimVideoLatent",
            json!({"samples": link(&sampler, 0), "trim_amount": link(&anim, 3)}),
        );
        put(
            &decode,
            "VAEDecode",
            json!({"samples": link(&trim, 0), "vae": link("vae", 0)}),
        );
        frames = if k == 0 {
            decode
        } else {
            let fresh = segment_node(k, "fresh");
            let joined = segment_node(k, "frames");
            put(
                &fresh,
                "ImageFromBatch",
                json!({"image": link(&decode, 0), "batch_index": link(&anim, 4), "length": 4096}),
            );
            put(
                &joined,
                "ImageBatch",
                json!({"image1": link(&frames, 0), "image2": link(&fresh, 0)}),
            );
            joined
        };
    }
    put(
        "out.frames",
        "ImageFromBatch",
        json!({"image": link(&frames, 0), "batch_index": 0, "length": plan.frames}),
    );
    let mut video = json!({"images": link("out.frames", 0), "fps": r.fps});
    if r.keep_audio {
        video["audio"] = link("src.parts", 1);
    }
    put("out.video", "CreateVideo", video);
    put(
        OUTPUT_NODE,
        "SaveVideo",
        json!({"video": link("out.video", 0), "filename_prefix": "video/wan22-animate", "format": "auto", "codec": "auto"}),
    );
    Value::Object(g)
}


/// Output node id for mask-only tracking graphs (history lookup).
pub const TRACKING_OUTPUT_NODE: &str = "track.export";

pub struct TrackingInputs<'a> {
    pub source_file: &'a str,
    pub keyframes_json: &'a str,
    pub frame_count: u64,
    pub request_id: &'a str,
    pub source_digest: &'a str,
    pub input_schema_digest: &'a str,
    pub mapped_keyframes: &'a [target_keyframes::Keyframe],
}

/// Mask-only tracking preview graph: no diffusion / WanAnimate / sampler nodes.
pub fn build_tracking(tr: &target_keyframes::TrackingRequest, inputs: &TrackingInputs<'_>) -> Value {
    let mut g = Map::new();
    let mut put = |id: &str, class: &str, inputs: Value| {
        g.insert(id.to_string(), json!({"class_type": class, "inputs": inputs}));
    };
    put("src", "LoadVideo", json!({"file": inputs.source_file}));
    put("src.parts", "GetVideoComponents", json!({"video": link("src", 0)}));
    put(
        "src.scaled",
        "ImageScale",
        json!({
            "image": link("src.parts", 0),
            "upscale_method": "lanczos",
            "width": tr.width,
            "height": tr.height,
            "crop": "center"
        }),
    );
    // Actual frame timestamps mean every decoded frame is a sample; only a constant rate resamples.
    let frames = if tr.fps == 0 {
        link("src.scaled", 0)
    } else {
        put(
            "src.seq",
            "BGXAnimateFrameSequence",
            json!({
                "images": link("src.scaled", 0),
                "source_fps": link("src.parts", 2),
                "target_fps": tr.fps,
                "frame_count": inputs.frame_count
            }),
        );
        link("src.seq", 0)
    };
    let frame_times_json =
        serde_json::to_string(&tr.frame_times_ms).unwrap_or_else(|_| String::from("[]"));
    put(
        "sam.model",
        "DownloadAndLoadSAM2Model",
        json!({"model": MODELS.sam2, "segmentor": "video", "device": "cuda", "precision": "fp16"}),
    );
    put(
        "sam.multi",
        "BGXAnimateMultiKeyframeMask",
        json!({
            "sam2_model": link("sam.model", 0),
            "image": frames,
            "keyframes_json": inputs.keyframes_json,
            "direction": tr.direction.as_str(),
            "frame_count": inputs.frame_count,
            "keep_model_loaded": true,
            "request_id": inputs.request_id,
            "selection_digest": tr.selection.selection_digest,
            "frame_times_json": frame_times_json,
            "span_ms": tr.end_ms - tr.start_ms
        }),
    );
    let mut resolved = alloc::vec::Vec::new();
    for kf in inputs.mapped_keyframes.iter() {
        resolved.push(json!({
            "keyframeId": kf.keyframe_id,
            "resolvedTimeMs": kf.resolved_time_ms,
            "frameIndex": kf.frame_index
        }));
    }
    let resolved_json = serde_json::to_string(&resolved).unwrap_or_else(|_| String::from("[]"));
    put(
        "track.export",
        "BGXAnimateMaskArtifactExport",
        json!({
            "masks": link("sam.multi", 0),
            "request_id": inputs.request_id,
            "selection_digest": tr.selection.selection_digest,
            "source_digest": inputs.source_digest,
            "direction": tr.direction.as_str(),
            "covered_indices_json": link("sam.multi", 1),
            "resolved_keyframes_json": resolved_json,
            "range_start_ms": tr.start_ms,
            "range_end_ms": tr.end_ms,
            "fps": tr.fps as f64,
            "frame_times_json": frame_times_json,
            "width": tr.width,
            "height": tr.height,
            "operation_id": crate::capability::TRACKING_OPERATION_ID,
            "operation_version": crate::capability::TRACKING_OPERATION_VERSION,
            "input_schema_digest": inputs.input_schema_digest
        }),
    );
    Value::Object(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        parameters::{
            parse,
            tests::{sample, sample_move},
            Mode,
        },
        planner::plan,
    };

    fn graph(end_ms: u64, audio: &str) -> (Value, Plan) {
        graph_for(sample(), Mode::Mix, end_ms, audio)
    }
    fn graph_for(mut v: Value, mode: Mode, end_ms: u64, audio: &str) -> (Value, Plan) {
        v["sourceRange"]["endMs"] = end_ms.into();
        v["audio"] = audio.into();
        v["width"] = 832.into();
        v["advanced"]["steps"] = 9.into();
        let r = parse(&v, mode).ok().unwrap();
        let p = plan(&r).ok().unwrap();
        let pts = [Point { x: 1.0, y: 2.0 }];
        (
            build(
                &r,
                &p,
                &Inputs {
                    source_file: "src.mp4",
                    replacement_file: "ref.png",
                    points: &pts,
                },
            ),
            p,
        )
    }

    #[test]
    fn multi_segment_graph_is_chained_and_uniform() {
        let (g, p) = graph(10_000, "source");
        assert_eq!(p.segments.len(), 4);
        let saves = g
            .as_object()
            .unwrap()
            .values()
            .filter(|n| n["class_type"] == "SaveVideo")
            .count();
        assert_eq!(saves, 1, "single final output");
        for k in 0..4 {
            let a = &g[segment_node(k, "animate")]["inputs"];
            assert_eq!(
                (a["width"].as_u64(), a["length"].as_u64()),
                (Some(832), Some(77))
            );
            assert_eq!(g[segment_node(k, "sampler")]["inputs"]["steps"], 9);
        }
        assert!(g["g0.animate"]["inputs"].get("continue_motion").is_none());
        assert_eq!(
            g["g1.animate"]["inputs"]["continue_motion"],
            json!(["g0.decode", 0])
        );
        assert_eq!(
            g["g2.animate"]["inputs"]["continue_motion"],
            json!(["g1.frames", 0])
        );
        assert_eq!(
            g["g3.animate"]["inputs"]["video_frame_offset"],
            json!(["g2.animate", 5])
        );
        assert_eq!(
            g["out.frames"]["inputs"],
            json!({"image": ["g3.frames", 0], "batch_index": 0, "length": 240})
        );
        assert_eq!(g["out.video"]["inputs"]["audio"], json!(["src.parts", 1]));
        // Every link targets an existing node.
        for n in g.as_object().unwrap().values() {
            for v in n["inputs"].as_object().unwrap().values() {
                if let Some(target) = v.as_array().and_then(|l| l[0].as_str()) {
                    assert!(g.get(target).is_some(), "dangling link to {target}");
                }
            }
        }
    }

    #[test]
    fn mix_and_move_graph_diff() {
        let (mix, _) = graph(10_000, "source");
        let (mov, _) = graph_for(sample_move(), Mode::Move, 10_000, "source");
        let keys = |g: &Value| {
            g.as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<alloc::collections::BTreeSet<_>>()
        };
        let only_mix: alloc::vec::Vec<_> = keys(&mix).difference(&keys(&mov)).cloned().collect();
        assert_eq!(
            only_mix,
            ["background", "mask.block", "mask.grow", "sam", "sam.model", "src.seq"]
        );
        assert!(keys(&mov).is_subset(&keys(&mix)));
        for k in 0..4 {
            let id = segment_node(k, "animate");
            let (m, v) = (&mix[&id]["inputs"], &mov[&id]["inputs"]);
            assert_eq!(m["background_video"], json!(["background", 0]));
            assert_eq!(m["character_mask"], json!(["mask.block", 0]));
            assert!(v.get("background_video").is_none() && v.get("character_mask").is_none());
            let mut stripped = m.clone();
            stripped.as_object_mut().unwrap().remove("background_video");
            stripped.as_object_mut().unwrap().remove("character_mask");
            assert_eq!(&stripped, v, "everything else is identical");
        }
    }

    #[test]
    fn single_segment_without_audio() {
        let (g, _) = graph(2_000, "none");
        assert_eq!(g["out.frames"]["inputs"]["image"], json!(["g0.decode", 0]));
        assert_eq!(g["out.frames"]["inputs"]["length"], 48);
        assert!(g["out.video"]["inputs"].get("audio").is_none());
        assert!(g.get("g1.animate").is_none());
    }

    #[test]
    fn mix_uses_unique_sequence_and_bidirectional_mask() {
        let (mix, p) = graph(10_000, "source");
        assert_eq!(mix["src.seq"]["class_type"], "BGXAnimateFrameSequence");
        assert_eq!(
            mix["src.seq"]["inputs"],
            json!({
                "images": ["src.scaled", 0],
                "source_fps": ["src.parts", 2],
                "target_fps": 24,
                "frame_count": p.frames
            })
        );
        assert_eq!(mix["sam"]["class_type"], "BGXAnimateBidirectionalMask");
        assert_eq!(mix["sam"]["inputs"]["image"], json!(["src.seq", 0]));
        assert_eq!(mix["sam"]["inputs"]["frame_time_ms"], 0);
        assert_eq!(mix["sam"]["inputs"]["target_fps"], 24);
        assert_eq!(mix["sam"]["inputs"]["frame_count"], p.frames);
        assert_eq!(mix["face"]["inputs"]["image"], json!(["src.seq", 0]));
        assert_eq!(mix["pose"]["inputs"]["image"], json!(["src.seq", 0]));
        assert_eq!(mix["background"]["inputs"]["image"], json!(["src.seq", 0]));
        assert_eq!(mix["mask.grow"]["inputs"]["mask"], json!(["sam", 0]));
        let (mov, _) = graph_for(sample_move(), Mode::Move, 10_000, "source");
        assert!(mov.get("src.seq").is_none());
        assert!(mov.get("sam").is_none());
        assert_eq!(mov["face"]["inputs"]["image"], json!(["src.scaled", 0]));
        assert_eq!(mov["pose"]["inputs"]["image"], json!(["src.scaled", 0]));
    }

    #[test]
    fn tracking_graph_has_no_diffusion() {
        use crate::target_keyframes::{
            Direction, FrameGeometry, Keyframe, PointLabel, SelPoint, Selection, TrackingRequest,
        };
        let kf = Keyframe {
            keyframe_id: String::from("kf0"),
            time_ms: 0,
            points: vec![SelPoint {
                point_id: String::from("p0"),
                x: 10.0,
                y: 10.0,
                label: PointLabel::Positive,
            }],
            box_: None,
            frame_index: 0,
            resolved_time_ms: 0,
        };
        let tr = TrackingRequest {
            source_media_id: String::from("src"),
            start_ms: 0,
            end_ms: 1000,
            selection: Selection {
                target_id: String::from("t1"),
                revision: String::from("r1"),
                frame_geometry: FrameGeometry { width: 1280, height: 720, rotation: 0 },
                keyframes: vec![kf.clone()],
                selection_digest: String::from("sha256:") + &"a".repeat(64),
            },
            direction: Direction::Both,
            anchor_keyframe_id: String::from("kf0"),
            coverage_start_ms: 0,
            coverage_end_ms: 1000,
            request_digest: String::from("sha256:") + &"b".repeat(64),
            source_digest: String::from("sha256:") + &"c".repeat(64),
            width: 1280,
            height: 720,
            fps: 24,
            sampled_frames: 24,
            frame_times_ms: (0..24).map(|i| i * 1000 / 24).collect(),
        };
        let inputs = TrackingInputs {
            source_file: "clip.mp4",
            keyframes_json: "[]",
            frame_count: 24,
            request_id: "req-1",
            source_digest: "sha256:cc",
            input_schema_digest: "dd",
            mapped_keyframes: core::slice::from_ref(&kf),
        };
        let g = build_tracking(&tr, &inputs);
        let obj = g.as_object().unwrap();
        assert!(obj.contains_key("sam.multi"));
        assert!(obj.contains_key("track.export"));
        assert!(!obj.keys().any(|k| k.contains("sampler") || k.contains("unet") || k.starts_with("s0.")));
        assert_eq!(obj["sam.multi"]["class_type"], "BGXAnimateMultiKeyframeMask");
        assert_eq!(obj["track.export"]["class_type"], "BGXAnimateMaskArtifactExport");
        assert_eq!(obj["sam.multi"]["inputs"]["image"], json!(["src.seq", 0]));
        assert_eq!(obj["sam.multi"]["inputs"]["request_id"], "req-1");

        let mut stamped = tr.clone();
        stamped.fps = 0;
        stamped.sampled_frames = 3;
        stamped.frame_times_ms = vec![0, 41, 90];
        let g = build_tracking(&stamped, &TrackingInputs { frame_count: 3, ..inputs });
        let obj = g.as_object().unwrap();
        assert!(!obj.contains_key("src.seq"), "actual timestamps must not be resampled");
        assert_eq!(obj["sam.multi"]["inputs"]["image"], json!(["src.scaled", 0]));
        assert_eq!(obj["sam.multi"]["inputs"]["frame_times_json"], "[0,41,90]");
        assert_eq!(obj["track.export"]["inputs"]["frame_times_json"], "[0,41,90]");
    }

    #[test]
    fn mix_with_mask_artifact_skips_sam_model() {
        use crate::parameters::{Advanced, Request};
        use crate::target_keyframes::MaskArtifactRef;
        let r = Request {
            source_media_id: String::from("s"),
            replacement_media_id: String::from("r"),
            start_ms: 0,
            end_ms: 2000,
            replace: None,
            mask_artifact: Some(MaskArtifactRef {
                artifact_id: String::from("art-1"),
                digest: String::from("sha256:") + &"e".repeat(64),
            }),
            mask_grow_pixels: Some(4),
            prompt: String::from("p"),
            negative_prompt: String::from("n"),
            width: 1280,
            height: 720,
            fps: 24,
            keep_audio: false,
            advanced: Advanced {
                window_frames: 77,
                overlap_frames: 5,
                seed: 0,
                steps: 6,
                cfg: 1.0,
                sampler: String::from("euler"),
                scheduler: String::from("simple"),
                sampling_shift: 8.0,
                distill_lora_strength: 1.0,
                relight_lora_strength: 1.0,
            },
        };
        let plan = crate::planner::plan(&r).unwrap();
        let g = build(
            &r,
            &plan,
            &Inputs {
                source_file: "s.mp4",
                replacement_file: "r.png",
                points: &[],
            },
        );
        let obj = g.as_object().unwrap();
        assert_eq!(obj["sam"]["class_type"], "BGXAnimateMaskArtifactLoad");
        assert!(!obj.contains_key("sam.model"));
        assert_eq!(obj["sam"]["inputs"]["artifact_id"], "art-1");
        assert!(obj.contains_key("mask.block"));
    }

    #[test]
    fn mix_passes_nonzero_frame_time_ms() {
        let mut v = sample();
        v["sourceRange"]["endMs"] = 10_000.into();
        v["targetPoints"]["frameTimeMs"] = 1500.into();
        let (g, p) = graph_for(v, Mode::Mix, 10_000, "source");
        assert_eq!(g["sam"]["inputs"]["frame_time_ms"], 1500);
        assert_eq!(g["src.seq"]["inputs"]["frame_count"], p.frames);
    }
}
