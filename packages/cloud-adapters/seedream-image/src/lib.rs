#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{format, string::{String, ToString}, vec::Vec};
use base64::{engine::general_purpose::STANDARD, Engine};
use bgx_adapter_v2_sdk::{guest, host, Adapter, Host, Invocation, Response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
const PLUGIN: &str = "local.bgx.seedream-image";
const OP: &str = "local.bgx.seedream-image/generate";
const MODEL: &str = "doubao-seedream-5-0-pro-260628";
const CHUNK: usize = 16384;
const MAX_OBJECT: usize = 32 * 1024 * 1024;
const INPUT: &str = r#"{"type":"object","properties":{"prompt":{"type":"string","minLength":1},"size":{"type":"string","enum":["2K","2048x2048","2720x1530","1530x2720"]},"watermark":{"type":"boolean"},"responseFormat":{"type":"string","enum":["url","b64_json"]},"referenceImages":{"type":"array","maxItems":10,"items":{"type":"object","properties":{"mediaId":{"type":"string","minLength":1}},"required":["mediaId"],"additionalProperties":false}}},"required":["prompt","referenceImages"],"additionalProperties":false}"#;
const CONFIG: &str = r#"{"type":"object","properties":{"endpointBaseUrl":{"type":"string","minLength":1},"credentialRef":{"type":"string","minLength":1}},"required":["endpointBaseUrl","credentialRef"],"additionalProperties":false}"#;
const OUTPUT: &str = r#"{"type":"object","properties":{"model":{"type":"string"},"providerRequestId":{"type":["string","null"]},"imageCount":{"type":"integer","minimum":1},"reportedUsageJson":{"type":["string","null"]}},"required":["model","providerRequestId","imageCount","reportedUsageJson"],"additionalProperties":false}"#;
#[derive(Debug)]
struct Failure {code:&'static str,message:String,vendor:Option<String>,request:Option<String>}
fn fail(code:&'static str,message:impl Into<String>)->Failure{Failure{code,message:message.into(),vendor:None,request:None}}
fn invalid(message:&str)->Failure{fail("INVALID_INPUT",message)}
fn malformed()->Failure{fail("UPSTREAM_ERROR","Invalid image API response; no retry attempted")}
fn text<'a>(v:&'a Value,k:&str)->Result<&'a str,Failure>{v[k].as_str().ok_or_else(malformed)}
fn port(h:&mut dyn Host,m:&str,args:Value)->Result<Value,Failure>{host::call(h,m,&args).map_err(|e|{
    let denied=serde_json::to_value(&e).ok().is_some_and(|v|v["code"]=="PERMISSION_DENIED");
    fail(if denied{"PERMISSION_DENIED"}else{"UPSTREAM_ERROR"},format!("Managed port {m}: {}; no retry attempted",host::diagnostic(&e)))})}
fn schema(id:&str,s:&str)->Value{let d:String=Sha256::digest(s.as_bytes()).iter().map(|b|format!("{b:02x}")).collect();json!({"id":id,"digest":d,"schemaJson":s})}
pub fn capabilities()->Value{
    let i=schema("input",INPUT);let c=schema("config",CONFIG);let o=schema("output",OUTPUT);
    json!({"schemaVersion":2,"pluginId":PLUGIN,"pluginVersion":env!("CARGO_PKG_VERSION"),"capabilityContracts":[{
        "schemaVersion":2,"localId":"image","capabilityKey":"image.generate","transportSlot":"image","modelIds":[MODEL],
        "configurationSchema":{"id":"config","digest":c["digest"]},"schemas":[i,c,o],"operations":[{
        "id":OP,"version":1,"label":"Seedream 5.0 Pro 单次图片生成预览","inputSchema":{"id":"input","digest":i["digest"]},"outputSchema":{"id":"output","digest":o["digest"]},"resumeSchema":null,
        "uiSchema":{"controls":[{"fieldPointer":"/prompt","widget":"multiline","label":"提示词","description":"原样提交","group":"输入"},{"fieldPointer":"/referenceImages","widget":"media","label":"有序参考图","description":"0至10张原件","group":"输入"},{"fieldPointer":"/size","widget":"select","label":"输出规格","description":"省略为2K","group":"参数"},{"fieldPointer":"/watermark","widget":"toggle","label":"水印","description":"省略为false","group":"参数"},{"fieldPointer":"/responseFormat","widget":"select","label":"响应格式","description":"省略为url","group":"参数"}]},
        "mediaSlots":[{"role":"reference","inputPointer":"/referenceImages","mediaKind":"image","minCount":0,"maxCount":10}],
        "outputSlots":[{"role":"artifact","kind":"image","minCount":1,"maxCount":null},{"role":"provider-response","kind":"file","minCount":1,"maxCount":1}],
        "execution":{"mode":"synchronous","streaming":true,"cancellation":"unsupported","resumable":false,"remoteIdempotency":false}}]}]})
}
fn read(h:&mut dyn Host,handle:&str)->Result<Vec<u8>,Failure>{
    let s=port(h,"host-media.info",json!({"handle":handle}))?;let n=text(&s,"byteLength")?.parse::<usize>().map_err(|_|malformed())?;
    if n>MAX_OBJECT{return Err(fail("BUDGET_EXCEEDED","Object exceeds the declared 32 MiB preview limit"));}
    let mut bytes=Vec::with_capacity(n);while bytes.len()<n{
        let part:Vec<u8>=serde_json::from_value(port(h,"host-media.read",json!({"handle":handle,"offset":bytes.len().to_string(),"maxBytes":CHUNK.min(n-bytes.len())}))?).map_err(|_|malformed())?;
        if part.is_empty(){return Err(malformed());}bytes.extend_from_slice(&part);
    }Ok(bytes)
}
fn write(h:&mut dyn Host,kind:&str,mime:&str,bytes:&[u8])->Result<String,Failure>{
    let handle=port(h,"host-media.create",json!({"kind":kind,"mimeHint":mime}))?.as_str().ok_or_else(malformed)?.to_string();
    for p in bytes.chunks(CHUNK){port(h,"host-media.write",json!({"handle":handle,"bytes":p}))?;}
    port(h,"host-media.finish",json!({"handle":handle}))?;Ok(handle)
}
fn image_mime(b:&[u8])->Result<&'static str,Failure>{
    if b.len()>=8&&b[..8]==[137,80,78,71,13,10,26,10]{Ok("image/png")}
    else if b.len()>=3&&b[..3]==[255,216,255]{Ok("image/jpeg")}
    else if b.len()>=12&&&b[..4]==b"RIFF"&&&b[8..12]==b"WEBP"{Ok("image/webp")}
    else if b.len()>=6&&(&b[..6]==b"GIF87a"||&b[..6]==b"GIF89a"){Ok("image/gif")}
    else{Err(fail("UPSTREAM_ERROR","Unknown image signature; bytes were not relabeled or transcoded"))}
}
fn output(h:&mut dyn Host,handle:&str,role:&str)->Result<Value,Failure>{
    let s=port(h,"host-media.info",json!({"handle":handle}))?;
    if role=="artifact"&&(s["kind"]!="image"||s["byteLength"]=="0"){return Err(malformed());}
    Ok(json!({"kind":"media","role":role,"media":{"id":s["id"],"kind":s["kind"],"handle":handle,"mime":s["mime"],"fileName":s["fileName"],"byteLength":s["byteLength"]}}))
}
fn emit(h:&mut dyn Host,r:&Invocation,sequence:&mut u64,kind:&str,payload:Value)->Result<(),Failure>{
    port(h,"host-stream.emit-event",json!({"event":{"requestId":r.r#request_id,"sequence":*sequence+1,"type":kind,"payload":payload}}))?;
    *sequence+=1;Ok(())
}
fn emit_media(h:&mut dyn Host,r:&Invocation,sequence:&mut u64,output:&Value)->Result<(),Failure>{
    emit(h,r,sequence,"media_chunk",json!({"role":output["role"],"media":output["media"]}))
}
fn body(h:&mut dyn Host,p:&Value)->Result<Value,Failure>{
    if p["modelId"]!=MODEL||p["operationId"]!=OP{return Err(invalid("Unknown explicit model or operation"));}
    let input=p["input"].as_object().ok_or_else(||invalid("input must be an object"))?;
    if input.keys().any(|k|!["prompt","size","watermark","responseFormat","referenceImages"].contains(&k.as_str())){return Err(invalid("Unsupported field; nothing was discarded"));}
    let prompt=text(&p["input"],"prompt")?;if prompt.is_empty(){return Err(invalid("prompt is empty"));}
    let mut b=json!({"model":p["modelId"],"prompt":prompt,"stream":false,"watermark":input.get("watermark").cloned().unwrap_or(json!(false)),"size":input.get("size").cloned().unwrap_or(json!("2K")),"response_format":input.get("responseFormat").cloned().unwrap_or(json!("url"))});
    let refs=p["input"]["referenceImages"].as_array().filter(|a|a.len()<=10).ok_or_else(||invalid("referenceImages requires 0..10 items"))?;
    let media=p["media"].as_array().ok_or_else(||invalid("media requires an array"))?;
    if media.len()!=refs.len(){return Err(invalid("Every media lease needs exactly one ordered input reference"));}
    let mut images=Vec::new();let mut used=Vec::new();
    for r in refs{
        let id=text(r,"mediaId")?;let matches:Vec<&Value>=media.iter().filter(|m|m["id"]==id).collect();
        if matches.len()!=1||used.contains(&id){return Err(invalid("Reference IDs must resolve exactly once"));}used.push(id);
        let m=matches[0];if m["kind"]!="image"{return Err(invalid("Reference must be an original image lease"));}
        let bytes=read(h,text(m,"handle")?)?;let mime=image_mime(&bytes)?;if m["mime"]!=mime{return Err(invalid("Reference MIME differs from actual bytes"));}
        images.push(format!("data:{mime};base64,{}",STANDARD.encode(bytes)));
    }
    if images.len()==1{b["image"]=json!(images[0]);}else if !images.is_empty(){b["image"]=json!(images);}Ok(b)
}
fn http(h:&mut dyn Host,method:&str,url:&str,body:Value,credential:Option<&str>,budget:u64)->Result<Value,Failure>{
    port(h,"host-http.execute",json!({"plan":{"method":method,"url":url,"headers":if method=="POST"{json!([{"name":"Content-Type","value":"application/json"}])}else{json!([])},"generatedHeaders":if credential.is_some(){json!([{"name":"Authorization","kind":"bearer","targetField":"Authorization"}])}else{json!([])},"body":body,"sink":{"kind":"media"},"timeoutMs":budget},"credential":credential}))
}
fn usage(v:&Value)->Result<Value,Failure>{
    let Some(u)=v.get("usage").filter(|u|!u.is_null())else{return Ok(Value::Null)};if !u.is_object(){return Err(malformed());}
    let mut m=Vec::new();for(k,metric,unit)in[("input_tokens","input_tokens","tokens"),("output_tokens","output_tokens","tokens"),("total_tokens","total_tokens","tokens"),("image_count","local.bgx.seedream-image/image_count","local.bgx.seedream-image/images"),("generated_images","local.bgx.seedream-image/generated_images","local.bgx.seedream-image/images")]{
        if let Some(n)=u.get(k){let n=n.as_u64().ok_or_else(malformed)?;m.push(json!({"metric":metric,"unit":unit,"amount":n.to_string(),"source":"reported"}));}}
    if m.is_empty(){Ok(Value::Null)}else{Ok(json!({"measurements":m}))}
}
fn submit(h:&mut dyn Host,r:&Invocation,p:&Value)->Result<Value,Failure>{
    let b=body(h,p)?;let bytes=serde_json::to_vec(&b).map_err(|_|invalid("Invalid request JSON"))?;
    let handle=write(h,"file","application/octet-stream",&bytes)?;
    let url=format!("{}/images/generations",text(&p["configuration"],"endpointBaseUrl")?.trim_end_matches('/'));
    let w=http(h,"POST",&url,json!({"kind":"media","handle":handle}),Some(text(&p["configuration"],"credentialRef")?),r.r#budget_ms).map_err(|mut e|{if e.code=="UPSTREAM_ERROR"{e.code="ACCEPTANCE_UNKNOWN";e.message=format!("Submit acceptance is unknown: {}",e.message);}e})?;
    let raw=text(&w["payload"],"handle")?.to_string();
    let mut sequence=0;
    // Preserve the complete API response before interpreting optional usage or result data.
    let raw_output=output(h,&raw,"provider-response")?;
    emit_media(h,r,&mut sequence,&raw_output)?;
    let bytes=read(h,&raw)?;
    let parsed=serde_json::from_slice::<Value>(&bytes);
    let status=w["status"].as_u64().ok_or_else(malformed)?;
    if !(200..300).contains(&status){let v=parsed.unwrap_or(Value::Null);let mut e=fail(match status{401|403=>"UNAUTHORIZED",429=>"RATE_LIMITED",_=>"UPSTREAM_ERROR"},format!("Image HTTP {status}; no retry attempted"));e.vendor=v["error"]["code"].as_str().map(str::to_string);e.request=v["request_id"].as_str().map(str::to_string);return Err(e);}
    let v=parsed.map_err(|_|malformed())?;
    let result=(||{
        let mut failure=None;
        if let Some(error)=v.get("error") {
            let mut e=fail("UPSTREAM_ERROR","Image API reported a business failure");
            e.vendor=error["code"].as_str().map(str::to_string);
            retain_failure(&mut failure,e);
        }
        let reported_usage=match usage(&v) {
            Ok(value)=>value,
            Err(error)=>{retain_failure(&mut failure,error);Value::Null}
        };
        if !reported_usage.is_null(){emit(h,r,&mut sequence,"usage",reported_usage.clone())?;}
        let data=match v["data"].as_array(){Some(data)=>data,None=>return Err(failure.unwrap_or_else(malformed))};
        let mut outputs=Vec::new();
        for item in data {
            let before=outputs.len();
            if let Some(error)=item.get("error") {
                let mut e=fail("UPSTREAM_ERROR","An image result item reported failure; existing media events remain available");
                e.vendor=error["code"].as_str().map(str::to_string);
                retain_failure(&mut failure,e);
            }
            let mut urls=Vec::new();
            if let Some(url)=item["url"].as_str().or_else(||item["image_url"].as_str()) {
                if !url.trim().is_empty(){urls.push(url.trim());}
            } else if let Some(values)=item["urls"].as_array() {
                for value in values {
                    match value.as_str() {
                        Some(url) if !url.trim().is_empty()=>urls.push(url.trim()),
                        _=>retain_failure(&mut failure,malformed()),
                    }
                }
            }
            for url in urls {
                let downloaded=(||{
                    let w=http(h,"GET",url,json!({"kind":"none"}),None,r.r#budget_ms)?;
                    if w["status"].as_u64()!=Some(200){return Err(fail("UPSTREAM_ERROR","An output download failed; no complete result claimed"));}
                    let handle=text(&w["payload"],"handle")?;
                    let head:Vec<u8>=serde_json::from_value(port(h,"host-media.read",json!({"handle":handle,"offset":"0","maxBytes":32}))?).map_err(|_|malformed())?;
                    let mime=image_mime(&head)?;
                    let info=port(h,"host-media.info",json!({"handle":handle}))?;
                    if info["mime"]!=mime{return Err(malformed());}
                    output(h,handle,"artifact")
                })();
                match downloaded {
                    Ok(out)=>{emit_media(h,r,&mut sequence,&out)?;outputs.push(out);}
                    Err(error)=>retain_failure(&mut failure,error),
                }
            }
            if let Some(encoded)=item.get("b64_json").or_else(||item.get("b64")).or_else(||item.get("image_base64")) {
                let decoded=(||{
                    let value=encoded.as_str().ok_or_else(malformed)?.trim();
                    let bytes=STANDARD.decode(value).map_err(|_|malformed())?;
                    if bytes.len()>MAX_OBJECT{return Err(fail("BUDGET_EXCEEDED","Image exceeds 32 MiB preview limit"));}
                    let mime=image_mime(&bytes)?;
                    let handle=write(h,"image",mime,&bytes)?;
                    output(h,&handle,"artifact")
                })();
                match decoded {
                    Ok(out)=>{emit_media(h,r,&mut sequence,&out)?;outputs.push(out);}
                    Err(error)=>retain_failure(&mut failure,error),
                }
            }
            if before==outputs.len() && item.get("error").is_none() {
                retain_failure(&mut failure,fail("UPSTREAM_ERROR","An image result item contains no supported output; nothing was silently discarded"));
            }
        }
        // Independent results have been collected; the first actual error remains the terminal result.
        if let Some(error)=failure{return Err(error);}
        let count=outputs.len();if count==0{return Err(malformed());}
        outputs.push(raw_output);
        let mut applied=serde_json::Map::new();
        for (input,wire) in [("size","size"),("watermark","watermark"),("responseFormat","response_format")] {
            if p["input"].get(input).is_some(){applied.insert(input.to_string(),b[wire].clone());}
        }
        Ok(json!({"kind":"completed","vendorTaskId":null,"appliedParams":applied,"outputs":outputs,"data":{"model":MODEL,"providerRequestId":v.get("request_id").cloned().unwrap_or(Value::Null),"imageCount":count,"reportedUsageJson":v.get("usage").filter(|u|!u.is_null()).map(|u|Value::String(u.to_string())).unwrap_or(Value::Null)},"usage":reported_usage}))
    })();
    result.map_err(|mut e:Failure|{if e.request.is_none(){e.request=v["request_id"].as_str().map(str::to_string);}e})
}
fn retain_failure(first:&mut Option<Failure>,error:Failure){
    if first.is_none(){*first=Some(error);}
}

pub struct Seedream;
impl Adapter for Seedream{fn invoke(request:Invocation,h:&mut dyn Host)->Response{
    let result=(||{let p:Value=serde_json::from_str(&request.r#payload_json).map_err(|_|invalid("Payload is not JSON"))?;let op=serde_json::to_value(&request.r#operation).expect("operation");match op.as_str(){
        Some("capabilities")=>Ok(capabilities()),Some("probe")if p["kind"]=="package_validation"=>Ok(json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]})),Some("submit")=>submit(h,&request,&p),_=>Err(fail("UNSUPPORTED_FEATURE","This synchronous preview has no upload/query/cancel lifecycle"))}})();
    match result{Ok(v)=>guest::completed_response(&request,&v).expect("response"),Err(e)=>serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,"error":{"code":e.code,"message":e.message,"retryable":false,"vendorCode":e.vendor,"vendorRequestId":e.request}})).expect("error")}
}}
bgx_adapter_v2_sdk::export_adapter!(Seedream);
