//! Dynamic API graph mirroring the certified "Motion Transfer (Wan Animate 2 Distilled)"
//! subgraph: shared loaders/encoders once, K WanAnimate2ToVideo passes chained by the
//! node-native continuation outputs, one final SaveVideo trimmed to N frames. The canvas'
//! side-by-side comparison branch ("Video Stitch" → second SaveVideo) is not emitted.
use alloc::string::{String, ToString};
use serde_json::{json, Map, Value};

use crate::{
    mapping::{CONTEXT_WINDOWS, MODELS},
    parameters::Request,
    planner::{segment_node, Plan},
};

pub const OUTPUT_NODE: &str = "out.save";

pub struct Inputs<'a> {
    /// Server file reference of the uploaded, prepared driving video.
    pub source_file: &'a str,
    /// Server file reference of the uploaded character image.
    pub replacement_file: &'a str,
}

fn link(node: &str, slot: u64) -> Value {
    json!([node, slot])
}

fn resize(input: Value, w: u64, h: u64) -> Value {
    json!({"input": input, "resize_type": "scale dimensions", "resize_type.width": w, "resize_type.height": h,
        "resize_type.crop": "center", "scale_method": "area"})
}

pub fn build(r: &Request, plan: &Plan, inputs: &Inputs) -> Value {
    let a = &r.advanced;
    let mut g = Map::new();
    let mut put = |id: &str, class: &str, inputs: Value| {
        g.insert(id.to_string(), json!({"class_type": class, "inputs": inputs}));
    };
    put("unet", "UNETLoader", json!({"unet_name": MODELS.unet, "weight_dtype": "default"}));
    // The canvas' ComfySwitchNode(switch_1) picks between the plain model and context windows.
    let cache_model = if a.context_windows {
        put("ctx", "ContextWindowsManual", json!({"model": link("unet", 0),
            "context_length": CONTEXT_WINDOWS.length, "context_overlap": CONTEXT_WINDOWS.overlap,
            "context_schedule": "standard_static", "context_stride": 1, "closed_loop": false, "fuse_method": "pyramid",
            "dim": CONTEXT_WINDOWS.dim, "freenoise": true, "cond_retain_index_list": "0", "split_conds_to_windows": false,
            "latent_retain_index_list": "", "causal_window_fix": true}));
        link("ctx", 0)
    } else {
        link("unet", 0)
    };
    put("cache", "WanAnimate2Cache", json!({"model": cache_model, "device": a.cache_device, "dtype": a.cache_dtype}));
    put("shift", "ModelSamplingSD3", json!({"model": link("cache", 0), "shift": a.sampling_shift}));
    put("sigmas", "BasicScheduler", json!({"model": link("cache", 0), "scheduler": a.scheduler, "steps": a.steps, "denoise": 1}));
    put("sampler", "KSamplerSelect", json!({"sampler_name": a.sampler}));
    put("clip", "CLIPLoader", json!({"clip_name": MODELS.text_encoder, "type": "wan", "device": "default"}));
    put("vae", "VAELoader", json!({"vae_name": MODELS.vae}));
    put("clipvision", "CLIPVisionLoader", json!({"clip_name": MODELS.clip_vision}));
    put("prompt.pos", "CLIPTextEncode", json!({"clip": link("clip", 0), "text": r.prompt}));
    put("prompt.neg", "CLIPTextEncode", json!({"clip": link("clip", 0), "text": r.negative_prompt}));
    let motion = !a.motion_prompt.is_empty();
    if motion {
        put("prompt.pose", "CLIPTextEncode", json!({"clip": link("clip", 0), "text": a.motion_prompt}));
    }
    put("ref", "LoadImage", json!({"image": inputs.replacement_file}));
    put("src", "LoadVideo", json!({"file": inputs.source_file}));
    put("src.parts", "GetVideoComponents", json!({"video": link("src", 0)}));
    put("pose.frames", "ResizeImageMaskNode", resize(link("src.parts", 0), r.width, r.height));
    put("ref.image", "ResizeImageMaskNode", resize(link("ref", 0), r.width, r.height));
    put("ref.vision", "CLIPVisionEncode", json!({"clip_vision": link("clipvision", 0), "image": link("ref.image", 0), "crop": "none"}));
    put("pose.first", "ImageFromBatch", json!({"image": link("pose.frames", 0), "batch_index": 0, "length": 1}));
    put("pose.vision", "CLIPVisionEncode", json!({"clip_vision": link("clipvision", 0), "image": link("pose.first", 0), "crop": "none"}));

    let mut frames = String::new();
    let mut previous_decode = String::new();
    for s in &plan.segments {
        let k = s.index;
        let (anim, sample, trim, decode) = (
            segment_node(k, "animate"),
            segment_node(k, "sample"),
            segment_node(k, "trim"),
            segment_node(k, "decode"),
        );
        let mut animate = json!({"positive": link("prompt.pos", 0), "negative": link("prompt.neg", 0), "vae": link("vae", 0),
            "reference_image": link("ref.image", 0), "pose_video": link("pose.frames", 0),
            "clip_vision_output": link("ref.vision", 0), "clip_vision_output_pose": link("pose.vision", 0),
            "width": r.width, "height": r.height, "length": a.segment_frames, "batch_size": 1, "video_frame_offset": 0,
            "pose_strength": a.pose_strength, "pose_start_percent": a.pose_start_percent, "pose_end_percent": a.pose_end_percent,
            "reference_image_strength": a.reference_image_strength});
        if motion {
            animate["positive_pose"] = link("prompt.pose", 0);
        }
        if k > 0 {
            animate["continue_motion"] = link(&previous_decode, 0);
            animate["video_frame_offset"] = link(&segment_node(k - 1, "animate"), 5);
        }
        put(&anim, "WanAnimate2ToVideo", animate);
        put(&sample, "SamplerCustom", json!({"model": link("shift", 0), "add_noise": true, "noise_seed": s.seed, "cfg": a.cfg,
            "positive": link(&anim, 0), "negative": link(&anim, 1), "sampler": link("sampler", 0), "sigmas": link("sigmas", 0),
            "latent_image": link(&anim, 2)}));
        put(&trim, "TrimVideoLatent", json!({"samples": link(&sample, 0), "trim_amount": link(&anim, 3)}));
        put(&decode, "VAEDecode", json!({"samples": link(&trim, 0), "vae": link("vae", 0)}));
        frames = if k == 0 {
            decode.clone()
        } else {
            // Continued pass repeats the joint frame; drop it (canvas switch `value_1 = true`).
            let fresh = segment_node(k, "fresh");
            let joined = segment_node(k, "frames");
            put(&fresh, "ImageFromBatch", json!({"image": link(&decode, 0), "batch_index": 1, "length": 4096}));
            put(&joined, "ImageBatch", json!({"image1": link(&frames, 0), "image2": link(&fresh, 0)}));
            joined
        };
        previous_decode = decode;
    }
    put("out.frames", "ImageFromBatch", json!({"image": link(&frames, 0), "batch_index": 0, "length": plan.frames}));
    let mut video = json!({"images": link("out.frames", 0), "fps": r.fps});
    if r.keep_audio {
        video["audio"] = link("src.parts", 1);
    }
    put("out.video", "CreateVideo", video);
    put(OUTPUT_NODE, "SaveVideo", json!({"video": link("out.video", 0), "filename_prefix": "video/wan-animate2", "format": "auto", "codec": "auto"}));
    Value::Object(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parameters::{parse, tests::sample}, planner::plan};

    fn graph(end_ms: u64, mutate: impl FnOnce(&mut Value)) -> (Value, Plan) {
        let mut v = sample();
        v["sourceRange"]["endMs"] = end_ms.into();
        mutate(&mut v);
        let r = parse(&v).ok().unwrap();
        let p = plan(&r).ok().unwrap();
        (build(&r, &p, &Inputs { source_file: "drive.mp4", replacement_file: "char.png" }), p)
    }
    fn of_class<'a>(g: &'a Value, class: &str) -> alloc::vec::Vec<(&'a String, &'a Value)> {
        g.as_object().unwrap().iter().filter(|(_, n)| n["class_type"] == class).collect()
    }

    #[test]
    fn single_pass_maps_both_texts_and_media_to_their_own_inputs() {
        let (g, p) = graph(2000, |_| {});
        assert_eq!((p.frames, p.segments.len()), (60, 1));
        assert_eq!(g["prompt.pos"]["inputs"]["text"], "a toy figure dancing");
        assert_eq!(g["prompt.pose"]["inputs"]["text"], "street dance");
        let anim = &g["g0.animate"]["inputs"];
        assert_eq!(anim["positive"], json!(["prompt.pos", 0]));
        assert_eq!(anim["positive_pose"], json!(["prompt.pose", 0]));
        assert_eq!(anim["reference_image"], json!(["ref.image", 0]));
        assert_eq!(anim["pose_video"], json!(["pose.frames", 0]));
        assert_eq!((anim["width"].as_u64(), anim["height"].as_u64(), anim["length"].as_u64()), (Some(480), Some(832), Some(81)));
        assert!(anim.get("continue_motion").is_none());
        assert_eq!(g["ref"]["inputs"]["image"], "char.png");
        assert_eq!(g["src"]["inputs"]["file"], "drive.mp4");
        assert_eq!(g["g0.sample"]["inputs"]["noise_seed"], 7);
        assert_eq!(g["out.frames"]["inputs"]["length"], 60);
        assert_eq!(g["out.video"]["inputs"]["audio"], json!(["src.parts", 1]));
        assert_eq!(of_class(&g, "SaveVideo").len(), 1, "only the final output, no comparison stitch");
        assert!(g.get("ctx").is_none());
        assert_eq!(g["cache"]["inputs"]["model"], json!(["unet", 0]));
    }

    #[test]
    fn empty_motion_prompt_leaves_pose_text_to_the_node_default() {
        let (g, _) = graph(2000, |v| v["advanced"]["motionPrompt"] = "".into());
        assert!(g.get("prompt.pose").is_none());
        assert!(g["g0.animate"]["inputs"].get("positive_pose").is_none());
    }

    #[test]
    fn long_range_chains_segments_and_drops_joint_frame() {
        let (g, p) = graph(6000, |v| { v["audio"] = "none".into(); v["advanced"]["contextWindows"] = true.into(); });
        assert_eq!((p.frames, p.segments.len()), (180, 3));
        let a1 = &g["g1.animate"]["inputs"];
        assert_eq!(a1["continue_motion"], json!(["g0.decode", 0]));
        assert_eq!(a1["video_frame_offset"], json!(["g0.animate", 5]));
        assert_eq!(g["g2.animate"]["inputs"]["continue_motion"], json!(["g1.decode", 0]));
        assert_eq!(g["g1.fresh"]["inputs"]["batch_index"], 1);
        assert_eq!(g["g2.frames"]["inputs"]["image1"], json!(["g1.frames", 0]));
        assert_eq!(g["out.frames"]["inputs"]["image"], json!(["g2.frames", 0]));
        assert_eq!(g["g2.sample"]["inputs"]["noise_seed"], 9);
        assert!(g["out.video"]["inputs"].get("audio").is_none());
        assert_eq!(g["cache"]["inputs"]["model"], json!(["ctx", 0]));
    }

    /// `BGX_WA2_GRAPH_DUMP=<file> cargo test dump_graph -- --ignored` writes a 3-segment graph
    /// for offline validation against a live ComfyUI `/object_info`.
    #[test]
    #[ignore]
    fn dump_graph() {
        extern crate std;
        let (g, _) = graph(6000, |v| v["advanced"]["contextWindows"] = true.into());
        std::fs::write(std::env::var("BGX_WA2_GRAPH_DUMP").unwrap(), g.to_string()).unwrap();
    }
}