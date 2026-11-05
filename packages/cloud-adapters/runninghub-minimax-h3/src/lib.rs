#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
pub const PLUGIN: &str = "local.example.runninghub-minimax-h3";
type Result<T> = core::result::Result<T, Failure>;
#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
    vendor: Option<String>,
    request: Option<String>,
    retryable: bool,
}
fn fail(code: &'static str, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
        vendor: None,
        request: None,
        retryable: false,
    }
}
fn invalid(s: &str) -> Failure {
    fail("INVALID_INPUT", s)
}
fn malformed() -> Failure {
    fail(
        "UPSTREAM_ERROR",
        "RunningHub返回不完整或未知结构；未自动重试",
    )
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().ok_or_else(malformed)
}
fn port(h: &mut dyn Host, m: &str, a: Value) -> Result<Value> {
    let value: Value = host::call(h, m, &a).map_err(|e| {
        let v = serde_json::to_value(&e).expect("public error");
        let code = match v["code"].as_str() {
            Some("PERMISSION_DENIED") => "PERMISSION_DENIED",
            Some("TIMEOUT") => "TIMEOUT",
            Some("BUDGET_EXCEEDED") => "BUDGET_EXCEEDED",
            _ => "RESOURCE_UNAVAILABLE",
        };
        fail(code, format!("受管端口{m}失败：{}", host::diagnostic(&e)))
    })?;
    let closed = match m {
        "host-http.execute" => serde_json::from_value::<
            bgx_adapter_v2_sdk::HostPortCallV2HostHttpExecuteResult,
        >(value.clone())
        .map(|_| ()),
        "host-media.info" => serde_json::from_value::<
            bgx_adapter_v2_sdk::HostPortCallV2HostMediaInfoResult,
        >(value.clone())
        .map(|_| ()),
        "host-checkpoint.read" => serde_json::from_value::<
            bgx_adapter_v2_sdk::HostPortCallV2HostCheckpointReadResult,
        >(value.clone())
        .map(|_| ()),
        "host-checkpoint.write" => serde_json::from_value::<
            bgx_adapter_v2_sdk::HostPortCallV2HostCheckpointWriteResult,
        >(value.clone())
        .map(|_| ()),
        _ => return Err(invalid("未声明的受管端口")),
    };
    closed.map_err(|_| fail("UPSTREAM_ERROR", format!("受管端口{m}回执不符合公开DTO")))?;
    Ok(value)
}
fn schema(id: &str, s: &str) -> Value {
    let normalized = s.replace("\r\n", "\n");
    let s = normalized.as_str();
    let d: String = Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({"id":id,"digest":d,"schemaJson":s})
}

mod config;
mod mapping;
use mapping::ModelKind;
const MODELS: [ModelKind; 7] = [
    ModelKind::Ref1,
    ModelKind::Slot2,
    ModelKind::Slot3,
    ModelKind::Slot4,
    ModelKind::I2vTurbo,
    ModelKind::Fl2va,
    ModelKind::Soundtrack,
];
impl ModelKind {
    fn slug(self) -> &'static str {
        match self {
            Self::Ref1 => "ref1",
            Self::Slot2 => "slot2",
            Self::Slot3 => "slot3",
            Self::Slot4 => "slot4",
            Self::I2vTurbo => "i2v",
            Self::Fl2va => "fl2va",
            Self::Soundtrack => "soundtrack",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Ref1 => "1 图",
            Self::Slot2 => "2 图",
            Self::Slot3 => "3 图",
            Self::Slot4 => "4 图",
            Self::I2vTurbo => "单图参考 Turbo",
            Self::Fl2va => "首帧视频 · 20 步",
            Self::Soundtrack => "参考音对口型 · 1 图",
        }
    }
    fn operation_id(self) -> String {
        format!("{PLUGIN}/{}", self.slug())
    }
    fn input_schema(self) -> &'static str {
        match self {
            Self::Ref1 => include_str!("../schemas/input-ref1.json"),
            Self::Slot2 => include_str!("../schemas/input-slot2.json"),
            Self::Slot3 => include_str!("../schemas/input-slot3.json"),
            Self::Slot4 => include_str!("../schemas/input-slot4.json"),
            Self::I2vTurbo => include_str!("../schemas/input-i2v.json"),
            Self::Fl2va => include_str!("../schemas/input-fl2va.json"),
            Self::Soundtrack => include_str!("../schemas/input-soundtrack.json"),
        }
    }
}
pub fn capabilities() -> Value {
    let contracts:Vec<Value>=MODELS.iter().map(|&kind|{
  let i=schema("input",kind.input_schema());let o=schema("output",include_str!("../schemas/output.json"));let c=schema("configuration",include_str!("../schemas/config.json"));let r=schema("resume",include_str!("../schemas/resume.json"));
  let mut media=alloc::vec![json!({"role":"reference_image","inputPointer":"/referenceImages","mediaKind":"image","minCount":kind.max_refs(),"maxCount":kind.max_refs()})];
  if kind.requires_reference_audio(){media.push(json!({"role":"reference_audio","inputPointer":"/referenceAudios","mediaKind":"audio","minCount":1,"maxCount":1}));}
  let controls:Vec<Value>=[("prompt","multiline","提示词原文"),("aspectRatio","select","画幅"),("resolution","select","清晰度（MP）"),("durationSeconds","number","时长（1～15秒）")].iter().map(|(f,w,l)|json!({"fieldPointer":format!("/{f}"),"widget":w,"label":l,"description":"用户实际值原样提交，平台拒绝显式回传","group":"视频输入"})).collect();
  json!({"schemaVersion":2,"localId":kind.slug(),"capabilityKey":"video.generate","transportSlot":"video","modelIds":[kind.model_id()],"configurationSchema":{"id":"configuration","digest":c["digest"]},"schemas":[i.clone(),o.clone(),c,r.clone()],"operations":[{"id":kind.operation_id(),"version":1,"label":kind.label(),"inputSchema":{"id":"input","digest":i["digest"]},"outputSchema":{"id":"output","digest":o["digest"]},"resumeSchema":{"id":"resume","digest":r["digest"]},"uiSchema":{"controls":controls},"mediaSlots":media,"outputSlots":[{"role":"video","kind":"video","minCount":1,"maxCount":1}],"execution":{"mode":"asynchronous","streaming":false,"cancellation":"unsupported","resumable":true,"remoteIdempotency":false}}]})
 }).collect();
    json!({"schemaVersion":2,"pluginId":PLUGIN,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":contracts})
}
fn identity(p: &Value, submit: bool) -> Result<ModelKind> {
    let kind = mapping::parse_model_kind(p["modelId"].as_str())
        .map_err(|_| invalid("未声明的模型身份"))?;
    if p["schemaVersion"] != 2
        || p["capabilityKey"] != "video.generate"
        || p["operationId"] != kind.operation_id()
        || p["operationVersion"] != 1
    {
        return Err(invalid("模型、能力或操作身份不匹配"));
    }
    if submit && p["inputSchemaDigest"] != schema("input", kind.input_schema())["digest"] {
        return Err(invalid("输入schema摘要不匹配"));
    }
    Ok(kind)
}
fn configuration(p: &Value) -> Result<&Value> {
    let c = &p["configuration"];
    if c.as_object().is_none_or(|o| o.len() != 3)
        || c["endpointBaseUrl"] != "https://www.runninghub.cn"
        || text(c, "credentialRef")?.is_empty()
        || !["default", "plus", "ultra"].contains(&text(c, "instanceType")?)
    {
        return Err(invalid("须配置中国站、受管凭据和实例类型"));
    }
    Ok(c)
}
fn refs<'a>(i: &'a Value, f: &str, n: usize) -> Result<&'a Vec<Value>> {
    let a = i[f].as_array().ok_or_else(|| invalid("参考媒体须为数组"))?;
    if a.len() != n
        || a.iter().any(|v| {
            v.as_object().is_none_or(|o| o.len() != 1)
                || !v["mediaId"].as_str().is_some_and(|s| !s.is_empty())
        })
    {
        return Err(invalid("素材数须与所选工作流的固定槽数一致"));
    }
    Ok(a)
}
fn validate_input(kind: ModelKind, i: &Value) -> Result<()> {
    let allowed = [
        "inputMode",
        "prompt",
        "referenceImages",
        "durationSeconds",
        "fps",
        "aspectRatio",
        "resolution",
    ];
    let o = i.as_object().ok_or_else(|| invalid("input须为对象"))?;
    if o.len() != allowed.len() + 2 * usize::from(kind.requires_reference_audio())
        || o.keys().any(|k| {
            !allowed.contains(&k.as_str())
                && !(kind.requires_reference_audio() && matches!(k.as_str(), "referenceAudios" | "advanced"))
        })
    {
        return Err(invalid("输入字段缺失或未声明"));
    }
    if i["inputMode"] != "multi_image_to_video" || text(i, "prompt")?.is_empty() {
        return Err(invalid("输入模式或提示词无效"));
    }
    refs(i, "referenceImages", kind.max_refs())?;
    if kind.requires_reference_audio() {
        refs(i, "referenceAudios", 1)?;
        if i["advanced"] != json!({"audioUsage":"soundtrack_copy"}) {
            return Err(invalid("本工作流仅支持原音驱动对口型 soundtrack_copy"));
        }
    }
    if i["durationSeconds"]
        .as_u64()
        .is_none_or(|v| !(1..=15).contains(&v))
        || i["fps"] != 24
    {
        return Err(invalid("时长须1～15整数秒，模型原生帧率固定24"));
    }
    mapping::resolution_combo(text(i, "aspectRatio")?).map_err(|_| invalid("画幅不在声明范围"))?;
    if !["mp:0.25", "mp:0.4", "mp:0.6", "mp:1"].contains(&text(i, "resolution")?) {
        return Err(invalid("清晰度须0.25、0.4、0.6或1MP"));
    }
    Ok(())
}
fn selected<'a>(p: &'a Value) -> Result<Vec<(&'static str, &'a Value)>> {
    let kind = identity(p, true)?;
    validate_input(kind, &p["input"])?;
    let media = p["media"]
        .as_array()
        .ok_or_else(|| invalid("媒体租约缺失"))?;
    let mut result = Vec::new();
    for (f, k, n) in [
        ("referenceImages", "image", kind.max_refs()),
        (
            "referenceAudios",
            "audio",
            if kind.requires_reference_audio() {
                1
            } else {
                0
            },
        ),
    ] {
        if n == 0 {
            continue;
        }
        for r in refs(&p["input"], f, n)? {
            let found: Vec<_> = media.iter().filter(|m| m["id"] == r["mediaId"]).collect();
            if found.len() != 1 || found[0]["kind"] != k {
                return Err(invalid("媒体身份或分类不匹配"));
            }
            let m = found[0];
            let mime = text(m, "mime")?;
            let accepted = if k == "image" {
                ["image/png", "image/jpeg", "image/webp"].contains(&mime)
            } else {
                ["audio/wav", "audio/mpeg", "audio/flac"].contains(&mime)
            };
            if !accepted
                || text(m, "handle")?.is_empty()
                || text(m, "fileName")?.is_empty()
                || text(m, "byteLength")?
                    .parse::<u64>()
                    .ok()
                    .is_none_or(|n| n == 0)
            {
                return Err(invalid("原件格式、长度或句柄无效"));
            }
            result.push((k, m));
        }
    }
    if media
        .iter()
        .any(|m| !result.iter().any(|(_, n)| n["id"] == m["id"]))
    {
        return Err(invalid("存在未被引用的媒体"));
    }
    Ok(result)
}
fn http(
    h: &mut dyn Host,
    r: &Invocation,
    c: &Value,
    method: &str,
    url: String,
    body: Value,
    download: bool,
) -> Result<Value> {
    port(
        h,
        "host-http.execute",
        json!({"plan":{"method":method,"url":url,"headers":if body["kind"]=="bytes"{json!([{"name":"Content-Type","value":"application/json"}])}else{json!([])},"generatedHeaders":if download{json!([])}else{json!([{"name":"Authorization","kind":"bearer","targetField":"Authorization"}])},"body":body,"sink":if download{json!({"kind":"media"})}else{json!({"kind":"buffer","maxBytes":1048576})},"timeoutMs":r.r#budget_ms},"credential":if download{Value::Null}else{c["credentialRef"].clone()}}),
    )
}
fn json_body(v: &Value) -> Result<Value> {
    let b = serde_json::to_vec(v).map_err(|_| invalid("请求JSON无效"))?;
    if b.len() > 1048576 {
        return Err(fail(
            "BUDGET_EXCEEDED",
            "请求超过公开SDK的1MiB缓冲预算，未截断提示词",
        ));
    }
    Ok(json!({"kind":"bytes","bytes":b}))
}
fn unwrap_response(r: &Value) -> Result<Value> {
    let status = r["status"].as_u64().ok_or_else(malformed)?;
    let bytes: Vec<u8> =
        serde_json::from_value(r["payload"]["bytes"].clone()).map_err(|_| malformed())?;
    if !(200..300).contains(&status) {
        return Err(fail(
            match status {
                401 | 403 => "UNAUTHORIZED",
                429 => "RATE_LIMITED",
                _ => "UPSTREAM_ERROR",
            },
            format!("RunningHub HTTP {status}，未自动重试"),
        ));
    }
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| malformed())?;
    if let Some(code) = v.get("code") {
        if code != 0 {
            let mut e = fail(
                "UPSTREAM_ERROR",
                v["msg"].as_str().unwrap_or("RunningHub拒绝请求"),
            );
            e.vendor = Some(code.to_string());
            return Err(e);
        }
    }
    if let Some(code) = v.get("errorCode") {
        let s = code.as_str().ok_or_else(malformed)?;
        if !s.is_empty() && s != "0" && v["status"] != "FAILED" {
            let mut e = fail(
                "UPSTREAM_ERROR",
                v["errorMessage"].as_str().unwrap_or("RunningHub拒绝请求"),
            );
            e.vendor = Some(s.into());
            return Err(e);
        }
    }
    Ok(v)
}
fn upload(h: &mut dyn Host, r: &Invocation, c: &Value, m: &Value) -> Result<String> {
    let stat = port(h, "host-media.info", json!({"handle":m["handle"]}))?;
    for key in ["kind", "mime", "fileName", "byteLength"] {
        if stat[key] != m[key] {
            return Err(invalid("原件与冻结描述不一致"));
        }
    }
    let v = unwrap_response(&http(
        h,
        r,
        c,
        "POST",
        "https://www.runninghub.cn/openapi/v2/media/upload/binary".into(),
        json!({"kind":"multipart","parts":[{"kind":"file","name":"file","fileName":m["fileName"],"handle":m["handle"],"mime":m["mime"]}]}),
        false,
    )?)?;
    if v["code"] != 0 {
        return Err(malformed());
    }
    let name = text(&v["data"], "fileName")?;
    if name.is_empty() || name.starts_with("https:") || name.starts_with("http:") {
        return Err(invalid(
            "上传须返回Comfy相对fileName，不能以download_url代替",
        ));
    }
    Ok(name.into())
}
fn persist(h: &mut dyn Host, r: &Invocation, a: &Value) -> Result<()> {
    let read = port(
        h,
        "host-checkpoint.read",
        json!({"requestId":r.r#request_id}),
    )?;
    let rev = text(&read, "revision")?;
    let next = rev
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(malformed)?;
    let mut v: Value = serde_json::from_str(text(&read, "valueJson")?).map_err(|_| malformed())?;
    v["revision"] = json!(next.to_string());
    v["state"] = json!("accepted");
    v["vendorTaskId"] = a["vendorTaskId"].clone();
    v["resumeToken"] = a["resumeToken"].clone();
    v["result"] = a.clone();
    let saved = port(
        h,
        "host-checkpoint.write",
        json!({"requestId":r.r#request_id,"expectedRevision":rev,"valueJson":v.to_string()}),
    )?;
    if saved["revision"] != next.to_string() {
        return Err(fail("UPSTREAM_ERROR", "受理快照修订回执不一致"));
    }
    Ok(())
}

fn node_info(kind: ModelKind, i: &Value, images: &[String], audios: &[String]) -> Result<Value> {
    validate_input(kind, i)?;
    let names: Vec<&str> = images.iter().map(String::as_str).collect();
    let fps = if kind.widgets().fps_node.is_empty() {
        None
    } else {
        Some(24.0)
    };
    mapping::mapped_node_info(
        kind,
        text(i, "prompt")?,
        i["durationSeconds"].as_f64(),
        fps,
        Some(text(i, "resolution")?),
        Some(text(i, "aspectRatio")?),
        &names,
        audios.first().map(String::as_str),
    )
    .map_err(|e| fail("INVALID_INPUT", e))
}
fn task_id(v: &Value) -> Result<String> {
    match &v["taskId"] {
        Value::String(s) if !s.is_empty() => Ok(s.clone()),
        Value::Number(n) if n.as_u64().is_some() => Ok(n.to_string()),
        _ => Err(malformed()),
    }
}
fn submit(h: &mut dyn Host, r: &Invocation, p: &Value) -> Result<Value> {
    let kind = identity(p, true)?;
    let chosen = selected(p)?;
    let c = configuration(p)?;
    let preview_images = (0..kind.max_refs())
        .map(|_| String::from("pending.png"))
        .collect::<Vec<_>>();
    let preview_audios = if kind.requires_reference_audio() {
        alloc::vec![String::from("pending.wav")]
    } else {
        Vec::new()
    };
    let body_for = |nodes: Value| json!({"nodeInfoList":nodes,"instanceType":c["instanceType"],"usePersonalQueue":false});
    json_body(&body_for(node_info(
        kind,
        &p["input"],
        &preview_images,
        &preview_audios,
    )?))?;
    let mut images = Vec::new();
    let mut audios = Vec::new();
    for (k, m) in chosen {
        let n = upload(h, r, c, m)?;
        if k == "image" {
            images.push(n)
        } else {
            audios.push(n)
        }
    }
    let response = http(
        h,
        r,
        c,
        "POST",
        kind.submit_url(),
        json_body(&body_for(node_info(kind, &p["input"], &images, &audios)?))?,
        false,
    )
    .map_err(|e| {
        if matches!(
            e.code,
            "PERMISSION_DENIED" | "INVALID_INPUT" | "BUDGET_EXCEEDED"
        ) {
            e
        } else {
            fail(
                "ACCEPTANCE_UNKNOWN",
                "提交回执中断，可能已受理；禁止自动重提",
            )
        }
    })?;
    let v = unwrap_response(&response).map_err(|e| {
        if e.vendor.is_none()
            && response["status"]
                .as_u64()
                .is_some_and(|n| (200..300).contains(&n))
        {
            fail("ACCEPTANCE_UNKNOWN", "提交回执无法确认；禁止自动重提")
        } else {
            e
        }
    })?;
    if v["status"] == "FAILED" {
        return Err(fail(
            "UPSTREAM_ERROR",
            v["errorMessage"].as_str().unwrap_or("RunningHub拒绝请求"),
        ));
    }
    let id =
        task_id(&v).map_err(|_| fail("ACCEPTANCE_UNKNOWN", "缺少真实顶层taskId；禁止自动重提"))?;
    let a = json!({"kind":"accepted","vendorTaskId":id,"appliedParams":null,"resumeToken":{"modelId":kind.model_id(),"appId":kind.app_id(),"taskId":id},"usage":usage(&v)});
    persist(h, r, &a).map_err(|_| {
        let mut e = fail(
            "ACCEPTANCE_UNKNOWN",
            "已受理但快照保存失败；保留任务号，不重提",
        );
        e.request = Some(id);
        e
    })?;
    Ok(a)
}
fn usage(v: &Value) -> Value {
    let mut m = Vec::new();
    for (key, metric, unit) in [
        ("consumeCoins", "credits", "RH_coin"),
        ("consumeMoney", "money", "vendor_currency"),
        (
            "thirdPartyConsumeMoney",
            "third_party_money",
            "vendor_currency",
        ),
        ("taskCostTime", "task_time", "vendor_reported_time_unit"),
    ] {
        let raw = &v["usage"][key];
        let amount = if let Some(s) = raw.as_str() {
            s.to_string()
        } else if raw.is_number() {
            raw.to_string()
        } else {
            continue;
        };
        let mut parts = amount.split('.');
        let w = parts.next().unwrap_or("");
        let f = parts.next();
        if w.is_empty()
            || !w.bytes().all(|b| b.is_ascii_digit())
            || (w.len() > 1 && w.starts_with('0'))
            || parts.next().is_some()
            || f.is_some_and(|x| x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        m.push(json!({"metric":format!("{PLUGIN}/{metric}"),"unit":format!("{PLUGIN}/{unit}"),"amount":amount,"source":"reported"}));
    }
    if m.is_empty() {
        Value::Null
    } else {
        json!({"measurements":m})
    }
}

fn query(h: &mut dyn Host, r: &Invocation, p: &Value) -> Result<Value> {
    let kind = identity(p, false)?;
    let c = configuration(p)?;
    let id = text(p, "vendorTaskId")?;
    if id.is_empty()
        || p["resumeToken"]["appId"] != kind.app_id()
        || p["resumeToken"]["modelId"] != kind.model_id()
        || p["resumeToken"]["taskId"] != id
    {
        return Err(invalid("续查身份、App或任务号不匹配"));
    }
    let v = unwrap_response(&http(
        h,
        r,
        c,
        "POST",
        "https://www.runninghub.cn/openapi/v2/query".into(),
        json_body(&json!({"taskId":id}))?,
        false,
    )?)?;
    if task_id(&v)? != id {
        return Err(malformed());
    }
    let raw = text(&v, "status")?;
    let state = match raw {
        "CREATE" | "QUEUED" => "queued",
        "RUNNING" => "running",
        "SUCCESS" => "succeeded",
        "FAILED" => "failed",
        _ => return Err(malformed()),
    };
    let mut outputs = Vec::new();
    if state == "succeeded" {
        let items = v["results"]
            .as_array()
            .filter(|a| a.len() == 1)
            .ok_or_else(malformed)?;
        let item = &items[0];
        let result = (|| {
            let url = item
                .get("url")
                .or_else(|| item.get("fileUrl"))
                .and_then(Value::as_str)
                .ok_or_else(malformed)?;
            if !url.starts_with("https://") {
                return Err(fail("UPSTREAM_ERROR", "原片地址不是HTTPS"));
            }
            let downloaded = http(h, r, c, "GET", url.into(), json!({"kind":"none"}), true)?;
            if downloaded["status"] != 200 {
                return Err(fail("UPSTREAM_ERROR", "原片下载失败，保留原任务号续查"));
            }
            let handle = text(&downloaded["payload"], "handle")?;
            let m = port(h, "host-media.info", json!({"handle":handle}))?;
            let positive_decimal = |v: &Value| {
                v.as_str().is_some_and(|s| {
                    s.parse::<u64>()
                        .ok()
                        .is_some_and(|n| n > 0 && n.to_string() == s)
                })
            };
            let name = text(&m, "fileName")?;
            let hash = text(&m, "sha256")?;
            let content_name = name
                .strip_prefix(hash)
                .and_then(|s| s.strip_prefix('.'))
                .is_some_and(|ext| {
                    !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_alphanumeric())
                });
            if m["kind"] != "video"
                || !text(&m, "mime")?
                    .strip_prefix("video/")
                    .is_some_and(|subtype| !subtype.is_empty())
                || !positive_decimal(&m["byteLength"])
                || !positive_decimal(&m["durationMs"])
                || !m["width"].as_u64().is_some_and(|n| n > 0)
                || !m["height"].as_u64().is_some_and(|n| n > 0)
                || hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || text(&m, "id")?.is_empty()
                || !content_name
                || name.is_empty()
                || matches!(name, "." | "..")
                || name.contains(['/', '\\', ':'])
                || name.chars().any(char::is_control)
            {
                return Err(fail("UPSTREAM_ERROR", "原片未通过受管媒体探测"));
            }
            Ok(
                json!({"kind":"media","role":"video","media":{"id":m["id"],"kind":"video","handle":handle,"mime":m["mime"],"fileName":m["fileName"],"byteLength":m["byteLength"]}}),
            )
        })();
        outputs.push(result.map_err(|mut e| {
            e.message = format!(
                "云端任务{id}已成功，原片本地接收失败，保留原任务续查：{}",
                e.message
            );
            e.retryable = true;
            e
        })?);
    }
    Ok(
        json!({"status":state,"outputs":outputs,"data":{"appId":kind.app_id(),"modelId":kind.model_id(),"remoteTaskId":id,"downloadedCount":outputs.len()},"usage":usage(&v),"progress":if matches!(state,"queued"|"running"){json!({"fraction":null,"message":raw})}else{Value::Null},"terminalFailure":if state=="failed"{json!({"category":"vendorTaskFailed","message":v["errorMessage"].as_str().unwrap_or("RunningHub任务失败"),"vendorCode":v["errorCode"],"details":{"taskId":id,"remoteStatus":raw}})}else{Value::Null},"resumeToken":p["resumeToken"]}),
    )
}
pub struct RunningHubH3;
impl Adapter for RunningHubH3 {
    fn invoke(r: Invocation, h: &mut dyn Host) -> Response {
        let result = (|| {
            let p: Value =
                serde_json::from_str(&r.r#payload_json).map_err(|_| invalid("请求不是JSON"))?;
            match serde_json::to_value(&r.r#operation)
                .unwrap()
                .as_str()
                .unwrap()
            {
                "capabilities" => Ok(capabilities()),
                "probe" => match p["kind"].as_str() {
                    Some("package_validation") => Ok(
                        json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]}),
                    ),
                    Some("connectivity") => Err(fail(
                        "UNSUPPORTED_FEATURE",
                        "不以计费生成冒充连通探测，请由用户手动测试",
                    )),
                    _ => Err(invalid("未知probe类型")),
                },
                "submit" => submit(h, &r, &p),
                "query" => query(h, &r, &p),
                "cancel" => {
                    identity(&p, false)?;
                    Ok(json!({"outcome":"unsupported"}))
                }
                "upload" => Err(fail(
                    "UNSUPPORTED_FEATURE",
                    "本包随submit上传原件，不提供独立uploadRef",
                )),
                _ => Err(fail("UNSUPPORTED_FEATURE", "不支持的操作")),
            }
        })();
        match result{Ok(v)=>guest::completed_response(&r,&v).expect("public result"),Err(e)=>serde_json::from_value(json!({"protocolVersion":2,"operation":r.r#operation,"error":{"code":e.code,"message":e.message,"retryable":e.retryable,"vendorCode":e.vendor,"vendorRequestId":e.request}})).expect("public error")}
    }
}
bgx_adapter_v2_sdk::export_adapter!(RunningHubH3);

#[cfg(test)]
mod tests;
