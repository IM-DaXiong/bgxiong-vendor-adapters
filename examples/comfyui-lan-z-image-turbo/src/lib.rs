//! Isolated LAN ComfyUI Z-Image-Turbo t2i guest.
//! Vendor strings stay in this crate. Host must not name Z-Image or this crate.

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
    parse_history, parse_submit_prompt_id, resolve_base_url, submit_body, validate_submit_payload,
    MappedStatus,
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
    let (kind, prompt, size) = validate_submit_payload(&payload)?;
    let _ = resolve_base_url(&payload);
    let _body = submit_body(kind, &prompt, size, None, "host-test")?;
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
        .map(|o| Output {
            media_kind: o.media_kind,
            source: o.source,
            value: o.value,
            mime: o.mime,
        })
        .collect();
    let sdk = query_result(&QueryResult {
        status: status.into(),
        outputs,
        progress_text: q.vendor_message,
    });
    serde_json::from_str(sdk.data_json.as_deref().unwrap_or("{}")).map_err(|e| format!("sdk: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comfy::{
        configured_endpoint_base_url, parse_size_wh, patch_prompt, ModelKind,
    };
    use crate::config::{
        size_label, COMFY_BASE_URL, LATENT_NODE_ID, MODEL_ID_TURBO, PLUGIN_ID, PLUGIN_VERSION,
        PROMPT_NODE_ID, SAMPLER_NODE_ID, SIZE_PRESETS, WORKFLOW_PIN_TURBO,
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
            r#"{"endpointBaseUrl":"http://192.168.18.8:8188"}"#,
        )
        .unwrap_err();
        assert!(err.contains("host-http"));
    }

    #[test]
    fn capabilities_returns_rcd_enum_resolution() {
        let v = dispatch_json("capabilities", "{}").expect("caps");
        assert_eq!(v["schemaVersion"], 1);
        assert_eq!(v["pluginId"], PLUGIN_ID);
        assert_eq!(v["pluginVersion"], PLUGIN_VERSION);
        assert_eq!(v["slots"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(v["slots"][0]["slot"], "image");
        assert_eq!(v["slots"][0]["modelId"], MODEL_ID_TURBO);
        assert_eq!(v["slots"][0]["maxReferenceImages"], 0);
        assert_eq!(v["slots"][0]["resolution"]["mode"], "enum");
        assert_eq!(v["slots"][0]["resolution"]["default"], "1920x1088");
        assert_eq!(v["slots"][0]["resolution"]["bindingId"], "57:13.size");
        let opts = v["slots"][0]["resolution"]["options"].as_array().unwrap();
        assert_eq!(opts.len(), 10);
        assert_eq!(opts[0]["aspectRatio"], "16:9");
        assert_eq!(opts[0]["value"], "1920x1088");
        assert!(v["slots"][0].get("duration").is_none());
        assert!(v["slots"][0].get("fps").is_none());
    }

    #[test]
    fn unknown_model_hard_fails() {
        let err = dispatch_json("submit", r#"{"model":"z.turbo.guagua","prompt":"x"}"#)
            .unwrap_err();
        assert!(err.contains("unknown model"));
    }

    #[test]
    fn empty_prompt_hard_fails() {
        let err = dispatch_json("submit", r#"{"model":"z.turbo","prompt":"  "}"#).unwrap_err();
        assert!(err.contains("prompt"));
    }

    #[test]
    fn reference_images_hard_fail() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"z.turbo","prompt":"x","referenceImagesB64":["abc"]}"#,
        )
        .unwrap_err();
        assert!(err.contains("reference"));
    }

    #[test]
    fn size_unknown_hard_fails() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"z.turbo","prompt":"x","size":"1024x1024"}"#,
        )
        .unwrap_err();
        assert!(err.contains("size"));
        assert!(err.contains("1920x1088"));
    }

    #[test]
    fn size_match_preset_ok_then_live_http_gate() {
        let err = dispatch_json(
            "submit",
            r#"{"model":"z.turbo","prompt":"a lantern","size":"1920x1088"}"#,
        )
        .unwrap_err();
        assert!(err.contains("host-http"));
        let err4k = dispatch_json(
            "submit",
            r#"{"model":"z.turbo","prompt":"a lantern","size":"2160x3840"}"#,
        )
        .unwrap_err();
        assert!(err4k.contains("host-http"));
    }

    #[test]
    fn mapper_writes_prompt_latent_seed_and_applied() {
        let (g, applied) = patch_prompt(ModelKind::Turbo, "hello lan", None, Some(42), "cid").unwrap();
        assert_eq!(g[PROMPT_NODE_ID]["inputs"]["text"], "hello lan");
        assert_eq!(g[LATENT_NODE_ID]["inputs"]["width"], 1920);
        assert_eq!(g[LATENT_NODE_ID]["inputs"]["height"], 1088);
        assert_eq!(g[SAMPLER_NODE_ID]["inputs"]["seed"], 42);
        assert_eq!(g[SAMPLER_NODE_ID]["inputs"]["steps"], 8);
        assert_eq!(g[SAMPLER_NODE_ID]["inputs"]["cfg"], 1);
        assert_eq!(applied.resolution, Some(serde_json::json!("1920x1088")));
        let (g4, applied4) = patch_prompt(
            ModelKind::Turbo,
            "wide",
            Some("2160x3840"),
            Some(7),
            "c",
        )
        .unwrap();
        assert_eq!(g4[LATENT_NODE_ID]["inputs"]["width"], 2160);
        assert_eq!(g4[LATENT_NODE_ID]["inputs"]["height"], 3840);
        assert_eq!(applied4.resolution, Some(serde_json::json!("2160x3840")));
    }

    #[test]
    fn ten_presets_patch_latent_wh() {
        for (id, w, h, _ar) in SIZE_PRESETS {
            let size = size_label(*w, *h);
            let (g, applied) =
                patch_prompt(ModelKind::Turbo, "x", Some(&size), Some(1), "c").unwrap();
            assert_eq!(g[LATENT_NODE_ID]["inputs"]["width"], *w, "preset {id}");
            assert_eq!(g[LATENT_NODE_ID]["inputs"]["height"], *h, "preset {id}");
            assert_eq!(applied.resolution, Some(serde_json::json!(size)));
        }
    }

    #[test]
    fn parse_size_wh_closed() {
        assert_eq!(parse_size_wh(None).unwrap(), None);
        assert_eq!(parse_size_wh(Some("1920x1088")).unwrap(), Some((1920, 1088)));
        assert!(parse_size_wh(Some("bad")).is_err());
    }

    #[test]
    fn submit_prompt_id_only_from_comfy_envelope() {
        let id = parse_submit_prompt_id(&serde_json::json!({"prompt_id":"abc-1","number":2}))
            .unwrap();
        assert_eq!(id, "abc-1");
        assert!(parse_submit_prompt_id(&serde_json::json!({"data":{"taskId":"nope"}})).is_err());
        assert!(parse_submit_prompt_id(&serde_json::json!({"node_errors":{"57:3":{}}} )).is_err());
    }

    #[test]
    fn history_queued_running_success_fail() {
        let q = parse_history(&serde_json::json!({}), "t1", "http://192.168.18.8:8188").unwrap();
        assert_eq!(q.status, MappedStatus::Queued);
        let running = parse_history(
            &serde_json::json!({"t1":{"status":{"completed":false,"status_str":"running"}}}),
            "t1",
            "http://192.168.18.8:8188",
        )
        .unwrap();
        assert_eq!(running.status, MappedStatus::Running);
        let fail = parse_history(
            &serde_json::json!({"t1":{"status":{"completed":true,"status_str":"error"}}}),
            "t1",
            "http://192.168.18.8:8188",
        )
        .unwrap();
        assert_eq!(fail.status, MappedStatus::Failed);
        let ok = map_query_response_json(
            &serde_json::json!({
                "t1": {
                    "status": {"completed": true, "status_str": "success"},
                    "outputs": {"9": {"images": [{"filename":"z-image-turbo_00001_.png","subfolder":"","type":"output"}]}}
                }
            }),
            "t1",
            "http://192.168.18.8:8188",
        )
        .unwrap();
        assert_eq!(ok["status"], "succeeded");
        assert_eq!(ok["outputs"][0]["source"], "url");
        assert_eq!(ok["outputs"][0]["mediaKind"], "image");
        assert!(ok["outputs"][0]["value"]
            .as_str()
            .unwrap()
            .contains("/view?filename=z-image-turbo_00001_.png"));
    }

    #[test]
    fn crate_pins_and_default_url() {
        assert_eq!(COMFY_BASE_URL, "http://192.168.18.8:8188");
        assert_eq!(WORKFLOW_PIN_TURBO.len(), 64);
        let src = include_str!("comfy.rs");
        assert!(!src.contains("runninghub"));
        assert!(!src.contains("/upload/image"));
        assert!(src.contains("/prompt"));
        let guest = include_str!("guest.rs");
        assert!(!guest.contains("/upload/image"));
        assert!(guest.contains("configured_endpoint_base_url"));
    }

    #[test]
    fn endpoint_base_url_prefers_payload() {
        let p = serde_json::json!({"endpointBaseUrl":"http://192.0.2.8:8188"});
        assert_eq!(resolve_base_url(&p), "http://192.0.2.8:8188");
        assert_eq!(configured_endpoint_base_url(&serde_json::json!({})), None);
        assert_eq!(
            resolve_base_url(&serde_json::json!({})),
            "http://192.168.18.8:8188"
        );
    }

    #[test]
    fn single_model_and_ten_presets() {
        assert_eq!(MODEL_ID_TURBO, "z.turbo");
        assert_eq!(SIZE_PRESETS.len(), 10);
        let (a, _) = patch_prompt(ModelKind::Turbo, "x", None, Some(1), "c").unwrap();
        assert_eq!(a[LATENT_NODE_ID]["inputs"]["width"], 1920);
        assert_eq!(a[LATENT_NODE_ID]["inputs"]["height"], 1088);
    }
}
