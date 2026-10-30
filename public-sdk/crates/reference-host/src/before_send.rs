//! Awaitable, vendor-neutral outbound persistence. No model-specific text mapping.
use crate::{
    credential::CredentialVault,
    http::MediaAccess,
    policy::{error, Result},
};
use bgx_adapter_v2_transport::Control;
use bgx_adapter_v2_validator::hash_bytes;
use serde_json::{json, Value};
use std::{future::Future, pin::Pin};

pub type PersistFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
pub trait BeforeSendSink: Send + Sync {
    /// Resolve successfully only after durable persistence. Failure prevents HTTP.
    fn persist<'a>(&'a self, record: &'a Value) -> PersistFuture<'a>;
}

// Serde's Value sorts object keys. Walk the original JSON stream separately so
// the audit text follows the submitted field/array order, without key heuristics.
struct Texts<'a>(&'a mut Vec<String>);
impl<'de> serde::de::DeserializeSeed<'de> for Texts<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}
impl<'de> serde::de::Visitor<'de> for Texts<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON value")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<(), E> {
        self.0.push(value.into());
        Ok(())
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> std::result::Result<(), A::Error> {
        while seq.next_element_seed(Texts(self.0))?.is_some() {}
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<(), A::Error> {
        while map.next_key::<String>()?.is_some() {
            map.next_value_seed(Texts(self.0))?;
        }
        Ok(())
    }
}
pub(crate) fn redact_url(raw: &str, vault: &CredentialVault) -> String {
    let Ok(mut url) = url::Url::parse(raw) else {
        return vault.redact(raw);
    };
    if url.query().is_some() {
        let pairs: Vec<_> = url
            .query_pairs()
            .map(|(k, v)| (vault.redact(&k), vault.redact(&v)))
            .collect();
        url.query_pairs_mut().clear().extend_pairs(pairs);
    }
    vault.redact(url.as_str())
}
pub(crate) fn redact(value: &mut Value, vault: &CredentialVault) {
    match value {
        Value::String(s) => *s = vault.redact(s),
        Value::Array(values) => values.iter_mut().for_each(|v| redact(v, vault)),
        Value::Object(values) => values.values_mut().for_each(|v| redact(v, vault)),
        _ => {}
    }
}
async fn media_description(access: &MediaAccess, handle: &str) -> Result<Value> {
    let handle = handle.to_owned();
    access
        .run(move |broker, scope| {
            let stat = broker.info(scope, &handle)?;
            Ok(json!({"mediaId":stat.r#id,"sha256":stat.r#sha256,"byteLength":stat.r#byte_length}))
        })
        .await
}
/// Preserve JSON string values/plain UTF-8 exactly. Binary uses measured identity.
pub(crate) async fn body(
    plan: &Value,
    access: &MediaAccess,
    control: &Control,
) -> Result<(Value, Vec<String>)> {
    let body = &plan["body"];
    let mut text = vec![];
    let value = match body["kind"].as_str() {
        Some("none") => Value::Null,
        Some("bytes") => {
            let bytes: Vec<u8> = serde_json::from_value(body["bytes"].clone())
                .map_err(|_| error("INVALID_INPUT", "Invalid outbound byte body"))?;
            match std::str::from_utf8(&bytes) {
                Ok(s) => {
                    let value = serde_json::from_str::<Value>(s)
                        .unwrap_or_else(|_| Value::String(s.to_owned()));
                    if serde_json::from_str::<Value>(s).is_ok() {
                        use serde::de::DeserializeSeed;
                        Texts(&mut text)
                            .deserialize(&mut serde_json::Deserializer::from_str(s))
                            .map_err(|_| {
                                error("INVALID_INPUT", "Cannot project JSON audit text")
                            })?;
                    } else {
                        text.push(s.to_owned());
                    }
                    value
                }
                Err(_) => json!({"sha256":hash_bytes(&bytes),"byteLength":bytes.len().to_string()}),
            }
        }
        Some("media") => media_description(access, body["handle"].as_str().unwrap()).await?,
        Some("multipart") => {
            let mut parts = vec![];
            for part in body["parts"].as_array().unwrap() {
                if control.is_cancelled() {
                    return Err(error("CANCELLED", "Outbound persistence cancelled"));
                }
                if part["kind"] == "field" {
                    text.push(part["value"].as_str().unwrap().to_owned());
                    parts.push(part.clone());
                } else {
                    parts.push(json!({"name":part["name"],"kind":"media",
                        "media":media_description(access, part["handle"].as_str().unwrap()).await?}));
                }
            }
            json!({"kind":"multipart","parts":parts})
        }
        _ => return Err(error("INVALID_INPUT", "Unknown outbound body")),
    };
    Ok((value, text))
}
