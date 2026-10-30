use crate::ContractError;
use jsonschema::{Draft, Registry, Retrieve, Uri, Validator};
use referencing::{Resolver, ResourceRef};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn policy() -> Value {
    serde_json::from_str(include_str!("../../../protocol/schema-policy.json"))
        .expect("published policy JSON")
}

pub(crate) fn public_documents() -> Vec<Value> {
    macro_rules! files { ($($name:literal),* $(,)?) => { vec![$(
        serde_json::from_str(include_str!(concat!("../../../schemas/", $name, ".schema.json"))).expect("published schema JSON")
    ),*] }; }
    files![
        "invocation",
        "response",
        "capability-contract",
        "control-operations",
        "operation-payload",
        "result",
        "media-input",
        "usage",
        "stream-event",
        "ui-schema",
        "interaction-values",
        "manifest",
        "host-ports",
        "worker-envelope",
        "checkpoint",
        "profile",
        "policy",
        "host-channel-v1"
    ]
}

#[derive(Clone)]
struct DenyRetrieval;
impl Retrieve for DenyRetrieval {
    fn retrieve(
        &self,
        _uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("unregistered local resource; retrieval denied".into())
    }
}

/// Schema positions only. Keys inside default/const/examples are application data.
fn children(node: &Value) -> Vec<(String, &Value)> {
    let mut result = Vec::new();
    for key in [
        "$defs",
        "properties",
        "patternProperties",
        "dependentSchemas",
    ] {
        if let Some(map) = node.get(key).and_then(Value::as_object) {
            for (name, value) in map {
                let escaped = name.replace('~', "~0").replace('/', "~1");
                result.push((format!("/{key}/{escaped}"), value));
            }
        }
    }
    for key in [
        "additionalProperties",
        "propertyNames",
        "items",
        "contains",
        "unevaluatedItems",
        "unevaluatedProperties",
        "not",
        "if",
        "then",
        "else",
    ] {
        if let Some(value) = node.get(key) {
            result.push((format!("/{key}"), value));
        }
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(array) = node.get(key).and_then(Value::as_array) {
            for (index, value) in array.iter().enumerate() {
                result.push((format!("/{key}/{index}"), value));
            }
        }
    }
    result
}

fn inspect(
    node: &Value,
    path: &str,
    depth: u64,
    count: &mut u64,
    rules: &Value,
    closed: bool,
) -> Result<(), ContractError> {
    *count += 1;
    if depth > rules["limits"]["maxSchemaDepth"].as_u64().unwrap()
        || *count > rules["limits"]["maxSchemaNodes"].as_u64().unwrap()
    {
        return Err(ContractError::new(
            "INVALID_PACKAGE",
            path,
            "schema resource budget exceeded",
        ));
    }
    let Some(map) = node.as_object() else {
        return if node.is_boolean() {
            Ok(())
        } else {
            Err(ContractError::new(
                "INVALID_PACKAGE",
                path,
                "schema must be an object or boolean",
            ))
        };
    };
    for key in map.keys() {
        if !rules["supportedKeywords"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some(key))
        {
            return Err(ContractError::new(
                "INVALID_PACKAGE",
                format!("{path}/{key}"),
                "unsupported schema keyword",
            ));
        }
    }
    if let Some(dialect) = node.get("$schema") {
        if dialect != &rules["dialect"] {
            return Err(ContractError::new(
                "UNSUPPORTED_VERSION",
                path,
                "unsupported schema dialect",
            ));
        }
    }
    let object = node.get("type").is_some_and(|t| {
        t == "object"
            || t.as_array()
                .is_some_and(|a| a.iter().any(|t| t == "object"))
    });
    if closed && object {
        let is_closed = node.get("properties").is_some_and(Value::is_object)
            && node.get("required").is_some_and(Value::is_array)
            && node.get("additionalProperties") == Some(&Value::Bool(false));
        let is_dictionary = node
            .get("additionalProperties")
            .is_some_and(Value::is_object)
            && node.get("propertyNames").is_some_and(Value::is_object);
        if !is_closed && !is_dictionary {
            return Err(ContractError::new(
                "INVALID_PACKAGE",
                path,
                "objects require explicit closed fields or a typed dictionary",
            ));
        }
    }
    for (suffix, child) in children(node) {
        inspect(
            child,
            &(path.to_owned() + &suffix),
            depth + 1,
            count,
            rules,
            closed,
        )?;
    }
    Ok(())
}

fn resource_uri(id: &str) -> Result<String, ContractError> {
    let uri = if id.contains(':') {
        id.to_owned()
    } else {
        format!("https://bgxiong.local/resources/{id}")
    };
    let parsed = jsonschema::uri::from_str(&uri)
        .map_err(|_| ContractError::new("INVALID_PACKAGE", id, "invalid resource URI"))?;
    if parsed.as_str().contains('#') {
        return Err(ContractError::new(
            "INVALID_PACKAGE",
            id,
            "resource identity cannot include a fragment",
        ));
    }
    Ok(parsed.to_string())
}

fn collect_ids(
    node: &Value,
    base: &str,
    owner: &str,
    pointer: &str,
    ids: &mut BTreeMap<String, (String, String)>,
) -> Result<(), ContractError> {
    let base_uri = jsonschema::uri::from_str(base)
        .map_err(|_| ContractError::new("INVALID_PACKAGE", owner, "invalid resource base"))?;
    let scoped = if let Some(id) = node.get("$id").and_then(Value::as_str) {
        jsonschema::uri::resolve_against(&base_uri.borrow(), id)
            .map_err(|_| {
                ContractError::new(
                    "INVALID_PACKAGE",
                    pointer,
                    "invalid scoped resource identity",
                )
            })?
            .to_string()
    } else {
        base.to_owned()
    };
    let mut names = Vec::new();
    if pointer.is_empty() || node.get("$id").is_some() {
        names.push(scoped.clone());
    }
    if let Some(anchor) = node.get("$anchor").and_then(Value::as_str) {
        names.push(format!("{scoped}#{anchor}"));
    }
    for uri in names {
        let key = (owner.to_owned(), pointer.to_owned());
        if ids.get(&uri).is_some_and(|prior| prior != &key) {
            return Err(ContractError::new(
                "INVALID_PACKAGE",
                pointer,
                "duplicate scoped resource identity or anchor",
            ));
        }
        ids.insert(uri, key);
    }
    for (suffix, child) in children(node) {
        collect_ids(child, &scoped, owner, &(pointer.to_owned() + &suffix), ids)?;
    }
    Ok(())
}

fn visit(
    node: &Value,
    resolver: &Resolver<'_>,
    visiting: &mut BTreeSet<(usize, String)>,
    done: &mut BTreeSet<(usize, String)>,
    ref_depth: u64,
    rules: &Value,
) -> Result<(), ContractError> {
    let key = (
        node as *const Value as usize,
        resolver.base_uri().to_string(),
    );
    if done.contains(&key) {
        return Ok(());
    }
    if ref_depth > rules["limits"]["maxRefDepth"].as_u64().unwrap() || !visiting.insert(key.clone())
    {
        return Err(ContractError::new(
            "INVALID_PACKAGE",
            "/",
            "recursive or over-budget schema references",
        ));
    }
    if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        let resolved = resolver.lookup(reference).map_err(|_| {
            ContractError::new(
                "INVALID_PACKAGE",
                reference,
                "reference is not an authorized local resource",
            )
        })?;
        visit(
            resolved.contents(),
            resolved.resolver(),
            visiting,
            done,
            ref_depth + 1,
            rules,
        )?;
    }
    for (_, child) in children(node) {
        let scoped = resolver
            .in_subresource(ResourceRef::new(child, Draft::Draft202012))
            .map_err(|_| ContractError::new("INVALID_PACKAGE", "/", "invalid scoped resource"))?;
        visit(child, &scoped, visiting, done, ref_depth, rules)?;
    }
    visiting.remove(&key);
    done.insert(key);
    Ok(())
}

pub struct SchemaRegistry {
    documents: BTreeMap<String, Value>,
    compiled: BTreeMap<String, Validator>,
    prepared: Registry<'static>,
}

fn resolve_node<'r>(
    mut node: &'r Value,
    mut scoped: Resolver<'r>,
    pointer: &str,
) -> Result<(&'r Value, Resolver<'r>), ContractError> {
    loop {
        let Some(reference) = node.get("$ref").and_then(Value::as_str) else {
            return Ok((node, scoped));
        };
        let target = scoped.lookup(reference).map_err(|_| {
            ContractError::new(
                "INVALID_PACKAGE",
                pointer,
                "unresolved input field reference",
            )
        })?;
        node = target.contents();
        scoped = target.resolver().clone();
    }
}

impl SchemaRegistry {
    /// Inputs are (resource ID, exact UTF-8 schemaJson, expected raw-byte digest).
    pub fn register(resources: Vec<(String, String, String)>) -> Result<Self, ContractError> {
        let rules = policy();
        if resources.len() as u64 > rules["limits"]["maxSchemasPerCapability"].as_u64().unwrap() {
            return Err(ContractError::new(
                "INVALID_PACKAGE",
                "/schemas",
                "too many schema resources",
            ));
        }
        let mut docs = BTreeMap::new();
        for schema in public_documents() {
            docs.insert(schema["$id"].as_str().unwrap().to_owned(), schema);
        }
        for (id, source, expected) in resources {
            if docs.contains_key(&id) {
                return Err(ContractError::new(
                    "INVALID_PACKAGE",
                    id,
                    "duplicate or reserved schema ID",
                ));
            }
            if source.len() as u64 > rules["limits"]["maxSchemaBytes"].as_u64().unwrap()
                || hash_bytes(source.as_bytes()) != expected
            {
                return Err(ContractError::new(
                    "INVALID_PACKAGE",
                    id,
                    "schema bytes or digest invalid",
                ));
            }
            let schema: Value = serde_json::from_str(&source)
                .map_err(|_| ContractError::new("INVALID_PACKAGE", &id, "invalid schema JSON"))?;
            inspect(&schema, &id, 0, &mut 0, &rules, true)?;
            jsonschema::meta::validate(&schema).map_err(|_| {
                ContractError::new("INVALID_PACKAGE", &id, "invalid JSON Schema 2020-12")
            })?;
            docs.insert(id, schema);
        }
        let mut ids = BTreeMap::new();
        for id in docs.keys() {
            ids.insert(resource_uri(id)?, (id.clone(), String::new()));
        }
        for (id, doc) in &docs {
            collect_ids(doc, &resource_uri(id)?, id, "", &mut ids)?;
        }
        let mut builder = Registry::new()
            .draft(Draft::Draft202012)
            .retriever(DenyRetrieval);
        for (id, doc) in &docs {
            builder = builder.add(resource_uri(id)?, doc.clone()).map_err(|_| {
                ContractError::new("INVALID_PACKAGE", id, "resource registration failed")
            })?;
        }
        let prepared: Registry<'static> = builder.prepare().map_err(|_| {
            ContractError::new(
                "INVALID_PACKAGE",
                "/schemas",
                "reference closure failed without external retrieval",
            )
        })?;
        let mut done = BTreeSet::new();
        let mut compiled = BTreeMap::new();
        for (id, doc) in &docs {
            let uri = resource_uri(id)?;
            let resolver = prepared.resolver(jsonschema::uri::from_str(&uri).unwrap());
            let root = resolver.lookup("").map_err(|_| {
                ContractError::new("INVALID_PACKAGE", id, "root resource unavailable")
            })?;
            let scoped = root
                .resolver()
                .in_subresource(ResourceRef::new(root.contents(), Draft::Draft202012))
                .map_err(|_| ContractError::new("INVALID_PACKAGE", id, "root scope invalid"))?;
            visit(
                root.contents(),
                &scoped,
                &mut BTreeSet::new(),
                &mut done,
                0,
                &rules,
            )?;
            let validator = jsonschema::draft202012::options()
                .with_registry(&prepared)
                .offline()
                .with_base_uri(uri)
                .should_validate_formats(true)
                .should_ignore_unknown_formats(false)
                .build(doc)
                .map_err(|_| {
                    ContractError::new("INVALID_PACKAGE", id, "schema compilation failed")
                })?;
            compiled.insert(id.clone(), validator);
        }
        Ok(Self {
            documents: docs,
            compiled,
            prepared,
        })
    }

    pub fn public() -> Result<Self, ContractError> {
        Self::register(Vec::new())
    }
    pub fn schema(&self, id: &str) -> Option<&Value> {
        self.documents.get(id)
    }
    /// Resolve a JSON Pointer through object properties using the native registry's
    /// scoped IDs, anchors and references; this is structural form introspection.
    pub fn input_field_schema(&self, id: &str, pointer: &str) -> Result<Value, ContractError> {
        if !pointer.is_empty() && !pointer.starts_with('/') {
            return Err(ContractError::new(
                "INVALID_PACKAGE",
                pointer,
                "field pointer must be absolute",
            ));
        }
        let resolver = self
            .prepared
            .resolver(jsonschema::uri::from_str(&resource_uri(id)?).unwrap());
        let root = resolver
            .lookup("")
            .map_err(|_| ContractError::new("INVALID_PACKAGE", id, "input schema unavailable"))?;
        let mut node = root.contents();
        let mut scoped = root
            .resolver()
            .in_subresource(ResourceRef::new(node, Draft::Draft202012))
            .map_err(|_| {
                ContractError::new("INVALID_PACKAGE", pointer, "input schema scope invalid")
            })?;
        for segment in pointer
            .strip_prefix('/')
            .into_iter()
            .flat_map(|path| path.split('/'))
        {
            (node, scoped) = resolve_node(node, scoped, pointer)?;
            let key = segment.replace("~1", "/").replace("~0", "~");
            node = node
                .get("properties")
                .and_then(|p| p.get(&key))
                .ok_or_else(|| {
                    ContractError::new(
                        "UNSUPPORTED_FEATURE",
                        pointer,
                        "field path cannot be rendered from this schema structure",
                    )
                })?;
            scoped = scoped
                .in_subresource(ResourceRef::new(node, Draft::Draft202012))
                .map_err(|_| {
                    ContractError::new("INVALID_PACKAGE", pointer, "field scope invalid")
                })?;
        }
        let (node, _) = resolve_node(node, scoped, pointer)?;
        Ok(node.clone())
    }
    pub fn validate(&self, id: &str, input: &Value) -> Result<(), ContractError> {
        self.compiled
            .get(id)
            .ok_or_else(|| ContractError::new("INVALID_PACKAGE", id, "schema not registered"))?
            .validate(input)
            .map_err(|error| {
                ContractError::new(
                    "INVALID_INPUT",
                    error.instance_path().to_string(),
                    "value does not satisfy registered schema",
                )
            })
    }

    pub fn validate_public(name: &str, input: &Value) -> Result<(), ContractError> {
        Self::public()?.validate(
            &format!("https://bgxiong.local/vendor-adapter/v2/{name}.schema.json"),
            input,
        )
    }

    pub fn validate_at(&self, id: &str, pointer: &str, input: &Value) -> Result<(), ContractError> {
        let target = format!("{}#{pointer}", resource_uri(id)?);
        let validator = jsonschema::draft202012::options()
            .with_registry(&self.prepared)
            .offline()
            .should_validate_formats(true)
            .should_ignore_unknown_formats(false)
            .build(&json!({"$ref":target}))
            .map_err(|_| {
                ContractError::new("INVALID_PACKAGE", pointer, "unresolved schema fragment")
            })?;
        validator.validate(input).map_err(|e| {
            ContractError::new(
                "INVALID_INPUT",
                e.instance_path().to_string(),
                "value does not satisfy schema fragment",
            )
        })
    }

    pub fn validate_dispatch(
        &self,
        operation: &str,
        side: &str,
        input: &Value,
    ) -> Result<(), ContractError> {
        let table: Value = serde_json::from_str(include_str!("../../../operation-contracts.json"))
            .expect("operation dispatch JSON");
        let entry = &table["operations"][operation][side];
        let choices: Vec<&Value> = if let Some(a) = entry.as_array() {
            a.iter().collect()
        } else {
            vec![entry]
        };
        for target in choices {
            let Some(target) = target.as_str() else {
                return Err(ContractError::new(
                    "INVALID_INPUT",
                    "/operation",
                    "unknown operation or schema side",
                ));
            };
            let (file, pointer) = target.split_once('#').unwrap_or((target, ""));
            let id = format!("https://bgxiong.local/vendor-adapter/v2/{file}");
            if self.validate_at(&id, pointer, input).is_ok() {
                return Ok(());
            }
        }
        Err(ContractError::new(
            "INVALID_INPUT",
            "/",
            "incorrect result or payload branch for operation",
        ))
    }
}
