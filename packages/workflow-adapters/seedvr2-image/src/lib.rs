#![cfg_attr(target_arch = "wasm32", no_std)]
//! One source image -> one SeedVR2 image. All node, model and HTTP semantics stay here.
extern crate alloc;

mod comfy;

use alloc::{
    format,
    string::{String, ToString},
};
use bgx_adapter_v2_sdk::{guest, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PLUGIN: &str = "local.bgx.seedvr2-image";
const OPERATION: &str = "local.bgx.seedvr2-image/upscale";
const INPUT: &str = include_str!("../schemas/input.schema.json");
const CONFIG: &str = include_str!("../schemas/config.schema.json");
const RESUME: &str = include_str!("../schemas/resume.schema.json");
const OUTPUT: &str = include_str!("../schemas/output.schema.json");

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    pub vendor: Option<String>,
}
impl Failure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            vendor: None,
        }
    }
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("INVALID_INPUT", message)
}
fn schema(id: &str, source: &str) -> Value {
    let digest: String = Sha256::digest(source.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({"id":id,"digest":digest,"schemaJson":source})
}
pub fn capabilities() -> Value {
    let input = schema("input", INPUT);
    let config = schema("config", CONFIG);
    let resume = schema("resume", RESUME);
    let output = schema("output", OUTPUT);
    json!({"schemaVersion":2,"pluginId":PLUGIN,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[{
        "schemaVersion":2,"localId":"single-image","capabilityKey":"local.bgx.seedvr2-image/upscale","transportSlot":"imageUpscale",
        "modelIds":["seedvr2-single-image"],"configurationSchema":{"id":"config","digest":config["digest"]},
        "operations":[{"id":OPERATION,"version":1,"label":"SeedVR2 单张图片放大",
            "inputSchema":{"id":"input","digest":input["digest"]},"outputSchema":{"id":"output","digest":output["digest"]},
            "resumeSchema":{"id":"resume","digest":resume["digest"]},
            "uiSchema":{"controls":[{"fieldPointer":"/source","widget":"media","label":"源图片","description":"原图字节","group":"输入"}]},
            "mediaSlots":[{"role":"source","inputPointer":"/source","mediaKind":"image","minCount":1,"maxCount":1}],
            "outputSlots":[{"role":"image","kind":"image","minCount":1,"maxCount":1}],
            "execution":{"mode":"asynchronous","streaming":false,"cancellation":"unsupported","resumable":true,"remoteIdempotency":false}}],
        "schemas":[input,config,resume,output]
    }]})
}

/// Maps every supported UI value to this frozen graph. The SDK validates scalar schema first;
/// cross-field executable restrictions (tiling) are checked before any upload.
fn graph(input: &Value, uploaded: &str) -> Result<Value, Failure> {
    let values = input["parameters"]
        .as_object()
        .ok_or_else(|| invalid("parameters must be an object"))?;
    let schema: Value = serde_json::from_str(INPUT).expect("checked input schema");
    let properties = schema["properties"]["parameters"]["properties"]
        .as_object()
        .expect("parameter properties");
    if values.keys().any(|k| !properties.contains_key(k)) {
        return Err(invalid("Unknown image upscale parameter"));
    }
    let get = |name: &str| {
        values
            .get(name)
            .unwrap_or(&properties[name]["default"])
            .clone()
    };
    for (enabled, size, overlap) in [
        ("encodeTiled", "encodeTileSize", "encodeTileOverlap"),
        ("decodeTiled", "decodeTileSize", "decodeTileOverlap"),
    ] {
        if get(enabled) == true
            && get(overlap)
                .as_u64()
                .zip(get(size).as_u64())
                .is_none_or(|(o, s)| o >= s)
        {
            return Err(invalid("Tile overlap must be smaller than tile size"));
        }
    }
    let dit = match get("enhancementModel").as_str() {
        Some("3b_fp16") => "seedvr2_ema_3b_fp16.safetensors",
        Some("7b_fp16") => "seedvr2_ema_7b_fp16.safetensors",
        Some("7b_fp8_mixed") => "seedvr2_ema_7b_fp8_e4m3fn_mixed_block35_fp16.safetensors",
        Some("7b_sharp_fp8_mixed") => {
            "seedvr2_ema_7b_sharp_fp8_e4m3fn_mixed_block35_fp16.safetensors"
        }
        _ => return Err(invalid("Unknown enhancement model")),
    };
    let mut graph: Value =
        serde_json::from_str(include_str!("workflow-api.json")).expect("frozen API graph");
    graph["16"]["inputs"]["image"] = json!(uploaded);
    for (name, target) in [
        ("targetResolution", "resolution"),
        ("maxResolution", "max_resolution"),
        ("seed", "seed"),
        ("colorCorrection", "color_correction"),
        ("inputNoiseScale", "input_noise_scale"),
        ("latentNoiseScale", "latent_noise_scale"),
    ] {
        graph["10"]["inputs"][target] = get(name);
    }
    graph["14"]["inputs"]["model"] = json!(dit);
    graph["14"]["inputs"]["blocks_to_swap"] = get("blocksToSwap");
    graph["14"]["inputs"]["swap_io_components"] = get("swapIoComponents");
    graph["14"]["inputs"]["offload_device"] = json!(if get("offloadDit") == true {
        "cpu"
    } else {
        "none"
    });
    graph["13"]["inputs"]["offload_device"] = json!(if get("offloadVae") == true {
        "cpu"
    } else {
        "none"
    });
    graph["13"]["inputs"]["model"] = get("vaeModel");
    for (name, target) in [
        ("encodeTiled", "encode_tiled"),
        ("encodeTileSize", "encode_tile_size"),
        ("encodeTileOverlap", "encode_tile_overlap"),
        ("decodeTiled", "decode_tiled"),
        ("decodeTileSize", "decode_tile_size"),
        ("decodeTileOverlap", "decode_tile_overlap"),
    ] {
        graph["13"]["inputs"][target] = get(name);
    }
    Ok(graph)
}
fn operation(payload: &Value) -> Result<(), Failure> {
    if payload["operationId"] != OPERATION {
        return Err(invalid("Unknown upscale operation"));
    }
    Ok(())
}
fn submit(host: &mut dyn Host, request: &Invocation, payload: &Value) -> Result<Value, Failure> {
    operation(payload)?;
    let source_id = payload["input"]["source"]["mediaId"]
        .as_str()
        .ok_or_else(|| invalid("source image is required"))?;
    let media = payload["media"]
        .as_array()
        .filter(|m| m.len() == 1)
        .and_then(|m| m.first())
        .filter(|m| m["id"] == source_id && m["kind"] == "image")
        .ok_or_else(|| invalid("Exactly one original image lease is required"))?;
    graph(&payload["input"], "pending.png")?;
    let context = comfy::Ctx {
        config: &payload["configuration"],
        budget_ms: request.r#budget_ms,
    };
    let uploaded = comfy::upload(host, &context, media)?;
    let graph = graph(&payload["input"], &uploaded)?;
    let remote = comfy::submit(host, &context, &graph, &request.r#request_id)?;
    let accepted = json!({"kind":"accepted","vendorTaskId":remote,"resumeToken":{"promptId":remote},"appliedParams":{},"usage":null});
    persist_accepted(host, request, &accepted)?;
    Ok(accepted)
}
fn persist_accepted(
    host: &mut dyn Host,
    request: &Invocation,
    accepted: &Value,
) -> Result<(), Failure> {
    let unknown = || Failure {
        code: "ACCEPTANCE_UNKNOWN",
        message: "Remote accepted; checkpoint failed. Do not resubmit.".into(),
        vendor: Some("CHECKPOINT_COMMIT_FAILED".into()),
    };
    let read = comfy::port(
        host,
        "host-checkpoint.read",
        json!({"requestId":request.r#request_id}),
    )
    .map_err(|_| unknown())?;
    let mut checkpoint: Value =
        serde_json::from_str(comfy::text(&read, "valueJson").map_err(|_| unknown())?)
            .map_err(|_| unknown())?;
    let old = comfy::text(&read, "revision").map_err(|_| unknown())?;
    let revision = old
        .parse::<u64>()
        .ok()
        .and_then(|r| r.checked_add(1))
        .ok_or_else(unknown)?;
    checkpoint["revision"] = json!(revision.to_string());
    checkpoint["state"] = json!("accepted");
    checkpoint["vendorTaskId"] = accepted["vendorTaskId"].clone();
    checkpoint["resumeToken"] = accepted["resumeToken"].clone();
    checkpoint["result"] = accepted.clone();
    comfy::port(host,"host-checkpoint.write",json!({"requestId":request.r#request_id,"expectedRevision":old,"valueJson":checkpoint.to_string()})).map_err(|_|unknown())?;
    Ok(())
}
fn query(host: &mut dyn Host, request: &Invocation, payload: &Value) -> Result<Value, Failure> {
    operation(payload)?;
    let context = comfy::Ctx {
        config: &payload["configuration"],
        budget_ms: request.r#budget_ms,
    };
    let id = comfy::text(payload, "vendorTaskId")?;
    if payload["resumeToken"]["promptId"] != id {
        return Err(invalid("Resume token must name the original task"));
    }
    let (status, outputs, failure) = match comfy::status(host, &context, id, "15")? {
        comfy::Remote::Queued => ("queued", json!([]), Value::Null),
        comfy::Remote::Running => ("running", json!([]), Value::Null),
        comfy::Remote::Succeeded(files) => (
            "succeeded",
            json!([comfy::download(host, &context, &files[0])?]),
            Value::Null,
        ),
        comfy::Remote::Failed(vendor) => (
            "failed",
            json!([]),
            json!({"category":"vendorTaskFailed","message":"SeedVR2 execution failed","vendorCode":vendor,"details":{"promptId":id}}),
        ),
        comfy::Remote::Lost => (
            "failed",
            json!([]),
            json!({"category":"vendorTaskFailed","message":"Original prompt is absent from the queue and history; it was not resubmitted","vendorCode":"PROMPT_NOT_FOUND","details":{"promptId":id}}),
        ),
    };
    let progress = if matches!(status, "queued" | "running") {
        json!({"fraction":null,"message":status})
    } else {
        Value::Null
    };
    Ok(
        json!({"status":status,"outputs":outputs,"data":{"promptId":id},"usage":null,"progress":progress,"terminalFailure":failure,"resumeToken":payload["resumeToken"]}),
    )
}
pub struct Seedvr2Image;
impl Adapter for Seedvr2Image {
    fn invoke(request: Invocation, host: &mut dyn Host) -> Response {
        let invocation = serde_json::to_value(&request).expect("typed invocation");
        let result = (|| {
            let payload: Value = serde_json::from_str(&request.r#payload_json)
                .map_err(|_| invalid("Payload is not JSON"))?;
            match invocation["operation"].as_str() {
                Some("capabilities") => Ok(capabilities()),
                Some("submit") => submit(host, &request, &payload),
                Some("query") => query(host, &request, &payload),
                Some("upload") => {
                    let context = comfy::Ctx {
                        config: &payload["configuration"],
                        budget_ms: request.r#budget_ms,
                    };
                    let reference = comfy::upload(host, &context, &payload["media"])?;
                    Ok(json!({"mediaId":payload["media"]["id"],"uploadRef":reference}))
                }
                Some("probe") => {
                    let checked = match payload["kind"].as_str() {
                        Some("package_validation") => false,
                        Some("connectivity") => {
                            comfy::status_probe(
                                host,
                                &comfy::Ctx {
                                    config: &payload["configuration"],
                                    budget_ms: request.r#budget_ms,
                                },
                            )?;
                            true
                        }
                        _ => return Err(invalid("Unknown probe kind")),
                    };
                    Ok(
                        json!({"packageValid":true,"connection":if checked {"ready"}else{"not_checked"},"authentication":"not_checked","resources":"not_checked","issues":[]}),
                    )
                }
                _ => Err(Failure::new(
                    "UNSUPPORTED_FEATURE",
                    "Transport operation is unsupported",
                )),
            }
        })();
        match result {
            Ok(value) => guest::completed_response(&request,&value).expect("serializable public result"),
            Err(error) => serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,"error":{
                "code":error.code,"message":error.message,"retryable":false,"vendorCode":error.vendor,"vendorRequestId":null}})).expect("public error"),
        }
    }
}
bgx_adapter_v2_sdk::export_adapter!(Seedvr2Image);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_graph_preserves_source_and_projects_confirmed_parameters() {
        let graph = graph(&json!({"parameters":{"targetResolution":512,"maxResolution":1024,"seed":17,"enhancementModel":"7b_fp8_mixed",
            "colorCorrection":"none","encodeTiled":true,"encodeTileSize":512,"encodeTileOverlap":64,"decodeTiled":true,"decodeTileSize":256,"decodeTileOverlap":32,
            "blocksToSwap":12,"swapIoComponents":true,"offloadDit":true,"offloadVae":true,"inputNoiseScale":0.2,"latentNoiseScale":0.4}}),"original.png").unwrap();
        assert_eq!(graph["16"]["inputs"]["image"], "original.png");
        assert_eq!(graph["10"]["inputs"]["resolution"], 512);
        assert_eq!(graph["10"]["inputs"]["batch_size"], 1);
        assert_eq!(graph["10"]["inputs"]["image"], json!(["16", 0]));
        assert_eq!(graph["10"]["inputs"]["seed"], 17);
        assert_eq!(graph["10"]["inputs"]["input_noise_scale"], 0.2);
        assert_eq!(
            graph["14"]["inputs"]["model"],
            "seedvr2_ema_7b_fp8_e4m3fn_mixed_block35_fp16.safetensors"
        );
        assert_eq!(graph["14"]["inputs"]["blocks_to_swap"], 12);
        assert_eq!(graph["14"]["inputs"]["swap_io_components"], true);
        assert_eq!(graph["14"]["inputs"]["offload_device"], "cpu");
        assert_eq!(graph["13"]["inputs"]["offload_device"], "cpu");
        assert_eq!(graph["13"]["inputs"]["decode_tile_overlap"], 32);
        assert_eq!(graph["15"]["inputs"]["images"], json!(["10", 0]));
        assert!(graph
            .as_object()
            .unwrap()
            .values()
            .all(|n| n["class_type"] != "LoadVideo"));
    }
    #[test]
    fn rejects_unconsumed_parameters_and_invalid_tile_pair() {
        assert!(graph(&json!({"parameters":{"writerCrf":18}}), "x").is_err());
        assert!(graph(
            &json!({"parameters":{"encodeTiled":true,"encodeTileSize":64,"encodeTileOverlap":64}}),
            "x"
        )
        .is_err());
    }
    #[test]
    fn one_image_operation_is_async_resumable_and_has_matching_schema_digests() {
        let caps = capabilities();
        let contract = &caps["capabilityContracts"][0];
        assert_eq!(contract["transportSlot"], "imageUpscale");
        assert_eq!(contract["operations"][0]["mediaSlots"][0]["maxCount"], 1);
        assert_eq!(contract["operations"][0]["execution"]["resumable"], true);
        for value in contract["schemas"].as_array().unwrap() {
            let source = value["schemaJson"].as_str().unwrap();
            let _: Value = serde_json::from_str(source).unwrap();
            assert_eq!(
                value["digest"],
                schema(value["id"].as_str().unwrap(), source)["digest"]
            );
        }
    }
}
