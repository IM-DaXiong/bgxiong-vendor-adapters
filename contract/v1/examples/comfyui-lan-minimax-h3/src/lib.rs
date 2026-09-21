//! Isolated LAN ComfyUI MiniMax H3 t2v guest.
//! Vendor strings stay in this crate. Host must not name MiniMax or LAN Comfy.

#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
extern crate alloc;

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

pub mod caps;
pub mod comfy;
pub mod config;

#[cfg(not(target_arch = "wasm32"))]
use bgx_vendor_adapter_sdk::{query_result, submit_accepted, Output, QueryResult};

#[cfg(not(target_arch = "wasm32"))]
use crate::comfy::{
    parse_history, parse_model_kind, parse_submit_prompt_id, resolve_base_url, submit_body_for_kind,
    MappedStatus, ModelKind,
};

#[cfg(any(target_arch = "wasm32", test))]
mod wasm_mem;

#[cfg(target_arch = "wasm32")]
#[path = "guest.rs"]
mod guest;

#[cfg(not(target_arch = "wasm32"))]
pub fn dispatch_json(operation: &str, payload_json: &str) -> Result<serde_json::Value, String> {
    match operation {
        "capabilities" => crate::caps::runtime_caps_doc(),
        "probe" => probe_op(payload_json),
        "submit" => submit_op(payload_json),
        "query" => query_op(payload_json),
        "cancel" | "upload" => Err(format!("{operation} is out of example scope")),
        other => Err(format!("unknown operation {other}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn probe_op(payload_json: &str) -> Result<serde_json::Value, String> {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).unwrap_or_else(|_| serde_json::json!({}));
    if crate::comfy::configured_endpoint_base_url(&payload).is_some() {
        return Err("live HTTP is only available inside the wasm guest via host-http".into());
    }
    Ok(crate::comfy::offline_probe_doc())
}

#[cfg(not(target_arch = "wasm32"))]
fn submit_op(payload_json: &str) -> Result<serde_json::Value, String> {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|e| format!("payload: {e}"))?;
    let kind = parse_model_kind(payload.get("model").and_then(|v| v.as_str()))?;
    let prompt = payload
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let duration = payload.get("durationSeconds").and_then(|v| v.as_f64());
    let fps = payload.get("fps").and_then(|v| v.as_f64());
    let resolution = payload.get("resolution").and_then(|v| v.as_str());
    let aspect = payload
        .get("aspect")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("aspectRatio").and_then(|v| v.as_str()));
    let _ = resolve_base_url(&payload);
    let images = crate::comfy::collect_lan_images(&payload)?;
    let first = match kind {
        ModelKind::I2vTurbo => Some(
            images
                .first()
                .map(|s| {
                    if s.file_name.trim().is_empty() {
                        "first.png".into()
                    } else {
                        s.file_name.clone()
                    }
                })
                .ok_or_else(|| "h3.i2v.turbo requires a first-frame image".to_string())?,
        ),
        _ => None,
    };
    let refs = match kind {
        ModelKind::R2vTurbo => {
            if images.is_empty() {
                return Err("h3.r2v.turbo requires at least one reference image".into());
            }
            images
                .iter()
                .map(|s| {
                    if s.file_name.trim().is_empty() {
                        "ref.png".into()
                    } else {
                        s.file_name.clone()
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let _body = submit_body_for_kind(
        kind,
        prompt,
        duration,
        fps,
        resolution,
        aspect,
        first.as_deref(),
        &refs,
        "host-test",
    )?;
    Err("live HTTP is only available inside the wasm guest via host-http".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn query_op(payload_json: &str) -> Result<serde_json::Value, String> {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|e| format!("payload: {e}"))?;
    let _id = payload
        .get("vendorTaskId")
        .and_then(|v| v.as_str())
        .ok_or("query requires vendorTaskId")?;
    Err("live HTTP is only available inside the wasm guest via host-http".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn map_submit_response_json(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let id = parse_submit_prompt_id(body)?;
    let sdk = submit_accepted(id);
    serde_json::from_str(sdk.data_json.as_deref().unwrap_or("{}"))
        .map_err(|e| format!("sdk: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn map_query_response_json(
    body: &serde_json::Value,
    prompt_id: &str,
    base: &str,
) -> Result<serde_json::Value, String> {
    let q = parse_history(body, prompt_id, base)?;
    let status = match q.status {
        MappedStatus::Queued => "queued",
        MappedStatus::Running => "running",
        MappedStatus::Succeeded => "succeeded",
        MappedStatus::Failed => "failed",
    };
    let outputs: Vec<Output> = q
        .outputs
        .into_iter()
        .map(|o| Output::Media {
            media_kind: o.media_kind,
            source: o.source,
            value: o.value,
            mime: o.mime,
        })
        .collect();
    let sdk = query_result(&QueryResult {
        status: status.into(),
        outputs,
        progress_text: if q.terminal_failure.is_some() {
            None
        } else {
            q.vendor_message
        },
        retry_after_ms: None,
        terminal_failure: q.terminal_failure,
    });
    serde_json::from_str(sdk.data_json.as_deref().unwrap_or("{}")).map_err(|e| format!("sdk: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comfy::{
        configured_endpoint_base_url, patch_t2v_prompt, resolution_combo, submit_body,
    };
    use crate::config::{
        DURATION_NODE_ID, FPS_NODE_ID, MATH_EXPRESSION_NODE_ID, MODEL_ID, PLUGIN_ID,
        PROMPT_NODE_ID, RESOLUTION_NODE_ID, WORKFLOW_PIN,
    };

    #[test]
    fn probe_is_offline_ok() {
        let v = dispatch_json("probe", "{}").expect("probe");
        assert_eq!(v["ok"], true);
        assert_eq!(v["live"], false);
        let v2 = dispatch_json("probe", r#"{"extra":{}}"#).expect("probe extra");
        assert_eq!(v2["live"], false);
    }

    #[test]
    fn probe_with_endpoint_does_not_fake_live_http() {
        let err = dispatch_json(
            "probe",
            r#"{"endpointBaseUrl":"http://192.0.2.8:8188"}"#,
        )
        .unwrap_err();
        assert!(err.contains("host-http"));
        let err2 = dispatch_json(
            "probe",
            r#"{"extra":{"endpointBaseUrl":"http://192.0.2.8:8188"}}"#,
        )
        .unwrap_err();
        assert!(err2.contains("host-http"));
    }

    #[test]
    fn capabilities_returns_rcd() {
        let v = dispatch_json("capabilities", "{}").expect("caps");
        assert_eq!(v["schemaVersion"], 1);
        assert_eq!(v["pluginId"], PLUGIN_ID);
        assert_eq!(v["pluginVersion"], crate::config::PLUGIN_VERSION);
        assert_eq!(v["slots"].as_array().map(|a| a.len()), Some(3));
        assert_eq!(v["slots"][0]["modelId"], MODEL_ID);
        assert_eq!(v["slots"][0]["implementedModeIds"][0], "text_to_video");
        assert_eq!(v["slots"][0]["maxReferenceImages"], 0);
        assert_eq!(v["slots"][1]["modelId"], crate::config::MODEL_ID_I2V_TURBO);
        assert_eq!(v["slots"][1]["implementedModeIds"][0], "image_to_video");
        assert_eq!(v["slots"][2]["modelId"], crate::config::MODEL_ID_R2V_TURBO);
        assert_eq!(v["slots"][2]["implementedModeIds"][0], "multi_image_to_video");
        assert_eq!(v["slots"][0]["duration"]["min"], 1.0);
        assert_eq!(v["slots"][0]["duration"]["bindingId"], DURATION_NODE_ID);
        assert_eq!(v["slots"][0]["fps"]["bindingId"], FPS_NODE_ID);
        assert_eq!(v["slots"][0]["resolution"]["bindingId"], "115:megapixels");
        assert_eq!(v["slots"][0]["aspect"]["bindingId"], "115:aspect_ratio");
        assert_eq!(v["slots"][0]["resolution"]["default"], "mp:0.4");
        assert_eq!(v["slots"][0]["aspect"]["default"], "16:9");
    }

    #[test]
    fn unknown_model_hard_fails() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"h3.i2v","prompt":"x","durationSeconds":5,"fps":24,"resolution":"16:9"}"#,
        )
        .unwrap_err();
        assert!(err.contains("unknown model"));
    }

    #[test]
    fn empty_prompt_hard_fails() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"h3.t2v","prompt":"  ","durationSeconds":5,"fps":24,"resolution":"16:9"}"#,
        )
        .unwrap_err();
        assert!(err.contains("prompt"));
    }

    #[test]
    fn mapper_writes_prompt_duration_fps_expression_resolution() {
        let (g, applied) = patch_t2v_prompt(
            "hello lan",
            Some(7.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
        )
        .unwrap();
        assert_eq!(g[PROMPT_NODE_ID]["inputs"]["prompt"], "hello lan");
        assert_eq!(g[DURATION_NODE_ID]["inputs"]["value"], 7.0);
        assert_eq!(g[FPS_NODE_ID]["inputs"]["fps"], 24.0);
        assert_eq!(
            g[MATH_EXPRESSION_NODE_ID]["inputs"]["expression"],
            "max(5, round(a * 24)) + (5 - (max(5, round(a * 24)) % 17)) % 17"
        );
        assert_eq!(
            g[RESOLUTION_NODE_ID]["inputs"]["aspect_ratio"],
            "16:9 (Widescreen)"
        );
        assert_eq!(g[RESOLUTION_NODE_ID]["inputs"]["megapixels"], 0.4);
        assert_eq!(applied.duration.unwrap(), serde_json::json!(7.0));
        assert_eq!(applied.fps.unwrap(), serde_json::json!(24.0));
        assert_eq!(applied.resolution.unwrap(), serde_json::json!("mp:0.4"));
        assert_eq!(applied.aspect.unwrap(), serde_json::json!("16:9"));
        assert_eq!(g["139"]["inputs"]["value"], false);
    }

    #[test]
    fn omitted_fps_keeps_pin_and_does_not_fail() {
        let (g, applied) = patch_t2v_prompt(
            "hello lan",
            Some(7.0),
            None,
            Some("mp:0.4"),
            Some("16:9"),
        )
        .unwrap();
        assert!(applied.fps.is_none());
        assert_eq!(g[DURATION_NODE_ID]["inputs"]["value"], 7.0);
        assert_eq!(applied.duration.unwrap(), serde_json::json!(7.0));
    }

    #[test]
    fn i2v_turbo_requires_frame_and_keeps_lightning_on() {
        let err = crate::comfy::patch_i2v_turbo_prompt(
            "x",
            Some(5.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            "",
        )
        .unwrap_err();
        assert!(err.contains("first-frame"));
        let (g, _) = crate::comfy::patch_i2v_turbo_prompt(
            "keep face",
            Some(5.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            "first.png",
        )
        .unwrap();
        assert_eq!(g["139"]["inputs"]["value"], true);
        assert_eq!(g["140"]["inputs"]["image"], "first.png");
        assert_eq!(g[PROMPT_NODE_ID]["inputs"]["first_frame"], serde_json::json!(["140", 0]));
    }

    #[test]
    fn r2v_turbo_rejects_empty_refs_and_trims_slots() {
        let err = crate::comfy::patch_r2v_turbo_prompt(
            "x",
            Some(5.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            &[],
        )
        .unwrap_err();
        assert!(err.contains("at least one reference image"));
        let mut names = Vec::new();
        names.push("a.png".into());
        names.push("b.jpg".into());
        let (g, _) = crate::comfy::patch_r2v_turbo_prompt(
            "use <Picture 1>",
            Some(6.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            &names,
        )
        .unwrap();
        assert_eq!(g["137"]["inputs"]["image"], "a.png");
        assert_eq!(g["139"]["inputs"]["image"], "b.jpg");
        assert!(g["136"]["inputs"].get("ref_images.ref_image_2").is_none());
        assert_eq!(g["124"]["inputs"]["steps"], 8);
        assert!(g["142"]["inputs"]["lora_name"]
            .as_str()
            .unwrap()
            .contains("turbo"));
    }

    #[test]
    fn r2v_submit_folds_start_frame_handle() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"h3.r2v.turbo","prompt":"x","durationSeconds":5,"fps":24,"resolution":"mp:0.4","aspect":"16:9","startFrame":{"handle":"h1","fileName":"a.png","mime":"image/png"}}"#,
        )
        .unwrap_err();
        assert!(err.contains("live HTTP"));
        let b64 = dispatch_json(
            "submit",
            r#"{"model":"h3.r2v.turbo","prompt":"x","durationSeconds":5,"fps":24,"resolution":"mp:0.4","aspect":"16:9","startFrameB64":"abc"}"#,
        )
        .unwrap_err();
        assert!(b64.contains("legacy B64"));
    }

    #[test]
    fn upload_name_and_magic() {
        assert_eq!(
            crate::comfy::parse_comfy_upload_name(&serde_json::json!({"name":"a.png"})).unwrap(),
            "a.png"
        );
        assert_eq!(
            crate::comfy::parse_comfy_upload_name(&serde_json::json!({"name":"a.png","subfolder":"in"}))
                .unwrap(),
            "in/a.png"
        );
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(
            crate::comfy::sniff_image_filename_and_mime(&png, "first").unwrap(),
            ("first.png".into(), "image/png".into())
        );
        assert!(crate::comfy::sniff_image_ext_mime(b"not-an-image").is_err());
    }

    #[test]
    fn illegal_fps_hard_fails() {
        let err = patch_t2v_prompt("x", Some(5.0), Some(23.0), Some("mp:0.4"), Some("16:9")).unwrap_err();
        assert!(err.contains("closed_set_fps"));
    }

    #[test]
    fn resolution_combo_closed_set() {
        assert_eq!(resolution_combo("1:1").unwrap(), "1:1 (Square)");
        assert!(resolution_combo("21:9").is_err());
    }

    #[test]
    fn submit_prompt_id_only_from_comfy_envelope() {
        let id = parse_submit_prompt_id(&serde_json::json!({"prompt_id":"abc-1","number":2}))
            .unwrap();
        assert_eq!(id, "abc-1");
        assert!(parse_submit_prompt_id(&serde_json::json!({"data":{"taskId":"nope"}})).is_err());
        assert!(parse_submit_prompt_id(&serde_json::json!({"node_errors":{"131":{}}} )).is_err());
    }

    #[test]
    fn history_queued_running_success_fail() {
        let q = parse_history(&serde_json::json!({}), "t1", "http://127.0.0.1:8188").unwrap();
        assert_eq!(q.status, MappedStatus::Queued);
        let running = parse_history(
            &serde_json::json!({"t1":{"status":{"completed":false,"status_str":"running"}}}),
            "t1",
            "http://127.0.0.1:8188",
        )
        .unwrap();
        assert_eq!(running.status, MappedStatus::Running);
        let fail = parse_history(
            &serde_json::json!({"t1":{"status":{"completed":true,"status_str":"error"}}}),
            "t1",
            "http://127.0.0.1:8188",
        )
        .unwrap();
        assert_eq!(fail.status, MappedStatus::Failed);
        let ok = map_query_response_json(
            &serde_json::json!({
                "t1": {
                    "status": {"completed": true, "status_str": "success"},
                    "outputs": {"92": {"gifs": [{"filename":"a.mp4","subfolder":"","type":"output"}]}}
                }
            }),
            "t1",
            "http://127.0.0.1:8188",
        )
        .unwrap();
        assert_eq!(ok["status"], "succeeded");
        assert_eq!(ok["outputs"][0]["source"], "url");
        assert!(ok["outputs"][0]["value"]
            .as_str()
            .unwrap()
            .contains("/view?filename=a.mp4"));
        let live_shape = map_query_response_json(
            &serde_json::json!({
                "t1": {
                    "status": {"completed": true, "status_str": "success"},
                    "outputs": {"92": {"images": [{"filename":"MiniMax_H3_00191_.mp4","subfolder":"video","type":"output"}], "animated": [true]}}
                }
            }),
            "t1",
            "http://127.0.0.1:8188",
        )
        .unwrap();
        assert_eq!(live_shape["status"], "succeeded");
        assert_eq!(live_shape["outputs"][0]["mediaKind"], "video");
        let v = live_shape["outputs"][0]["value"].as_str().unwrap();
        assert!(v.contains("/view?filename=MiniMax_H3_00191_.mp4"));
        assert!(v.contains("subfolder=video"));
    }

    #[test]
    fn crate_does_not_call_runninghub() {
        let src = include_str!("comfy.rs");
        assert!(!src.contains("runninghub"));
        assert!(!src.contains("openapi/v2"));
        assert!(src.contains("/prompt"));
        assert_eq!(WORKFLOW_PIN.len(), 64);
        assert_eq!(crate::config::WORKFLOW_PIN_I2V_TURBO.len(), 64);
        assert_eq!(crate::config::WORKFLOW_PIN_R2V_TURBO.len(), 64);
        let guest = include_str!("guest.rs");
        assert!(guest.contains("/upload/image"));
    }

    #[test]
    fn endpoint_base_url_prefers_payload() {
        let p = serde_json::json!({"endpointBaseUrl":"http://192.0.2.8:8188"});
        assert_eq!(resolve_base_url(&p), "http://192.0.2.8:8188");
        assert_eq!(
            configured_endpoint_base_url(&serde_json::json!({})),
            None
        );
        assert_eq!(
            configured_endpoint_base_url(&serde_json::json!({"endpointBaseUrl":"  "})),
            None
        );
    }

    /// Author-machine dump. CI must not run this. Set `BGX_DUMP_PROMPT`.
    #[test]
    #[ignore]
    fn dump_live_submit_body() {
        let path = std::env::var("BGX_DUMP_PROMPT").expect("BGX_DUMP_PROMPT");
        let (body, _) = submit_body(
            "a red lantern hanging still, gentle breeze, cinematic",
            Some(1.0),
            Some(24.0),
            Some("mp:0.4"),
            Some("16:9"),
            "bgx-lan-live",
        )
        .expect("submit body");
        std::fs::write(&path, serde_json::to_vec(&body).expect("json")).expect("write dump");
    }
}
