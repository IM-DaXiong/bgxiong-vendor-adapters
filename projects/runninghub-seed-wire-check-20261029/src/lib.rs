mod image_generation {
pub fn dimensions(size: &str) -> Result<(u32, u32), String> {
    let (w, h) = size.split_once('x').ok_or("图片操作要求明确的宽高")?;
    let w = w.parse::<u32>().map_err(|_| "图片宽度无效")?;
    let h = h.parse::<u32>().map_err(|_| "图片高度无效")?;
    if w == 0 || h == 0 {
        return Err("图片尺寸必须为正数".into());
    }
    Ok((w, h))
}
}
mod video_generation {
use serde::{Serialize,Deserialize};
use serde_json::{json,Map,Value};
/// Frozen package/operation identity with an explicit source discriminant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub plugin_id: String,
    pub version: String,
    pub model_id: String,
    pub capability_key: String,
    pub operation_id: String,
    pub operation_version: u64,
    pub input_schema_digest: String,
    pub source: SelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum SelectionSource {
    #[serde(rename = "installed")]
    Installed,
    #[serde(rename = "workflow")]
    Workflow {
        entry_id: String,
        source_revision: u64,
        source_digest: String,
    },
}

impl SelectionSource {
    pub fn installed() -> Self {
        Self::Installed
    }
    pub fn workflow(entry_id: impl Into<String>, source_revision: u64, source_digest: impl Into<String>) -> Self {
        Self::Workflow {
            entry_id: entry_id.into(),
            source_revision,
            source_digest: source_digest.into(),
        }
    }
}

/// Workbench-confirmed controls that travel with the frozen selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Parameters {
    pub selection: Selection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced: Option<Value>,
}

fn schema_fixed_value(prop: &Value) -> Option<Value> {
    if let Some(c) = prop.get("const") {
        return Some(c.clone());
    }
    if let Some(e) = prop.get("enum").and_then(Value::as_array) {
        if e.len() == 1 {
            return Some(e[0].clone());
        }
    }
    None
}

fn draft_has_key(draft: &Value, key: &str) -> bool {
    draft.as_object().is_some_and(|o| o.contains_key(key))
}

/// Unique owner that assembles §2 standard input. Fixed schema values always travel;
/// editable values come from the workbench draft / controls. Empty string, false, 0, and
/// empty arrays are preserved when the author declared the field.
pub fn assemble_standard_input(
    schema: &Value,
    prompt: &str,
    controls: &Parameters,
    draft: &Value,
) -> Result<Value, String> {
    let props = schema["properties"]
        .as_object()
        .ok_or("视频作者操作缺少输入字段声明")?;
    let mut input = Map::new();
    input.insert("prompt".into(), json!(prompt));

    if let Some(mode_prop) = props.get("inputMode") {
        if let Some(fixed) = schema_fixed_value(mode_prop) {
            input.insert("inputMode".into(), fixed);
        } else if draft_has_key(draft, "inputMode") || draft_has_key(draft, "intentInputMode") {
            let mode = draft
                .get("inputMode")
                .or_else(|| draft.get("intentInputMode"))
                .cloned()
                .ok_or("请选择视频模式")?;
            input.insert("inputMode".into(), mode);
        }
    }

    if props.contains_key("negativePrompt") {
        if draft_has_key(draft, "negativePrompt") {
            // Preserve explicit empty string.
            input.insert(
                "negativePrompt".into(),
                draft.get("negativePrompt").cloned().unwrap_or(json!("")),
            );
        } else if let Some(fixed) = schema_fixed_value(&props["negativePrompt"]) {
            input.insert("negativePrompt".into(), fixed);
        } else if let Some(d) = props["negativePrompt"].get("default") {
            input.insert("negativePrompt".into(), d.clone());
        }
    }

    for key in ["durationSeconds", "fps", "aspectRatio"] {
        if !props.contains_key(key) {
            continue;
        }
        if let Some(fixed) = schema_fixed_value(&props[key]) {
            input.insert(key.into(), fixed);
            continue;
        }
        if draft_has_key(draft, key) {
            let v = draft.get(key).cloned().unwrap_or(Value::Null);
            // Preserve 0 / false / "" when present as JSON values.
            input.insert(key.into(), v);
        } else if let Some(d) = props[key].get("default") {
            input.insert(key.into(), d.clone());
        } else {
            return Err(format!("请选择视频参数：{key}"));
        }
    }

    // Size representation: only the representation(s) declared by the package.
    if props.contains_key("width") && props.contains_key("height") {
        if let (Some(fw), Some(fh)) = (
            schema_fixed_value(&props["width"]),
            schema_fixed_value(&props["height"]),
        ) {
            input.insert("width".into(), fw);
            input.insert("height".into(), fh);
        } else if let Some(res) = draft.get("resolution").and_then(Value::as_str) {
            let (w, h) = super::image_generation::dimensions(res)?;
            input.insert("width".into(), json!(w));
            input.insert("height".into(), json!(h));
        } else if draft_has_key(draft, "width") && draft_has_key(draft, "height") {
            input.insert("width".into(), draft["width"].clone());
            input.insert("height".into(), draft["height"].clone());
        } else if let (Some(dw), Some(dh)) = (
            props["width"].get("default"),
            props["height"].get("default"),
        ) {
            input.insert("width".into(), dw.clone());
            input.insert("height".into(), dh.clone());
        } else {
            return Err("请选择视频尺寸".into());
        }
    }
    if props.contains_key("resolution") {
        if let Some(fixed) = schema_fixed_value(&props["resolution"]) {
            input.insert("resolution".into(), fixed);
        } else if draft_has_key(draft, "resolution") {
            input.insert("resolution".into(), draft["resolution"].clone());
        } else if let Some(d) = props["resolution"].get("default") {
            input.insert("resolution".into(), d.clone());
        } else {
            return Err("请选择分辨率档位".into());
        }
    }
    if props.contains_key("resolutionValue") {
        if let Some(fixed) = schema_fixed_value(&props["resolutionValue"]) {
            input.insert("resolutionValue".into(), fixed);
        } else if draft_has_key(draft, "resolutionValue") {
            input.insert("resolutionValue".into(), draft["resolutionValue"].clone());
        } else if let Some(d) = props["resolutionValue"].get("default") {
            input.insert("resolutionValue".into(), d.clone());
        } else {
            return Err("请选择数值清晰度".into());
        }
    }

    let required = |key: &str| schema["required"].as_array()
        .is_some_and(|fields| fields.iter().any(|field| field.as_str() == Some(key)));
    if props.contains_key("seed") {
        if let Some(seed) = controls.seed {
            input.insert("seed".into(), json!(seed));
        } else if required("seed") {
            return Err("请确认视频作者包声明的随机种子后提交".into());
        }
    }
    if props.contains_key("advanced") {
        if let Some(advanced) = &controls.advanced {
            if !advanced.is_object() {
                return Err("视频作者包高级参数须为对象".into());
            }
            input.insert("advanced".into(), advanced.clone());
        } else if required("advanced") {
            return Err("请确认视频作者包声明的高级参数后提交".into());
        }
    }
    Ok(Value::Object(input))
}

}

#[cfg(test)] mod tests {
use super::video_generation::*;
use serde_json::{json,Value};
fn selection()->Value { json!({"pluginId":"local.example.runninghub-minimax-h3","version":"1.0.2","modelId":"h3.r2v.4slot","capabilityKey":"video.generate","operationId":"classic","operationVersion":1,"inputSchemaDigest":"d".repeat(64),"source":{"kind":"installed"}}) }
#[test] fn real_seven_schemas_accept_absent_unexposed_controls_without_inventing_values(){
for name in ["ref1","slot2","slot3","slot4","i2v","fl2va","soundtrack"] {
let path=format!("L:/00Dev/bgxiong-ai-story/packages/cloud-adapters/runninghub-minimax-h3/schemas/input-{name}.json");
let schema:Value=serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
let mut wire=json!({"selection":selection()});
if name=="soundtrack" { wire["advanced"]=json!({"audioUsage":"soundtrack_copy"}); }
let parsed:Parameters=serde_json::from_value(wire.clone()).unwrap();
assert_eq!(parsed.seed,None);
assert_eq!(serde_json::to_value(&parsed).unwrap(),wire);
let actual=assemble_standard_input(&schema,"complete original",&parsed,&json!({"durationSeconds":15,"aspectRatio":"9:16","resolution":"mp:1"})).unwrap();
assert!(actual.get("seed").is_none());assert_eq!(actual.get("advanced").is_some(),name=="soundtrack");
assert_eq!(actual["resolution"],"mp:1");assert_eq!(actual["durationSeconds"],15);
}}
#[test] fn declared_controls_cannot_be_missing_and_explicit_zero_survives(){
let mut schema=json!({"required":["seed","advanced"],"properties":{"prompt":{"type":"string"},"seed":{"type":"integer","minimum":0,"maximum":99,"default":1},"advanced":{"type":"object","additionalProperties":false,"properties":{}}}});
let mut wire=json!({"selection":selection()});
let p:Parameters=serde_json::from_value(wire.clone()).unwrap();assert!(assemble_standard_input(&schema,"p",&p,&json!({})).is_err());
wire["seed"]=json!(0);let p:Parameters=serde_json::from_value(wire.clone()).unwrap();assert!(assemble_standard_input(&schema,"p",&p,&json!({})).is_err());
wire["advanced"]=json!({});let p:Parameters=serde_json::from_value(wire.clone()).unwrap();assert_eq!(assemble_standard_input(&schema,"p",&p,&json!({})).unwrap(),json!({"prompt":"p","seed":0,"advanced":{}}));
wire["seed"]=json!("0");assert!(serde_json::from_value::<Parameters>(wire.clone()).is_err());
wire["seed"]=json!(-1);assert!(serde_json::from_value::<Parameters>(wire.clone()).is_err());
wire["seed"]=json!(0);wire["advanced"]=json!("invalid");let p:Parameters=serde_json::from_value(wire).unwrap();assert!(assemble_standard_input(&schema,"p",&p,&json!({})).is_err());
schema["required"]=json!([]);
let p:Parameters=serde_json::from_value(json!({"selection":selection()})).unwrap();assert_eq!(assemble_standard_input(&schema,"p",&p,&json!({})).unwrap(),json!({"prompt":"p"}));
}
}
