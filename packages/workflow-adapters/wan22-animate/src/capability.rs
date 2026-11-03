//! Capability declaration: video.edit with Mix, Move, and tracking-preview operations.
//! Display text is catalog data, not a code branch key.
use alloc::{format, string::String, vec::Vec};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const PLUGIN_ID: &str = "local.bgx.wan22-animate";
pub const LOCAL_ID: &str = "animate";
pub const OPERATION_ID: &str = "local.bgx.wan22-animate/character-replace";
pub const MOVE_OPERATION_ID: &str = "local.bgx.wan22-animate/move";
pub const TRACKING_OPERATION_ID: &str = "local.bgx.wan22-animate/tracking-preview";

pub const INPUT_SCHEMA: &str = include_str!("../schemas/input.schema.json");
pub const MOVE_INPUT_SCHEMA: &str = include_str!("../schemas/input-move.schema.json");
pub const TRACKING_INPUT_SCHEMA: &str = include_str!("../schemas/tracking.input.schema.json");
pub const TRACKING_OUTPUT_SCHEMA: &str = include_str!("../schemas/tracking.output.schema.json");
pub const OUTPUT_SCHEMA: &str = include_str!("../schemas/output.schema.json");
pub const CONFIG_SCHEMA: &str = include_str!("../schemas/config.schema.json");
pub const TRACKING_RESUME_SCHEMA: &str = include_str!("../schemas/tracking-resume.schema.json");
pub const RESUME_SCHEMA: &str = include_str!("../schemas/resume.schema.json");

pub const MIX_OPERATION_VERSION: &str = "3.0.0";
pub const MOVE_OPERATION_VERSION: &str = "1.0.0";
pub const TRACKING_OPERATION_VERSION: &str = "1.0.0";
/// The public tracking input carries the source as `sourceIdentity.mediaId`.
pub const TRACKING_SOURCE_POINTER: &str = "/sourceIdentity/mediaId";

fn digest(source: &str) -> String {
    Sha256::digest(source.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn schema(id: &str, source: &str) -> Value {
    json!({"id": id, "digest": digest(source), "schemaJson": source})
}
fn reference(id: &str, source: &str) -> Value {
    json!({"id": id, "digest": digest(source)})
}
fn control(pointer: &str, widget: &str, label: &str, group: &str) -> Value {
    json!({"fieldPointer": pointer, "widget": widget, "label": label, "description": "", "group": group})
}

fn controls(replace: bool) -> Value {
    let mut c: Vec<Value> = Vec::new();
    c.push(control("/source", "media", "源视频", "基础"));
    c.push(control("/sourceRange", "time_span", "处理范围", "基础"));
    c.push(control(
        "/replacement",
        "media",
        if replace {
            "替换人物图片"
        } else {
            "驱动人物图片"
        },
        "基础",
    ));
    if replace {
        c.push(control(
            "/targetPoints/positive",
            "repeat",
            "人物选择（暂停到清晰帧点选）",
            "基础",
        ));
    }
    for (p, w, l) in [
        ("/prompt", "multiline", "提示词"),
        ("/negativePrompt", "multiline", "负面提示词"),
        ("/width", "integer", "输出宽度"),
        ("/height", "integer", "输出高度"),
        ("/fps", "integer", "帧率"),
        ("/audio", "select", "音频"),
    ] {
        c.push(control(p, w, l, "基础"));
    }
    for (p, w, l) in [
        ("/advanced/windowFrames", "integer", "每段帧数"),
        ("/advanced/overlapFrames", "integer", "重叠帧数"),
        ("/advanced/seed", "integer", "种子"),
        ("/advanced/steps", "integer", "步数"),
        ("/advanced/cfg", "number", "CFG"),
        ("/advanced/sampler", "select", "采样器"),
        ("/advanced/scheduler", "select", "调度器"),
        ("/advanced/samplingShift", "number", "采样偏移"),
        ("/advanced/distillLoraStrength", "number", "蒸馏 LoRA 强度"),
        (
            "/advanced/relightLoraStrength",
            "number",
            "重打光 LoRA 强度",
        ),
    ] {
        c.push(control(p, w, l, "高级"));
    }
    if replace {
        c.push(control(
            "/advanced/maskGrowPixels",
            "integer",
            "遮罩外扩像素",
            "高级",
        ));
    }
    Value::Array(c)
}

/// Every required field of the public tracking input; the host fills them from the workbench.
fn tracking_controls() -> Value {
    Value::Array(alloc::vec![
        control("/sourceIdentity", "media", "源视频", "基础"),
        control("/range", "time_span", "处理范围", "基础"),
        control("/selection", "point", "关键帧选择", "基础"),
        control("/direction", "select", "跟踪方向", "基础"),
        control("/anchorKeyframeId", "text", "起始关键帧", "基础"),
        control("/requestedCoverage", "time_span", "跟踪覆盖范围", "基础"),
        control("/requestDigest", "text", "请求摘要", "基础"),
    ])
}

fn operation(id: &str, version: u64, label: &str, input_id: &str, input: &str, replace: bool) -> Value {
    json!({
        "id": id, "version": version, "label": label,
        "inputSchema": reference(input_id, input),
        "outputSchema": reference("output", OUTPUT_SCHEMA),
        "resumeSchema": reference("resume", RESUME_SCHEMA),
        "uiSchema": {"controls": controls(replace)},
        "mediaSlots": [
            {"role": "source", "inputPointer": "/source", "mediaKind": "video", "minCount": 1, "maxCount": 1},
            {"role": "replacement", "inputPointer": "/replacement", "mediaKind": "image", "minCount": 1, "maxCount": 1}
        ],
        "outputSlots": [{"role": "video", "kind": "video", "minCount": 1, "maxCount": 1}],
        "execution": {"mode": "asynchronous", "streaming": false, "cancellation": "unsupported", "resumable": true, "remoteIdempotency": false}
    })
}

fn tracking_operation() -> Value {
    json!({
        "id": TRACKING_OPERATION_ID,
        "version": 1,
        "label": "目标跟踪预览",
        "inputSchema": reference("tracking-input", TRACKING_INPUT_SCHEMA),
        "outputSchema": reference("tracking-output", TRACKING_OUTPUT_SCHEMA),
        "resumeSchema": reference("tracking-resume", TRACKING_RESUME_SCHEMA),
        "uiSchema": {"controls": tracking_controls()},
        "mediaSlots": [
            {"role": "source", "inputPointer": TRACKING_SOURCE_POINTER, "mediaKind": "video", "minCount": 1, "maxCount": 1}
        ],
        "outputSlots": [
            {"role": "frameIndex", "kind": "file", "minCount": 1, "maxCount": 1},
            {"role": "framePage", "kind": "file", "minCount": 1, "maxCount": null},
            {"role": "maskFrame", "kind": "image", "minCount": 0, "maxCount": null}
        ],
        "execution": {"mode": "asynchronous", "streaming": true, "cancellation": "remote", "resumable": true, "remoteIdempotency": false}
    })
}

pub fn capabilities() -> Value {
    json!({"schemaVersion": 2, "pluginId": PLUGIN_ID, "pluginVersion": env!("CARGO_PKG_VERSION"), "capabilityContracts": [{
        "schemaVersion": 2, "localId": LOCAL_ID, "capabilityKey": "video.edit", "transportSlot": "video",
        "modelIds": ["wan2.2-animate-14b"],
        "configurationSchema": reference("config", CONFIG_SCHEMA),
        "operations": [
            operation(OPERATION_ID, 3, "人物替换", "input", INPUT_SCHEMA, true),
            operation(MOVE_OPERATION_ID, 1, "驱动表演", "input-move", MOVE_INPUT_SCHEMA, false),
            tracking_operation()
        ],
        "schemas": [
            schema("input", INPUT_SCHEMA),
            schema("input-move", MOVE_INPUT_SCHEMA),
            schema("tracking-input", TRACKING_INPUT_SCHEMA),
            schema("tracking-output", TRACKING_OUTPUT_SCHEMA),
            schema("output", OUTPUT_SCHEMA),
            schema("config", CONFIG_SCHEMA),
            schema("resume", RESUME_SCHEMA),
            schema("tracking-resume", TRACKING_RESUME_SCHEMA)
        ]
    }]})
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn tracking_acceptance_and_resume_validate_against_the_published_contract() {
        let caps = super::capabilities();
        let contract = serde_json::from_value(caps["capabilityContracts"][0].clone()).unwrap();
        for direction in ["towardStart", "towardEnd", "both"] {
            let accepted = crate::tracking::accepted_result("prompt-1", "request-1", "digest-1", direction);
            let result = serde_json::from_value(accepted.clone()).unwrap();
            bgx_adapter_v2_validator::result::validate_declared_result(
                &contract, super::TRACKING_OPERATION_ID, 1, "submit", &result,
            ).unwrap();
            let mut invalid = accepted;
            invalid["resumeToken"].as_object_mut().unwrap().remove("trackingRequestId");
            let invalid = serde_json::from_value(invalid).unwrap();
            assert!(bgx_adapter_v2_validator::result::validate_declared_result(
                &contract, super::TRACKING_OPERATION_ID, 1, "submit", &invalid,
            ).is_err());
        }

        for retired_direction in ["forward", "backward"] {
            let accepted = crate::tracking::accepted_result("prompt-1", "request-1", "digest-1", retired_direction);
            let invalid = serde_json::from_value(accepted).unwrap();
            assert!(bgx_adapter_v2_validator::result::validate_declared_result(
                &contract, super::TRACKING_OPERATION_ID, 1, "submit", &invalid,
            ).is_err());
        }

    }

    #[test]
    fn schemas_are_json_and_digests_match() {
        let caps = super::capabilities();
        let c = &caps["capabilityContracts"][0];
        let schemas = c["schemas"].as_array().unwrap();
        for s in schemas {
            let _: Value = serde_json::from_str(s["schemaJson"].as_str().unwrap()).unwrap();
        }
        for op in c["operations"].as_array().unwrap() {
            let r = &op["inputSchema"];
            let s = schemas.iter().find(|s| s["id"] == r["id"]).unwrap();
            assert_eq!(r["digest"], s["digest"]);
        }
        let mv: Value = serde_json::from_str(super::MOVE_INPUT_SCHEMA).unwrap();
        assert!(mv["properties"].get("targetPoints").is_none());
        assert!(mv.get("x-targetTracking").is_none(), "Move must not fake tracking support");
        let ops = c["operations"].as_array().unwrap();
        let mix = ops.iter().find(|o| o["id"] == super::OPERATION_ID).unwrap();
        let mov = ops.iter().find(|o| o["id"] == super::MOVE_OPERATION_ID).unwrap();
        let trk = ops.iter().find(|o| o["id"] == super::TRACKING_OPERATION_ID).unwrap();
        assert_eq!(mix["version"], 3);
        assert_eq!(mov["version"], 1);
        assert_eq!(trk["version"], 1);
        assert_eq!(trk["execution"]["cancellation"], "remote");
        let roles: Vec<(&str, &str)> = trk["outputSlots"].as_array().unwrap().iter()
            .map(|s| (s["role"].as_str().unwrap(), s["kind"].as_str().unwrap())).collect();
        assert_eq!(roles, [("frameIndex", "file"), ("framePage", "file"), ("maskFrame", "image")]);
        let parsed: Value = serde_json::from_str(super::INPUT_SCHEMA).unwrap();
        let xt = &parsed["x-targetTracking"];
        assert_eq!(xt["profileId"], "video-target-keyframes-v1");
        assert_eq!(xt["revision"], 1);
        assert_eq!(
            xt["roles"]["trackingPreview"]["operationId"],
            super::TRACKING_OPERATION_ID
        );
        assert_eq!(
            xt["roles"]["generate"]["operationId"],
            super::OPERATION_ID
        );
        assert_eq!(xt["roles"]["generate"]["operationVersion"], "3.0.0");
        assert_eq!(xt["roles"]["trackingPreview"]["operationVersion"], "1.0.0");
        assert_eq!(
            xt["roles"]["trackingPreview"]["inputSchemaDigest"],
            trk["inputSchema"]["digest"],
            "trackingPreview role must name the exported tracking schema"
        );
        assert_eq!(trk["mediaSlots"][0]["inputPointer"], super::TRACKING_SOURCE_POINTER);
        let tracking: Value = serde_json::from_str(super::TRACKING_INPUT_SCHEMA).unwrap();
        for retired in ["source", "sourceRange", "width", "height", "fps"] {
            assert!(tracking["properties"].get(retired).is_none(), "{retired}");
        }
        assert!(tracking["$defs"]["selection"]["properties"].get("sourceIdentity").is_some());
        assert_eq!(xt["referenceChangeInvalidatesTracking"], false);
        assert!(parsed["properties"].get("maskArtifact").is_some());
        let tools = xt["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t == "positivePoint"));
        assert!(tools.iter().any(|t| t == "negativePoint"));
        assert!(tools.iter().any(|t| t == "box"));
        let dirs = xt["directions"].as_array().unwrap();
        assert_eq!(dirs.len(), 3);
    }
}
