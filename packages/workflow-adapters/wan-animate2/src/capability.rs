//! Capability declaration: one `video.edit` capability with one operation, character
//! animation (character image + driving video). Display text is catalog data only.
use alloc::{format, string::String, vec::Vec};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const PLUGIN_ID: &str = "local.bgx.wan-animate2";
pub const LOCAL_ID: &str = "animate";
pub const OPERATION_ID: &str = "local.bgx.wan-animate2/animate";

pub const INPUT_SCHEMA: &str = include_str!("../schemas/input.schema.json");
pub const OUTPUT_SCHEMA: &str = include_str!("../schemas/output.schema.json");
pub const CONFIG_SCHEMA: &str = include_str!("../schemas/config.schema.json");
pub const RESUME_SCHEMA: &str = include_str!("../schemas/resume.schema.json");

fn digest(source: &str) -> String {
    Sha256::digest(source.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
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

fn controls() -> Value {
    let mut c: Vec<Value> = Vec::new();
    for (p, w, l) in [
        ("/source", "media", "驱动视频（动作来源）"),
        ("/sourceRange", "time_span", "处理范围"),
        ("/replacement", "media", "角色图片（外观来源）"),
        ("/prompt", "multiline", "角色外观与背景描述"),
        ("/negativePrompt", "multiline", "负面提示词"),
        ("/width", "integer", "输出宽度"),
        ("/height", "integer", "输出高度"),
        ("/fps", "integer", "帧率"),
        ("/audio", "select", "音频"),
    ] {
        c.push(control(p, w, l, "基础"));
    }
    for (p, w, l) in [
        ("/advanced/motionPrompt", "multiline", "动作描述"),
        ("/advanced/segmentFrames", "integer", "每段帧数"),
        ("/advanced/seed", "integer", "种子"),
        ("/advanced/steps", "integer", "步数"),
        ("/advanced/cfg", "number", "CFG"),
        ("/advanced/sampler", "select", "采样器"),
        ("/advanced/scheduler", "select", "调度器"),
        ("/advanced/samplingShift", "number", "采样偏移"),
        ("/advanced/poseStrength", "number", "动作跟随强度"),
        ("/advanced/poseStartPercent", "number", "动作影响起点"),
        ("/advanced/poseEndPercent", "number", "动作影响终点"),
        ("/advanced/referenceImageStrength", "number", "角色外观保持强度"),
        ("/advanced/contextWindows", "toggle", "长片上下文窗口"),
        ("/advanced/cacheDevice", "select", "动作缓存位置"),
        ("/advanced/cacheDtype", "select", "动作缓存精度"),
    ] {
        c.push(control(p, w, l, "高级"));
    }
    Value::Array(c)
}

pub fn capabilities() -> Value {
    json!({"schemaVersion": 2, "pluginId": PLUGIN_ID, "pluginVersion": env!("CARGO_PKG_VERSION"), "capabilityContracts": [{
        "schemaVersion": 2, "localId": LOCAL_ID, "capabilityKey": "video.edit", "transportSlot": "video",
        "modelIds": ["wan-animate-2-distilled"],
        "configurationSchema": reference("config", CONFIG_SCHEMA),
        "operations": [{
            "id": OPERATION_ID, "version": 1, "label": "角色动画（动作迁移）",
            "inputSchema": reference("input", INPUT_SCHEMA),
            "outputSchema": reference("output", OUTPUT_SCHEMA),
            "resumeSchema": reference("resume", RESUME_SCHEMA),
            "uiSchema": {"controls": controls()},
            "mediaSlots": [
                {"role": "source", "inputPointer": "/source", "mediaKind": "video", "minCount": 1, "maxCount": 1},
                {"role": "replacement", "inputPointer": "/replacement", "mediaKind": "image", "minCount": 1, "maxCount": 1}
            ],
            "outputSlots": [{"role": "video", "kind": "video", "minCount": 1, "maxCount": 1}],
            "execution": {"mode": "asynchronous", "streaming": false, "cancellation": "unsupported", "resumable": true, "remoteIdempotency": false}
        }],
        "schemas": [schema("input", INPUT_SCHEMA), schema("output", OUTPUT_SCHEMA), schema("config", CONFIG_SCHEMA), schema("resume", RESUME_SCHEMA)]
    }]})
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn schemas_are_json_digests_match_and_defaults_parse() {
        let caps = super::capabilities();
        let c = &caps["capabilityContracts"][0];
        let schemas = c["schemas"].as_array().unwrap();
        for s in schemas {
            let _: Value = serde_json::from_str(s["schemaJson"].as_str().unwrap()).unwrap();
        }
        let op = &c["operations"][0];
        let s = schemas.iter().find(|s| s["id"] == op["inputSchema"]["id"]).unwrap();
        assert_eq!(op["inputSchema"]["digest"], s["digest"]);
        let parsed: Value = serde_json::from_str(s["schemaJson"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["x-resourceHint"]["blocking"], false);
        assert_eq!(parsed["properties"]["source"]["x-mediaKind"], "video");
        assert_eq!(parsed["properties"]["replacement"]["x-mediaKind"], "image");
        // Every declared default must be accepted by the strict parser (no hidden fill-in).
        let props = &parsed["properties"];
        let adv = props["advanced"]["properties"].as_object().unwrap();
        let mut input = crate::parameters::tests::sample();
        for (k, v) in adv {
            input["advanced"][k] = v["default"].clone();
        }
        for k in ["width", "height", "fps", "audio", "negativePrompt"] {
            input[k] = props[k]["default"].clone();
        }
        crate::parameters::parse(&input).ok().expect("schema defaults are legal");
        let required: alloc::vec::Vec<&str> = props["advanced"]["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(required.len(), adv.len(), "every advanced key is required and declared");
    }
}