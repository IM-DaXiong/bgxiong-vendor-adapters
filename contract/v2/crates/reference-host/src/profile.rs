//! Declarative author runtime. No durable state, ambient IO, scripts or retries.
//! Kernel preparation, canonical response validation and cleanup stay shared.
use crate::policy;
use bgx_adapter_v2_ports::{
    resource_policy,
    wire::{
        host::HostError, CapabilityContractV2, HostPortCallV2, Invocation, ProfileV2, Response,
    },
};
use bgx_adapter_v2_transport::{Control, ManagedHost};
use bgx_adapter_v2_validator::{
    capabilities::{Binding, CapabilityRegistry},
    hash_bytes,
    package::VerifiedPackage,
    ContractError, SchemaRegistry,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::OnceLock,
};

type Result<T> = std::result::Result<T, ContractError>;
type Key = (String, String, u64);
const PUBLIC_PORTS: &str = "https://bgxiong.local/vendor-adapter/v2/host-ports.schema.json";
const PUBLIC_MEDIA: &str = "https://bgxiong.local/vendor-adapter/v2/media-input.schema.json";

fn invalid(path: impl Into<String>, reason: impl Into<String>) -> ContractError {
    ContractError::new("INVALID_PACKAGE", path, reason)
}
fn unsupported(path: impl Into<String>, reason: impl Into<String>) -> ContractError {
    ContractError::new("UNSUPPORTED_FEATURE", path, reason)
}
fn semantics() -> &'static Value {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    DOCUMENT.get_or_init(|| {
        serde_json::from_str(include_str!("../../../protocol/profile-semantics.json"))
            .expect("published profile semantics")
    })
}
fn limit(name: &str) -> u64 {
    resource_policy()["limits"][name]
        .as_u64()
        .expect("published resource policy")
}
fn value<T: serde::Serialize>(source: &T) -> Value {
    serde_json::to_value(source).expect("public wire serialization")
}
fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
fn escaped(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// RFC6901 only: no wildcard, append operator, implicit path or expression.
fn pointer(source: &str) -> Result<Vec<String>> {
    if source.is_empty() {
        return Ok(vec![]);
    }
    if !source.starts_with('/') {
        return Err(invalid(source, "Expected an RFC6901 JSON pointer"));
    }
    source[1..]
        .split('/')
        .map(|segment| {
            let mut decoded = String::new();
            let mut chars = segment.chars();
            while let Some(ch) = chars.next() {
                if ch == '~' {
                    decoded.push(match chars.next() {
                        Some('0') => '~',
                        Some('1') => '/',
                        _ => return Err(invalid(source, "Invalid JSON pointer escape")),
                    });
                } else {
                    decoded.push(ch);
                }
            }
            Ok(decoded)
        })
        .collect()
}

struct Counter {
    size: u64,
    maximum: u64,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.size = self
            .size
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("JSON size overflow"))?;
        if self.size > self.maximum {
            return Err(io::Error::other("JSON byte budget exceeded"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn bounded(v: &Value, maximum: u64) -> Result<()> {
    serde_json::to_writer(&mut Counter { size: 0, maximum }, v).map_err(|_| {
        ContractError::new(
            "BUDGET_EXCEEDED",
            "/profile",
            "Profile value exceeds the public byte budget",
        )
    })
}

fn field_kind(schema: &Value, witness: &Value) -> bool {
    let expected = kind(witness);
    let accepts = |t: &Value| {
        t == expected
            || (t == "integer"
                && witness
                    .as_number()
                    .is_some_and(|n| n.is_i64() || n.is_u64()))
    };
    if let Some(t) = schema.get("type") {
        return if let Some(types) = t.as_array() {
            types.iter().any(accepts)
        } else {
            accepts(t)
        };
    }
    if let Some(c) = schema.get("const") {
        return kind(c) == expected;
    }
    schema
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty() && items.iter().all(|v| kind(v) == expected))
}
/// Introspection is shared with author forms; non-renderable schema structures
/// are explicitly outside this declarative runtime rather than guessed.
fn required_field(registry: &SchemaRegistry, id: &str, path: &str, witness: &Value) -> Result<()> {
    let segments = pointer(path)?;
    let mut parent = registry.input_field_schema(id, "")?;
    let mut prefix = String::new();
    for segment in segments {
        if !parent
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|r| r.iter().any(|v| v == &segment))
        {
            return Err(unsupported(path,"Source pointer must resolve through required declared object fields; use the Rust runtime for optional, array-index or conditional fields"));
        }
        prefix.push('/');
        prefix.push_str(&escaped(&segment));
        parent = registry.input_field_schema(id, &prefix)?;
    }
    if !field_kind(&parent, witness) {
        return Err(invalid(
            path,
            "Source schema and explicit mapping type witness differ",
        ));
    }
    Ok(())
}

#[derive(Clone)]
struct Mapping {
    source: String,
    source_pointer: String,
    target_pointer: String,
    witness: Value,
}
#[derive(Clone)]
enum UrlPart {
    Literal(String),
    Configuration(String),
}
struct Plan {
    definition: Value,
    request_template: Value,
    result_template: Value,
    request: Vec<Mapping>,
    response: Vec<Mapping>,
    url: Vec<UrlPart>,
    headers: Value,
    generated: Value,
    credential: Option<String>,
}
struct Capability {
    contract: CapabilityContractV2,
    raw: Value,
    registry: SchemaRegistry,
}
pub struct Profile {
    capabilities: Vec<Capability>,
    plans: BTreeMap<Key, Plan>,
    probe: Option<Key>,
    runtime_caps: Value,
    wall_ms: u64,
    output_bytes: u64,
}

fn registry(contract: &Value) -> Result<SchemaRegistry> {
    SchemaRegistry::register(
        contract["schemas"]
            .as_array()
            .expect("validated capability schemas")
            .iter()
            .map(|s| {
                (
                    s["id"].as_str().unwrap().into(),
                    s["schemaJson"].as_str().unwrap().into(),
                    s["digest"].as_str().unwrap().into(),
                )
            })
            .collect(),
    )
}
fn operation<'a>(cap: &'a Capability, id: &str, version: u64) -> Option<&'a Value> {
    cap.raw["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["id"] == id && op["version"] == version)
}
fn declared_result(
    cap: &Capability,
    id: &str,
    version: u64,
    transport: &str,
    result: &Value,
) -> Result<()> {
    let result = serde_json::from_value(result.clone())
        .map_err(|_| invalid("/response/resultTemplate", "Invalid typed operation result"))?;
    bgx_adapter_v2_validator::result::validate_declared_result(
        &cap.contract,
        id,
        version,
        transport,
        &result,
    )
}

fn source_check(
    mapping: &Mapping,
    response: bool,
    transport: &str,
    cap: &Capability,
    op: &Value,
) -> Result<()> {
    pointer(&mapping.source_pointer)?;
    match mapping.source.as_str(){
        "constant"=>{if !mapping.source_pointer.is_empty(){return Err(invalid(&mapping.source_pointer,"Constant mappings require the empty source pointer"));}Ok(())},
        "input" if transport=="submit"=>required_field(&cap.registry,op["inputSchema"]["id"].as_str().unwrap(),&mapping.source_pointer,&mapping.witness),
        "configuration"=>required_field(&cap.registry,cap.raw["configurationSchema"]["id"].as_str().unwrap(),&mapping.source_pointer,&mapping.witness),
        "task" if response=>Ok(()), // Future HTTP JSON is checked at execution, not falsely install-proven.
        "task" if matches!(transport,"query"|"cancel")=>{
            let segments=pointer(&mapping.source_pointer)?;
            if segments.first().is_some_and(|part|part=="resumeToken"){
                if let Some(id)=op["resumeSchema"].get("id").and_then(Value::as_str){let nested=segments[1..].iter().map(|part|format!("/{}",escaped(part))).collect::<String>();return required_field(&cap.registry,id,&nested,&mapping.witness);}
                return if segments.len()==1&&mapping.witness.is_null(){Ok(())}else{Err(invalid(&mapping.source_pointer,"Task has no declared resume schema"))};
            }
            let document=SchemaRegistry::public()?;let root=&document.schema("https://bgxiong.local/vendor-adapter/v2/operation-payload.schema.json").unwrap()["$defs"]["taskRef"];
            let mut field=root;for segment in pointer(&mapping.source_pointer)?{if !field["required"].as_array().is_some_and(|r|r.iter().any(|v|v==&segment)){return Err(unsupported(&mapping.source_pointer,"Task pointer is not a statically required field"));}field=field.get("properties").and_then(|p|p.get(&segment)).ok_or_else(||invalid(&mapping.source_pointer,"Unknown task field"))?;}
            if !field_kind(field,&mapping.witness){return Err(invalid(&mapping.source_pointer,"Task field and type witness differ"));}Ok(())
        },
        "media" if transport=="upload"=>required_field(&SchemaRegistry::public()?,PUBLIC_MEDIA,&mapping.source_pointer,&mapping.witness),
        "media" if transport=="submit" && mapping.source_pointer.is_empty() && mapping.witness.is_array()=>Ok(()),
        _=>Err(unsupported(&mapping.source_pointer,"Mapping source is unavailable in this transport; use a declared source or the Rust author runtime")),
    }
}
fn mappings(
    definitions: &Value,
    template: &Value,
    response: bool,
    transport: &str,
    cap: &Capability,
    op: &Value,
) -> Result<(Vec<Mapping>, Value)> {
    let mut materialized = template.clone();
    let mut targets: Vec<Vec<String>> = vec![];
    let mut compiled = vec![];
    for definition in definitions.as_array().unwrap() {
        let m = Mapping {
            source: definition["source"].as_str().unwrap().into(),
            source_pointer: definition["sourcePointer"].as_str().unwrap().into(),
            target_pointer: definition["targetPointer"].as_str().unwrap().into(),
            witness: definition["value"].clone(),
        };
        let parts = pointer(&m.target_pointer)?;
        if parts.is_empty()
            || targets
                .iter()
                .any(|prior| prior.starts_with(&parts) || parts.starts_with(prior))
        {
            return Err(invalid(
                &m.target_pointer,
                "Duplicate, root or overlapping mapping target",
            ));
        }
        targets.push(parts);
        source_check(&m, response, transport, cap, op)?;
        let target = materialized.pointer_mut(&m.target_pointer).ok_or_else(|| {
            invalid(
                &m.target_pointer,
                "Mapping target must already exist in its template",
            )
        })?;
        if !target.is_null() && kind(target) != kind(&m.witness) {
            return Err(invalid(
                &m.target_pointer,
                "Template target and mapping witness have different JSON types",
            ));
        }
        *target = m.witness.clone();
        compiled.push(m);
    }
    bounded(&materialized, limit("maxBufferedHttpBytes"))?;
    Ok((compiled, materialized))
}
fn url_parts(template: &str, cap: &Capability) -> Result<Vec<UrlPart>> {
    let mut rest = template;
    let mut result = vec![];
    while let Some(start) = rest.find("{{") {
        let literal = &rest[..start];
        if literal.contains(['{', '}']) {
            return Err(invalid("/urlTemplate", "Malformed URL template expression"));
        }
        result.push(UrlPart::Literal(literal.into()));
        let expression = &rest[start + 2..];
        let end = expression
            .find("}}")
            .ok_or_else(|| invalid("/urlTemplate", "Unclosed URL template expression"))?;
        let path = expression[..end]
            .strip_prefix("configuration:")
            .ok_or_else(|| {
                unsupported(
                    "/urlTemplate",
                    "URL templates only accept declared configuration pointers",
                )
            })?;
        if !path.starts_with('/') {
            return Err(invalid(
                "/urlTemplate",
                "URL configuration pointer must be absolute",
            ));
        }
        required_field(
            &cap.registry,
            cap.raw["configurationSchema"]["id"].as_str().unwrap(),
            path,
            &json!(""),
        )?;
        result.push(UrlPart::Configuration(path.into()));
        rest = &expression[end + 2..];
    }
    if rest.contains(['{', '}']) {
        return Err(invalid("/urlTemplate", "Malformed URL template expression"));
    }
    result.push(UrlPart::Literal(rest.into()));
    if result.iter().all(|p| matches!(p, UrlPart::Literal(_))) {
        policy::parse_url(template).map_err(|_| {
            invalid(
                "/urlTemplate",
                "Static URL must be an absolute HTTP(S) URL without user information or fragment",
            )
        })?;
    }
    Ok(result)
}
fn headers(
    request: &Value,
    manifest: &Value,
    cap: &Capability,
) -> Result<(Value, Value, Option<String>)> {
    let marker = semantics()["customHeader"]["placeholder"]
        .as_str()
        .expect("published custom header marker");
    let mode = request["credentialMode"].as_str().unwrap();
    let slot = request["credentialSlot"].as_str();
    let mut literals = vec![];
    let mut generated = vec![];
    let mut names = BTreeSet::new();
    for header in request["headers"].as_array().unwrap() {
        let name = header["name"].as_str().unwrap();
        let text = header["value"].as_str().unwrap();
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
            || reqwest::header::HeaderValue::from_str(text).is_err()
            || !names.insert(name.to_ascii_lowercase())
        {
            return Err(invalid(
                "/request/headers",
                "Invalid or duplicate HTTP header",
            ));
        }
        if text.contains(marker) {
            if mode != "custom-header"
                || text != marker
                || !generated.is_empty()
                || policy::protected_header(name)
            {
                return Err(invalid(
                    "/request/headers",
                    "Credential marker must occupy exactly one unprotected custom header value",
                ));
            }
            generated.push(json!({"name":name,"kind":mode,"targetField":name}));
        } else {
            if policy::protected_header(name) {
                return Err(invalid(
                    "/request/headers",
                    "Protected literal headers are not supported by the declarative runtime",
                ));
            }
            literals.push(header.clone());
        }
    }
    if mode == "none" {
        if slot.is_some() {
            return Err(invalid(
                "/request/credentialSlot",
                "Unauthenticated profile must use an explicit null credential slot",
            ));
        }
    } else {
        if !manifest["requestedCapabilities"]["auth"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == mode)
        {
            return Err(invalid(
                "/request/credentialMode",
                "Credential mode was not requested by the manifest",
            ));
        }
        let slot = slot.ok_or_else(|| {
            invalid(
                "/request/credentialSlot",
                "Authenticated request requires a declared nonsecret credential reference pointer",
            )
        })?;
        if !slot.starts_with('/') {
            return Err(invalid(
                slot,
                "Credential slot must be an absolute configuration pointer",
            ));
        }
        required_field(
            &cap.registry,
            cap.raw["configurationSchema"]["id"].as_str().unwrap(),
            slot,
            &json!(""),
        )?;
        if mode == "bearer" {
            generated.push(
                json!({"name":"Authorization","kind":"bearer","targetField":"Authorization"}),
            );
        } else if generated.len() != 1 {
            return Err(invalid(
                "/request/headers",
                "Custom header authentication requires exactly one explicit marker header",
            ));
        }
    }
    Ok((
        Value::Array(literals),
        Value::Array(generated),
        slot.map(str::to_owned),
    ))
}

impl Profile {
    pub fn load(package: &VerifiedPackage, bytes: &[u8]) -> Result<Self> {
        if package.manifest()["language"] != "profile"
            || package.manifest()["artifactSha256"] != hash_bytes(bytes)
        {
            return Err(invalid(
                "/artifactSha256",
                "Profile bytes differ from the verified profile package",
            ));
        }
        if bytes.len() as u64 > limit("maxStateBytes") {
            return Err(ContractError::new(
                "BUDGET_EXCEEDED",
                "/profile",
                "Profile exceeds the public state byte budget",
            ));
        }
        let raw: Value = serde_json::from_slice(bytes)
            .map_err(|_| invalid("/profile", "Profile is not valid JSON"))?;
        SchemaRegistry::validate_public("profile", &raw)?;
        let dto: ProfileV2 = serde_json::from_value(raw.clone()).map_err(|_| {
            invalid(
                "/profile",
                "Profile does not match generated public wire types",
            )
        })?;
        let binding = Binding::new("profile-installation".into(), package, 1, json!({}))?;
        CapabilityRegistry::default().register(package, &binding, &dto.r#capabilities)?;
        let mut capabilities = vec![];
        let mut routes = BTreeSet::new();
        for contract in dto.r#capabilities {
            let raw = value(&contract);
            for op in raw["operations"].as_array().unwrap() {
                if op["outputSlots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|slot| slot["kind"] != "text")
                {
                    return Err(unsupported("/outputSlots", "Declarative buffered JSON profiles cannot create leased media outputs; explicitly use the Rust author runtime"));
                }
                if op["execution"]["streaming"] == true {
                    return Err(unsupported("/execution/streaming","Declarative JSON profiles do not express streaming events; explicitly use the Rust author runtime"));
                }
                for model in raw["modelIds"].as_array().unwrap() {
                    if !routes.insert((
                        model.to_string(),
                        raw["capabilityKey"].to_string(),
                        op["id"].to_string(),
                        op["version"].to_string(),
                    )) {
                        return Err(unsupported(
                            "/capabilities",
                            "Profile transport routing identity is ambiguous across capabilities",
                        ));
                    }
                }
            }
            capabilities.push(Capability {
                registry: registry(&raw)?,
                contract,
                raw,
            });
        }
        let public = SchemaRegistry::public()?;
        let mut plans = BTreeMap::new();
        let mut probe: Option<Key> = None;
        for definition in raw["operations"].as_array().unwrap() {
            let transport = definition["transportOperation"].as_str().unwrap();
            if transport == "upload" {
                return Err(unsupported("/operations/transportOperation", "Declarative JSON profiles do not express binary upload/provenance; explicitly use the Rust author runtime"));
            }
            let id = definition["operationId"].as_str().unwrap();
            let version = definition["operationVersion"].as_u64().unwrap();
            let key = (transport.to_owned(), id.to_owned(), version);
            if plans.contains_key(&key) {
                return Err(invalid(
                    "/operations",
                    "Duplicate profile transport/operation/version mapping",
                ));
            }
            let mut compiled = None;
            for cap in &capabilities {
                let Some(op) = operation(cap, id, version) else {
                    continue;
                };
                let (request, body) = mappings(
                    &definition["request"]["mappings"],
                    &definition["request"]["bodyTemplate"],
                    false,
                    transport,
                    cap,
                    op,
                )?;
                let (response, result) = mappings(
                    &definition["response"]["mappings"],
                    &definition["response"]["resultTemplate"],
                    true,
                    transport,
                    cap,
                    op,
                )?;
                public.validate_dispatch(transport, "result", &result)?;
                if result
                    .get("outputs")
                    .and_then(Value::as_array)
                    .is_some_and(|outputs| outputs.iter().any(|output| output["kind"] == "media"))
                {
                    return Err(unsupported("/response/resultTemplate/outputs","Declarative buffered JSON profiles cannot create leased media outputs; explicitly use the Rust author runtime"));
                }
                if matches!(transport, "submit" | "query") {
                    declared_result(cap, id, version, transport, &result)?;
                }
                let url = url_parts(definition["request"]["urlTemplate"].as_str().unwrap(), cap)?;
                let (headers, generated, credential) =
                    headers(&definition["request"], package.manifest(), cap)?;
                compiled = Some(Plan {
                    definition: definition.clone(),
                    request_template: definition["request"]["bodyTemplate"].clone(),
                    result_template: definition["response"]["resultTemplate"].clone(),
                    request,
                    response,
                    url,
                    headers,
                    generated,
                    credential,
                });
                let _ = body;
            }
            let plan = compiled.ok_or_else(|| {
                invalid(
                    "/operations/operationId",
                    "Profile mapping does not identify a declared operation/version",
                )
            })?;
            if transport == "probe" {
                if let Some(prior) = probe.as_ref().and_then(|key| plans.get(key)) {
                    let prior: &Plan = prior;
                    if prior.definition["request"] != definition["request"]
                        || prior.definition["response"] != definition["response"]
                    {
                        return Err(unsupported("/operations","Connectivity probe has distinct plans but no public selection field; explicitly use the Rust runtime"));
                    }
                } else {
                    probe = Some(key.clone());
                }
            }
            plans.insert(key, plan);
        }
        for cap in &capabilities {
            for op in cap.raw["operations"].as_array().unwrap() {
                for transport in [
                    Some("submit"),
                    (op["execution"]["mode"] == "asynchronous").then_some("query"),
                    (op["execution"]["cancellation"] == "remote").then_some("cancel"),
                ]
                .into_iter()
                .flatten()
                {
                    if !plans.contains_key(&(
                        transport.into(),
                        op["id"].as_str().unwrap().into(),
                        op["version"].as_u64().unwrap(),
                    )) {
                        return Err(invalid(
                            "/operations",
                            "A declared operation is missing its required transport mapping",
                        ));
                    }
                }
            }
        }
        Ok(Self {
            runtime_caps: json!({"schemaVersion":2,"pluginId":package.identity().plugin_id,"pluginVersion":package.identity().version,"capabilityContracts":raw["capabilities"]}),
            capabilities,
            plans,
            probe,
            wall_ms: package.manifest()["limits"]["wallMs"].as_u64().unwrap(),
            output_bytes: package.manifest()["limits"]["outputBytes"]
                .as_u64()
                .unwrap(),
        })
    }
    pub fn runtime(&self) -> &'static str {
        "profile"
    }
    pub fn capabilities(&self) -> Vec<CapabilityContractV2> {
        self.capabilities
            .iter()
            .map(|cap| cap.contract.clone())
            .collect()
    }
    pub async fn invoke(
        &self,
        invocation: &Invocation,
        host: &mut dyn ManagedHost,
        control: &Control,
    ) -> std::result::Result<Response, HostError> {
        // Keep the parent's cancellation token while imposing the author's
        // complete invocation budget, including mapping and credential calls.
        let mut scoped = control.clone();
        let deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_millis(
                invocation.r#budget_ms.min(self.wall_ms),
            ))
            .ok_or_else(|| {
                policy::error(
                    "BUDGET_EXCEEDED",
                    "Profile wall budget is not representable",
                )
            })?;
        scoped.deadline = Some(
            control
                .deadline
                .map_or(deadline, |parent| parent.min(deadline)),
        );
        let control = &scoped;
        check_control(control)?;
        if invocation.r#payload_json.len() as u64 > limit("maxFrameBytes") {
            return Err(policy::error(
                "BUDGET_EXCEEDED",
                "Profile invocation payload exceeds public frame budget",
            ));
        }
        let operation_value = value(&invocation.r#operation);
        let transport = operation_value.as_str().expect("public transport enum");
        let payload: Value = serde_json::from_str(&invocation.r#payload_json)
            .map_err(|_| policy::error("INVALID_INPUT", "Invocation payload is not valid JSON"))?;
        let public = SchemaRegistry::public()
            .map_err(|_| policy::error("INTERNAL_ERROR", "Public schemas unavailable"))?;
        public
            .validate_dispatch(transport, "input", &payload)
            .map_err(|_| {
                policy::error(
                    "INVALID_INPUT",
                    "Invocation payload does not match the public operation schema",
                )
            })?;
        if transport == "capabilities" {
            return self.respond(invocation, self.runtime_caps.clone(), control);
        }
        if transport == "probe" && payload["kind"] == "package_validation" {
            return self.respond(
                invocation,
                json!({"packageValid":true,"connection":"not_checked","authentication":"not_checked","resources":"not_checked","issues":[]}),
                control,
            );
        }
        let key = if transport == "probe" {
            self.probe.as_ref()
        } else {
            None
        };
        let owned_key;
        if let Some(key) = key {
            owned_key = key.clone();
        } else if transport == "probe" {
            return error_response(
                invocation,
                "UNSUPPORTED_FEATURE",
                "This profile has no unambiguous connectivity probe",
                None,
            );
        } else {
            owned_key = (
                transport.into(),
                payload["operationId"].as_str().unwrap().into(),
                payload["operationVersion"].as_u64().unwrap(),
            );
        }
        let Some(plan) = self.plans.get(&owned_key) else {
            return error_response(
                invocation,
                "UNSUPPORTED_FEATURE",
                "This profile does not declare the requested transport operation",
                None,
            );
        };
        let cap = if transport == "probe" {
            self.capabilities
                .iter()
                .find(|c| operation(c, &owned_key.1, owned_key.2).is_some())
        } else {
            self.capabilities.iter().find(|c| {
                c.raw["capabilityKey"] == payload["capabilityKey"]
                    && c.raw["modelIds"]
                        .as_array()
                        .unwrap()
                        .contains(&payload["modelId"])
                    && operation(c, &owned_key.1, owned_key.2).is_some()
            })
        }
        .ok_or_else(|| {
            policy::error(
                "INVALID_INPUT",
                "Invocation does not identify a profile capability/model/operation",
            )
        })?;
        cap.registry
            .validate(
                cap.raw["configurationSchema"]["id"].as_str().unwrap(),
                &payload["configuration"],
            )
            .map_err(|_| {
                policy::error(
                    "INVALID_INPUT",
                    "Profile configuration does not match its declared schema",
                )
            })?;
        let body = render(
            &plan.request_template,
            &plan.request,
            &payload,
            None,
            control,
        );
        let body = match body {
            Ok(body) => body,
            Err(error) => {
                check_control(control)?;
                return Err(policy::error(if error.code=="BUDGET_EXCEEDED"{"BUDGET_EXCEEDED"}else{"INVALID_INPUT"},"Profile request mapping source is missing, oversized or has a different JSON type"));
            }
        };
        let url = render_url(&plan.url, &payload["configuration"]).map_err(|_| {
            policy::error(
                "INVALID_INPUT",
                "Profile URL configuration is missing, invalid or exceeds its budget",
            )
        })?;
        let credential = plan
            .credential
            .as_ref()
            .map(|path| {
                payload["configuration"]
                    .pointer(path)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        policy::error("INVALID_INPUT", "Profile credential reference is missing")
                    })
            })
            .transpose()?;
        if let Some(handle) = &credential {
            let call: HostPortCallV2 = serde_json::from_value(
                json!({"method":"host-credential.capabilities","args":{"handle":handle}}),
            )
            .expect("public credential capability port");
            let modes = managed_call(host, call, control).await?;
            let modes: Vec<String> = serde_json::from_value(modes).map_err(|_| {
                policy::error(
                    "INTERNAL_ERROR",
                    "Credential capability port returned invalid data",
                )
            })?;
            if !modes.iter().any(|m| {
                m == plan.definition["request"]["credentialMode"]
                    .as_str()
                    .unwrap()
            }) {
                return Err(policy::error(
                    "PERMISSION_DENIED",
                    "Credential reference does not permit the declared profile authentication mode",
                ));
            }
        }
        let bytes = serde_json::to_vec(&body).map_err(|_| {
            policy::error("INVALID_INPUT", "Profile JSON request cannot be encoded")
        })?;
        let plan = json!({"method":plan.definition["request"]["method"],"url":url,"headers":plan.headers,"generatedHeaders":plan.generated,"body":{"kind":"bytes","bytes":bytes},"sink":{"kind":"buffer","maxBytes":limit("maxBufferedHttpBytes").min(self.output_bytes)},"timeoutMs":invocation.r#budget_ms.min(self.wall_ms).min(u32::MAX as u64)});
        let call: HostPortCallV2 = serde_json::from_value(
            json!({"method":"host-http.execute","args":{"plan":plan,"credential":credential}}),
        )
        .map_err(|_| {
            policy::error(
                "INVALID_INPUT",
                "Profile HTTP plan cannot be represented by the public managed port",
            )
        })?;
        let remote = managed_call(host, call, control).await?;
        bounded(&remote, limit("maxFrameBytes")).map_err(|_| {
            policy::error(
                "BUDGET_EXCEEDED",
                "Managed HTTP response exceeds public frame budget",
            )
        })?;
        public
            .validate_at(PUBLIC_PORTS, "/$defs/httpResponse", &remote)
            .map_err(|_| {
                policy::error(
                    "INTERNAL_ERROR",
                    "Managed HTTP port returned an invalid response",
                )
            })?;
        let status = remote["status"].as_u64().unwrap();
        if !(200..300).contains(&status) {
            return error_response(
                invocation,
                "UPSTREAM_ERROR",
                "Upstream HTTP request returned an unsuccessful status",
                Some(status.to_string()),
            );
        }
        let bytes: Vec<u8> =
            serde_json::from_value(remote["payload"]["bytes"].clone()).map_err(|_| {
                policy::error(
                    "INTERNAL_ERROR",
                    "Profile requires a buffered managed HTTP response",
                )
            })?;
        if bytes.len() as u64 > limit("maxBufferedHttpBytes").min(self.output_bytes) {
            return Err(policy::error(
                "BUDGET_EXCEEDED",
                "Managed profile response exceeds its explicit buffer budget",
            ));
        }
        let remote: Value = match serde_json::from_slice(&bytes) {
            Ok(body) => body,
            Err(_) => {
                return error_response(
                    invocation,
                    "UPSTREAM_ERROR",
                    "Upstream response is not valid JSON",
                    None,
                )
            }
        };
        let plan = self.plans.get(&owned_key).expect("selected compiled plan");
        let result = match render(
            &plan.result_template,
            &plan.response,
            &payload,
            Some(&remote),
            control,
        ) {
            Ok(result) => result,
            Err(error) => {
                check_control(control)?;
                if error.code == "BUDGET_EXCEEDED" {
                    return Err(policy::error(
                        "BUDGET_EXCEEDED",
                        "Mapped response exceeds the public byte budget",
                    ));
                }
                return error_response(
                    invocation,
                    "UPSTREAM_ERROR",
                    "Upstream response mapping source is missing or has a different JSON type",
                    None,
                );
            }
        };
        if public
            .validate_dispatch(transport, "result", &result)
            .is_err()
        {
            return error_response(
                invocation,
                "UPSTREAM_ERROR",
                "Mapped response does not satisfy the public operation result schema",
                None,
            );
        }
        if result
            .get("outputs")
            .and_then(Value::as_array)
            .is_some_and(|outputs| outputs.iter().any(|output| output["kind"] == "media"))
        {
            return error_response(invocation,"UPSTREAM_ERROR","Mapped response cannot provide a leased media output through this declarative JSON runtime",None);
        }
        if matches!(transport, "submit" | "query") {
            if declared_result(cap, &owned_key.1, owned_key.2, transport, &result).is_err() {
                return error_response(
                    invocation,
                    "UPSTREAM_ERROR",
                    "Mapped response does not satisfy the declared operation contract",
                    None,
                );
            }
        }
        check_control(control)?;
        self.respond(invocation, result, control)
    }

    fn respond(
        &self,
        invocation: &Invocation,
        data: Value,
        control: &Control,
    ) -> std::result::Result<Response, HostError> {
        check_control(control)?;
        bounded(&data, self.output_bytes.min(limit("maxFrameBytes"))).map_err(|_| {
            policy::error(
                "BUDGET_EXCEEDED",
                "Profile response exceeds the package output byte budget",
            )
        })?;
        let result = response(invocation, data)?;
        check_control(control)?;
        Ok(result)
    }
}

async fn managed_call(
    host: &mut dyn ManagedHost,
    call: HostPortCallV2,
    control: &Control,
) -> std::result::Result<Value, HostError> {
    check_control(control)?;
    tokio::select! {biased;_=control.cancelled()=>Err(policy::error("CANCELLED","Profile execution was interrupted")),_=control.expired()=>Err(policy::error("TIMEOUT","Profile execution deadline exhausted")),result=host.call(call,control)=>result}
}
fn check_control(control: &Control) -> std::result::Result<(), HostError> {
    if control.is_cancelled() {
        return Err(policy::error(
            "CANCELLED",
            "Profile execution was interrupted",
        ));
    }
    if control
        .deadline
        .is_some_and(|at| std::time::Instant::now() >= at)
    {
        return Err(policy::error(
            "TIMEOUT",
            "Profile execution deadline exhausted",
        ));
    }
    Ok(())
}
fn render(
    template: &Value,
    mappings: &[Mapping],
    payload: &Value,
    remote: Option<&Value>,
    control: &Control,
) -> Result<Value> {
    let mut rendered = template.clone();
    for m in mappings {
        check_control(control).map_err(|_| {
            ContractError::new("INVALID_INPUT", "/mapping", "Profile control expired")
        })?;
        let source = match m.source.as_str() {
            "constant" => Some(&m.witness),
            "input" => payload.get("input"),
            "configuration" => payload.get("configuration"),
            "media" => payload.get("media"),
            "task" => remote.or_else(|| Some(payload)),
            _ => None,
        }
        .and_then(|source| {
            if m.source == "constant" {
                Some(source)
            } else {
                source.pointer(&m.source_pointer)
            }
        })
        .ok_or_else(|| invalid(&m.source_pointer, "Mapping source does not exist"))?;
        if kind(source) != kind(&m.witness) {
            return Err(invalid(
                &m.source_pointer,
                "Mapping source differs from its explicit JSON type witness",
            ));
        }
        bounded(source, limit("maxBufferedHttpBytes"))?;
        *rendered
            .pointer_mut(&m.target_pointer)
            .ok_or_else(|| invalid(&m.target_pointer, "Compiled mapping target disappeared"))? =
            source.clone();
        bounded(&rendered, limit("maxBufferedHttpBytes"))?;
    }
    Ok(rendered)
}
fn render_url(parts: &[UrlPart], configuration: &Value) -> Result<String> {
    let mut rendered = String::new();
    for part in parts {
        match part {
            UrlPart::Literal(text) => rendered.push_str(text),
            UrlPart::Configuration(path) => rendered.push_str(
                configuration
                    .pointer(path)
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        invalid(
                            path,
                            "URL configuration pointer must exist and contain a string",
                        )
                    })?,
            ),
        }
        if rendered.len() as u64 > limit("maxBufferedHttpBytes") {
            return Err(invalid(
                "/urlTemplate",
                "Rendered URL exceeds public byte budget",
            ));
        }
    }
    policy::parse_url(&rendered).map_err(|_| {
        invalid(
            "/urlTemplate",
            "Rendered URL must be an absolute HTTP(S) URL",
        )
    })?;
    Ok(rendered)
}
fn response(invocation: &Invocation, data: Value) -> std::result::Result<Response, HostError> {
    serde_json::from_value(
        json!({"protocolVersion":2,"operation":invocation.r#operation,"dataJson":data.to_string()}),
    )
    .map_err(|_| policy::error("INTERNAL_ERROR", "Cannot encode public profile response"))
}
fn error_response(
    invocation: &Invocation,
    code: &str,
    message: &str,
    vendor_code: Option<String>,
) -> std::result::Result<Response, HostError> {
    serde_json::from_value(json!({"protocolVersion":2,"operation":invocation.r#operation,"error":{"code":code,"message":message,"retryable":false,"vendorCode":vendor_code,"vendorRequestId":null}})).map_err(|_|policy::error("INTERNAL_ERROR","Cannot encode public profile error"))
}
