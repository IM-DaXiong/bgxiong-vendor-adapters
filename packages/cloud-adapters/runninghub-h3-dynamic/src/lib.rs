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
pub const PLUGIN: &str = "local.thirdparty.runninghub-h3-dynamic";
pub const OP: &str = "local.thirdparty.runninghub-h3-dynamic/reference-video";
pub const MODEL: &str = "runninghub-h3-r2v-turbo-dynamic";
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
    let value = host::call(h, m, &a).map_err(|e| {
        let v = serde_json::to_value(&e).expect("public error");
        let code = match v["code"].as_str() {
            Some("PERMISSION_DENIED") => "PERMISSION_DENIED",
            Some("TIMEOUT") => "TIMEOUT",
            Some("BUDGET_EXCEEDED") => "BUDGET_EXCEEDED",
            _ => "RESOURCE_UNAVAILABLE",
        };
        fail(code, format!("受管端口{m}失败：{}", host::diagnostic(&e)))
    })?;
    // Decode every consumed managed result against the frozen public SDK DTO.
    // A mock missing required facts must fail here just like a real host reply.
    macro_rules! checked {
        ($dto:ty) => {{
            let typed: $dto = serde_json::from_value(value)
                .map_err(|_| fail("INVALID_INPUT", format!("受管端口{m}返回违反公开SDK合同")))?;
            Ok(serde_json::to_value(typed).expect("public managed result"))
        }};
    }
    match m {
        "host-media.info" => checked!(bgx_adapter_v2_sdk::HostPortCallV2MediaStat),
        "host-http.execute" => checked!(bgx_adapter_v2_sdk::HostPortCallV2HttpResponse),
        "host-checkpoint.read" => checked!(Option<bgx_adapter_v2_sdk::HostPortCallV2CheckpointRead>),
        "host-checkpoint.write" => checked!(bgx_adapter_v2_sdk::HostPortCallV2CheckpointWrite),
        _ => Err(invalid("本包未声明的受管端口")),
    }
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

const WORKFLOW: &str = "2100758868111486978";
const GRAPH: &str = include_str!("workflow-api.json");
pub fn capabilities() -> Value {
    let i = schema("input", include_str!("../schemas/input.json"));
    let o = schema("output", include_str!("../schemas/output.json"));
    let c = schema("configuration", include_str!("../schemas/config.json"));
    let r = schema("resume", include_str!("../schemas/resume.json"));
    let controls: Vec<Value> = [
        ("inputMode", "text", "输入模式（固定 multi_image_to_video）"),
        ("prompt", "multiline", "提示词原文"),
        ("referenceImages", "media", "有序参考图（1～9张）"),
        ("referenceAudios", "media", "有序参考音（0～3段）"),
        ("aspectRatio", "select", "画幅"),
        ("resolutionValue/value", "number", "清晰度（0.1～16 MP）"),
        ("durationSeconds", "number", "目标秒数（1～30）"),
        ("fps", "number", "帧率（固定24）"),
        ("seed", "number", "随机种子"),
        ("advanced/audioUsage", "select", "参考音用途（音色条件）"),
    ]
    .iter()
    .map(|(f, w, l)| {
        json!({
            "fieldPointer": format!("/{f}"),
            "widget": w,
            "label": l,
            "description": "工作台实际值原样提交；画幅和MP写入官方尺寸节点；平台拒绝显式回传，不静默改参数",
            "group": "视频输入"
        })
    })
    .collect();
    json!({"schemaVersion":2,"pluginId":PLUGIN,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[{
        "schemaVersion":2,"localId":"reference-video","capabilityKey":"video.generate","transportSlot":"video","modelIds":[MODEL],
        "configurationSchema":{"id":"configuration","digest":c["digest"]},"schemas":[i.clone(),o.clone(),c,r.clone()],
        "operations":[{"id":OP,"version":1,"label":"动态图音参考","inputSchema":{"id":"input","digest":i["digest"]},"outputSchema":{"id":"output","digest":o["digest"]},"resumeSchema":{"id":"resume","digest":r["digest"]},"uiSchema":{"controls":controls},
        "mediaSlots":[
            {"role":"reference_image","inputPointer":"/referenceImages","mediaKind":"image","minCount":1,"maxCount":9},
            {"role":"reference_audio","inputPointer":"/referenceAudios","mediaKind":"audio","minCount":0,"maxCount":3}
        ],
        "outputSlots":[{"role":"video","kind":"video","minCount":1,"maxCount":1}],"execution":{"mode":"asynchronous","streaming":false,"cancellation":"unsupported","resumable":true,"remoteIdempotency":false}}]}]})
}

fn identity(p: &Value, submit: bool) -> Result<()> {
    if p["schemaVersion"] != 2
        || p["modelId"] != MODEL
        || p["capabilityKey"] != "video.generate"
        || p["operationId"] != OP
        || p["operationVersion"] != 1
    {
        return Err(invalid("模型、能力或操作身份不匹配"));
    }
    if submit
        && p["inputSchemaDigest"]
            != capabilities()["capabilityContracts"][0]["operations"][0]["inputSchema"]["digest"]
    {
        return Err(invalid("输入schema摘要不匹配"));
    }
    Ok(())
}
fn configuration(p: &Value) -> Result<&Value> {
    let c = &p["configuration"];
    if c.as_object().is_none_or(|o| o.len() != 3)
        || c["endpointBaseUrl"] != "https://www.runninghub.cn"
        || text(c, "credentialRef")?.is_empty()
        || !["default", "plus", "ultra"].contains(&text(c, "instanceType")?)
    {
        return Err(invalid("须配置RunningHub中国站、受管凭据引用和实例类型"));
    }
    Ok(c)
}
fn refs<'a>(i: &'a Value, f: &str, min: usize, max: usize) -> Result<&'a Vec<Value>> {
    let a = i[f].as_array().ok_or_else(|| invalid("参考媒体须为数组"))?;
    if a.len() < min
        || a.len() > max
        || a.iter().any(|v| {
            v.as_object().is_none_or(|o| o.len() != 1)
                || !v["mediaId"].as_str().is_some_and(|s| !s.is_empty())
        })
    {
        return Err(invalid("参考图1～9张、参考音0～3段，每项仅含mediaId"));
    }
    Ok(a)
}
fn number(i: &Value, f: &str) -> Result<u64> {
    i[f].as_u64().ok_or_else(|| invalid("缺少合法整数参数"))
}
fn validate_input(i: &Value) -> Result<()> {
    let keys = [
        "inputMode",
        "prompt",
        "referenceImages",
        "referenceAudios",
        "aspectRatio",
        "resolutionValue",
        "durationSeconds",
        "fps",
        "seed",
        "advanced",
    ];
    let o = i.as_object().ok_or_else(|| invalid("input须为对象"))?;
    if o.len() != keys.len() || o.keys().any(|k| !keys.contains(&k.as_str())) {
        return Err(invalid("输入字段缺失或未声明"));
    }
    if i["inputMode"] != "multi_image_to_video" {
        return Err(invalid("本操作 inputMode 固定为 multi_image_to_video"));
    }
    if text(i, "prompt")?.is_empty() {
        return Err(invalid("提示词不能为空"));
    }
    // 画幅+MP是本操作唯一尺寸表示；不混入上一版像素宽高。
    if i["advanced"] != json!({"audioUsage":"voice_timbre"}) {
        return Err(invalid("本操作仅支持参考音用途 voice_timbre，不支持配乐拷贝"));
    }
    refs(i, "referenceImages", 1, 9)?;
    refs(i, "referenceAudios", 0, 3)?;
    aspect_label(i["aspectRatio"].as_str().ok_or_else(|| invalid("画幅须为字符串"))?)?;
    let rv = &i["resolutionValue"];
    if rv.as_object().is_none_or(|o| o.len() != 2) || rv["kind"] != "megapixels" {
        return Err(invalid("清晰度须为{kind:megapixels,value}，不能混入其它尺寸表示"));
    }
    let mp = rv["value"].as_f64().ok_or_else(|| invalid("MP须为数值"))?;
    if !(0.1..=16.0).contains(&mp) || (((mp * 10.0 + 0.5) as u64) as f64 - mp * 10.0).abs() > 1e-8 {
        return Err(invalid("官方尺寸节点允许0.1～16MP，步长0.1"));
    }
    if !(1..=30).contains(&number(i, "durationSeconds")?)
        || number(i, "fps")? != 24
        || number(i, "seed")? > 9007199254740991
    {
        return Err(invalid("时长1～30整数秒，H3原生24fps，种子不超过2^53-1"));
    }
    Ok(())
}
// Official ResolutionSelector enum; platform mapping stays in this independent package.
fn aspect_label(value: &str) -> Result<&'static str> {
    match value {
        "1:1" => Ok("1:1 (Square)"),
        "2:3" => Ok("2:3 (Portrait Photo)"),
        "3:2" => Ok("3:2 (Photo)"),
        "3:4" => Ok("3:4 (Portrait Standard)"),
        "4:3" => Ok("4:3 (Standard)"),
        "9:16" => Ok("9:16 (Portrait Widescreen)"),
        "16:9" => Ok("16:9 (Widescreen)"),
        "21:9" => Ok("21:9 (Ultrawide)"),
        _ => Err(invalid("画幅不在官方尺寸节点的8项选项内")),
    }
}
/// A fresh graph per submit: remove baked loaders AND reference links before inserting exactly N/M assets.
fn build_graph(i: &Value, images: &[String], audios: &[String]) -> Result<Value> {
    validate_input(i)?;
    if images.len() != refs(i, "referenceImages", 1, 9)?.len()
        || audios.len() != refs(i, "referenceAudios", 0, 3)?.len()
        || images.iter().chain(audios.iter()).any(|s| s.is_empty())
    {
        return Err(invalid("上传回执数量或路径无效"));
    }
    let mut g: Value = serde_json::from_str(GRAPH).expect("frozen published graph");
    g.as_object_mut().unwrap().retain(|_, n| {
        !["LoadImage", "LoadAudio"]
            .iter()
            .any(|s| n["class_type"] == *s)
    });
    let c = g["136"]["inputs"].as_object_mut().unwrap();
    c.retain(|k, _| !k.starts_with("ref_images.") && !k.starts_with("ref_audios."));
    g["115"]["inputs"]["aspect_ratio"] = json!(aspect_label(i["aspectRatio"].as_str().unwrap())?);
    g["115"]["inputs"]["megapixels"] = i["resolutionValue"]["value"].clone();
    for (kind, paths) in [("image", images), ("audio", audios)] {
        for (n, path) in paths.iter().enumerate() {
            let id = (if kind == "image" { 1000 + n } else { 2000 + n }).to_string();
            g[&id] = if kind == "image" {
                json!({"class_type":"LoadImage","inputs":{"image":path}})
            } else {
                json!({"class_type":"LoadAudio","inputs":{"audio":path}})
            };
            g["136"]["inputs"][format!("ref_{kind}s.ref_{kind}_{n}")] = json!([id, 0]);
        }
    }
    g["138"]["inputs"]["value"] = i["prompt"].clone();
    g["129"]["inputs"]["noise_seed"] = i["seed"].clone();
    g["132"]["inputs"]["value"] = i["durationSeconds"].clone();
    // Pass the allowed raw count (<=3600). The official H3 node aligns internally.
    // Pre-aligning 150s to 3609 would violate its input bound before execution.
    g["131"]["inputs"]["expression"] = json!("max(5, round(a * 24))");
    g["130"]["inputs"]["fps"] = i["fps"].clone();
    // A distinct output identity detects a cloud endpoint ignoring workflow overrides.
    let output = g
        .as_object_mut()
        .unwrap()
        .remove("92")
        .expect("source SaveVideo");
    g["9000"] = output;
    for node in g.as_object().unwrap().values() {
        for v in node["inputs"].as_object().unwrap().values() {
            if let Some(a) = v.as_array() {
                if a.len() != 2 || !a[0].as_str().is_some_and(|id| g.get(id).is_some()) {
                    return Err(invalid("组装图存在断开节点引用"));
                }
            }
        }
    }
    Ok(g)
}
fn selected<'a>(p: &'a Value) -> Result<Vec<(&'static str, &'a Value)>> {
    identity(p, true)?;
    validate_input(&p["input"])?;
    configuration(p)?;
    if !p["uploads"].as_array().is_some_and(|a| a.is_empty()) {
        return Err(invalid("本包随submit上传原件，不接受外部uploadRef"));
    }
    let media = p["media"]
        .as_array()
        .ok_or_else(|| invalid("缺少受管媒体列表"))?;
    let mut result = Vec::new();
    for (f, k, min, max) in [
        ("referenceImages", "image", 1, 9),
        ("referenceAudios", "audio", 0, 3),
    ] {
        for r in refs(&p["input"], f, min, max)? {
            let found: Vec<_> = media.iter().filter(|m| m["id"] == r["mediaId"]).collect();
            if found.len() != 1 || found[0]["kind"] != k {
                return Err(invalid("媒体ID或分类不匹配"));
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
    let response = port(
        h,
        "host-http.execute",
        json!({"plan":{"method":method,"url":url,"headers":if body["kind"]=="bytes"{json!([{"name":"Content-Type","value":"application/json"}])}else{json!([])},"generatedHeaders":if download{json!([])}else{json!([{"name":"Authorization","kind":"bearer","targetField":"Authorization"}])},"body":body,"sink":if download{json!({"kind":"media"})}else{json!({"kind":"buffer","maxBytes":1048576})},"timeoutMs":r.r#budget_ms},"credential":if download{Value::Null}else{c["credentialRef"].clone()}}),
    )?;
    let expected = if download { "media" } else { "buffered" };
    if response["payload"]["kind"] != expected {
        return Err(fail("INVALID_INPUT", "受管HTTP回执类型与声明sink不一致"));
    }
    Ok(response)
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
    // Leave the overall nine-minute budget for the whole group; a stalled single
    // transfer returns through the adapter so it can record explicit non-submission.
    let mut upload_request = r.clone();
    upload_request.r#budget_ms = r.r#budget_ms.min(180000);
    let v = unwrap_response(&http(
        h,
        &upload_request,
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
    let receipt = port(
        h,
        "host-checkpoint.write",
        json!({"requestId":r.r#request_id,"expectedRevision":rev,"valueJson":v.to_string()}),
    )?;
    if receipt["revision"] != next.to_string() {
        return Err(invalid("快照写入回执版本与请求不一致"));
    }
    Ok(())
}
/// Only the independent adapter knows this failure happened before the billable request.
fn reject_before_submission(h: &mut dyn Host, r: &Invocation, reason: &str, evidence: Value) -> Result<()> {
    let read = port(h, "host-checkpoint.read", json!({"requestId":r.r#request_id}))?;
    let checkpoint: Value = serde_json::from_str(text(&read, "valueJson")?).map_err(|_| malformed())?;
    let candidate = bgx_adapter_v2_sdk::submission_rejection::checkpoint_candidate(
        &checkpoint, "before_submission", reason, &evidence,
    ).map_err(invalid)?;
    let receipt = port(h, "host-checkpoint.write", json!({"requestId":r.r#request_id,
        "expectedRevision":read["revision"],"valueJson":candidate.to_string()}))?;
    if receipt["revision"] != candidate["revision"] {
        return Err(invalid("未提交事实的保存回执不一致"));
    }
    Ok(())
}
fn submit(h: &mut dyn Host, r: &Invocation, p: &Value) -> Result<Value> {
    let chosen = selected(p)?;
    let c = configuration(p)?;
    let ni = refs(&p["input"], "referenceImages", 1, 9)?.len();
    let na = refs(&p["input"], "referenceAudios", 0, 3)?.len();
    let placeholders = |n| (0..n).map(|_| String::from("pending")).collect::<Vec<_>>();
    let preview = build_graph(&p["input"], &placeholders(ni), &placeholders(na))?;
    let body_for = |g: &Value| json!({"workflow":g.to_string(),"nodeInfoList":[],"addMetadata":true,"instanceType":c["instanceType"],"usePersonalQueue":false});
    json_body(&body_for(&preview))?;
    let mut images = Vec::new();
    let mut audios = Vec::new();
    for (kind, m) in chosen {
        let name = match upload(h, r, c, m) {
            Ok(name) => name,
            Err(mut error) => {
                let reason = match error.code {
                    "TIMEOUT" => "素材上传超时，请检查网络或稍后重试",
                    "RESOURCE_UNAVAILABLE" => "素材上传连接中断，请检查网络或稍后重试",
                    "UNAUTHORIZED" => "素材上传被拒绝，请检查适配器账号和授权",
                    "RATE_LIMITED" => "素材上传过于频繁，请稍后重试",
                    _ => "素材上传失败",
                };
                reject_before_submission(h, r, reason, json!({"phase":"media_upload",
                    "mediaId":m["id"],"errorCode":error.code}))
                    .map_err(|_| fail("ACCEPTANCE_UNKNOWN", "素材上传失败，未能保存提交状态；请核对第三方任务记录后再操作"))?;
                error.message = if matches!(error.code, "TIMEOUT" | "RESOURCE_UNAVAILABLE" | "UNAUTHORIZED" | "RATE_LIMITED") {
                    format!("{reason}。视频生成尚未提交。")
                } else {
                    format!("{reason}：{}。视频生成尚未提交。", error.message)
                };
                return Err(error);
            }
        };
        if kind == "image" {
            images.push(name)
        } else {
            audios.push(name)
        }
    }
    let graph = build_graph(&p["input"], &images, &audios)?;
    let response = http(
        h,
        r,
        c,
        "POST",
        format!("https://www.runninghub.cn/openapi/v2/run/workflow/{WORKFLOW}"),
        json_body(&body_for(&graph))?,
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
                "提交回执中断，可能已受理；核对云端任务，禁止自动重提",
            )
        }
    })?;
    let v = unwrap_response(&response).map_err(|e| {
        if e.vendor.is_none()
            && response["status"]
                .as_u64()
                .is_some_and(|n| (200..300).contains(&n))
        {
            fail("ACCEPTANCE_UNKNOWN", "提交回执无法确认，禁止自动重提")
        } else {
            e
        }
    })?;
    if v["status"] == "FAILED" {
        let msg = v["errorMessage"]
            .as_str()
            .unwrap_or("RunningHub拒绝工作流");
        // 平台参数/尺寸拒绝沿错误通道显式回传；保留用户草稿宽高，禁止静默改参重提。
        return Err(fail(
            "UPSTREAM_ERROR",
            format!("{msg}；未修改用户画幅和MP或其它参数，禁止自动重提"),
        ));
    }
    let id = text(&v, "taskId")
        .map_err(|_| fail("ACCEPTANCE_UNKNOWN", "未收到顶层taskId，禁止自动重提"))?;
    if id.is_empty() {
        return Err(fail("ACCEPTANCE_UNKNOWN", "任务号为空，禁止自动重提"));
    }
    let a = json!({"kind":"accepted","vendorTaskId":id,"appliedParams":null,"resumeToken":{"workflowId":WORKFLOW,"taskId":id},"usage":usage(&v)});
    persist(h, r, &a).map_err(|_| {
        let mut e = fail(
            "ACCEPTANCE_UNKNOWN",
            "已受理但快照落盘失败；保留任务号，禁止重提",
        );
        e.request = Some(id.into());
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
fn validate_video_artifact(stat: &Value) -> Result<()> {
    let positive_decimal = |value: &Value| value.as_str()
        .and_then(|s| s.parse::<u64>().ok().filter(|n| *n > 0 && n.to_string() == s));
    // The managed broker owns byte detection and video probing. Consume its
    // actual video facts without a second package-specific container whitelist.
    let mime = stat["mime"].as_str().ok_or_else(malformed)?;
    if !mime.starts_with("video/") {
        return Err(fail("UPSTREAM_ERROR", "受管原片探测未返回视频MIME"));
    }
    let digest = stat["sha256"].as_str().ok_or_else(malformed)?;
    let name = stat["fileName"].as_str().ok_or_else(malformed)?;
    let extension = name.strip_prefix(digest).and_then(|s| s.strip_prefix('.'));
    let safe_content_name = extension.is_some_and(|ext| !ext.is_empty()
        && ext.bytes().all(|b| b.is_ascii_alphanumeric()));
    if stat["kind"] != "video"
        || positive_decimal(&stat["byteLength"]).is_none()
        || positive_decimal(&stat["durationMs"]).is_none()
        || stat["width"].as_u64().is_none_or(|n| n == 0)
        || stat["height"].as_u64().is_none_or(|n| n == 0)
        || digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !safe_content_name
    {
        return Err(fail("UPSTREAM_ERROR", "原片受管探测事实不完整或容器、文件名、大小、时长、尺寸、摘要不一致"));
    }
    Ok(())
}
fn query(h: &mut dyn Host, r: &Invocation, p: &Value) -> Result<Value> {
    identity(p, false)?;
    let c = configuration(p)?;
    let id = text(p, "vendorTaskId")?;
    if id.is_empty()
        || p["resumeToken"]["workflowId"] != WORKFLOW
        || p["resumeToken"]["taskId"] != id
    {
        return Err(invalid("续查身份或任务号不匹配"));
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
    if v["taskId"] != id {
        return Err(malformed());
    }
    let raw = text(&v, "status")?;
    let state = match raw {
        "QUEUED" => "queued",
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
        if item["nodeId"] != "9000" {
            return Err(fail(
                "UPSTREAM_ERROR",
                "云端输出不是本包整图的节点9000；可能忽略了workflow覆盖，未用旧图结果冒充成功",
            ));
        }
        let result = (|| {
            let url = text(item, "url")?;
            if !url.starts_with("https://") {
                return Err(fail("UPSTREAM_ERROR", "原片地址不是HTTPS"));
            }
            let download = http(h, r, c, "GET", url.into(), json!({"kind":"none"}), true)?;
            if download["status"] != 200 {
                return Err(fail("UPSTREAM_ERROR", "原片下载失败，保留原任务号续查"));
            }
            let handle = text(&download["payload"], "handle")?;
            let s = port(h, "host-media.info", json!({"handle":handle}))?;
            validate_video_artifact(&s)?;
            Ok(
                json!({"kind":"media","role":"video","media":{"id":s["id"],"kind":"video","handle":handle,"mime":s["mime"],"fileName":s["fileName"],"byteLength":s["byteLength"]}}),
            )
        })();
        match result {
            Ok(o) => outputs.push(o),
            Err(mut e) => {
                e.message = format!("云端任务{id}已成功，原片取回失败；保留原任务号续查，不重新生成：{}", e.message);
                e.retryable = true;
                return Err(e);
            }
        }
    }
    Ok(
        json!({"status":state,"outputs":outputs,"data":{"workflowId":WORKFLOW,"remoteTaskId":id,"downloadedCount":outputs.len()},"usage":usage(&v),"progress":if matches!(state,"queued"|"running"){json!({"fraction":null,"message":raw})}else{Value::Null},"terminalFailure":if state=="failed"{json!({"category":"vendorTaskFailed","message":v["errorMessage"].as_str().unwrap_or("RunningHub任务生成失败"),"vendorCode":v["errorCode"],"details":{"taskId":id,"remoteStatus":raw}})}else{Value::Null},"resumeToken":p["resumeToken"]}),
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
