"""Deterministic wire type generator for the public schemas, not a validator.

The full validator handles ranges, patterns and cross-field invariants. Generated
serde shapes reject unknown object fields, enum variants and missing required
nullable fields. Unsupported schema shapes stop generation rather than widening.
"""
from pathlib import Path
import argparse
import hashlib
import json
import re

ROOT = Path(__file__).resolve().parents[1]
BASE = "https://bgxiong.local/vendor-adapter/v2/"
NAMES = {
    "invocation": "Invocation", "response": "Response",
    "capability-contract": "CapabilityContractV2", "media-input": "MediaInputV2",
    "operation-payload": "OperationPayloadV2", "result": "OperationResultV2",
    "usage": "UsageV2", "stream-event": "StreamEventV1", "ui-schema": "UiSchemaV2",
    "control-operations": "ControlOperationsV2", "interaction-values": "InteractionValuesV2",
    "manifest": "ManifestV2", "worker-envelope": "WorkerEnvelopeV2",
    "host-ports": "HostPortCallV2", "checkpoint": "CheckpointV2",
    "profile": "ProfileV2", "policy": "SchemaPolicyV2",
    "host-channel-v1": "HostChannelV1",
    "synthesize.input": "SpeechInputV1", "synthesize.output": "SpeechOutputV1",
    "batch.input": "SpeechBatchInputV1", "batch.output": "SpeechBatchOutputV1",
    "voices.input": "SpeechVoicesInputV1", "voices.output": "SpeechVoicesOutputV1",
    "resume": "SpeechResumeV1", "descriptor": "SpeechDescriptorV1",
    "configuration": "SpeechRuntimeConfigurationV1", "host": "SpeechHostV1",
    "bundle": "LocalRuntimeBundleV1", "runtime": "LocalRuntimeV1", "control": "LocalRuntimeControlV1",
}
DEFS = {("operation-payload", "submit"): "OperationSubmitV2",
        ("operation-payload", "taskRef"): "OperationTaskRefV2",
        ("result", "accepted"): "SubmitAcceptedV2",
        ("result", "completed"): "SubmitCompletedV2", ("result", "query"): "QueryResult",
        ("control-operations", "runtimeCaps"): "RuntimeCapsV2"}

# Module integration receipts are release governance, never adapter wire DTOs.
# Every other schema must have an explicit wire name; unknown categories fail.
NON_WIRE_SCHEMAS = {"adoption-evidence"}

def wire_schema_paths():
    paths = sorted((ROOT / "schemas").glob("*.json"))
    paths += sorted((ROOT / "profiles/speech-v1").glob("*.schema.json"))
    paths += sorted((ROOT / "profiles/local-runtime-v1").glob("*.schema.json"))
    stems = {p.stem.removesuffix(".schema") for p in paths}
    if not set(NAMES) <= stems or stems - set(NAMES) - NON_WIRE_SCHEMAS:
        raise ValueError("missing wire schema or unclassified public schema")
    return [p for p in paths if p.stem.removesuffix(".schema") in NAMES]


def pascal(value):
    return "".join(part[:1].upper() + part[1:] for part in re.split(r"[^a-zA-Z0-9]", value) if part)


def snake(value):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", value).replace("-", "_").lower()


def quoted(value):
    return json.dumps(value, ensure_ascii=False)


class Generator:
    def __init__(self):
        self.schemas = {p.stem.removesuffix(".schema"): json.loads(p.read_text(encoding="utf-8"))
                        for p in wire_schema_paths()}
        self.refs, self.pending, self.rs, self.ts, self.shapes = {}, {}, {}, {}, {}
        for stem, schema in self.schemas.items():
            name = NAMES[stem]
            self.refs[schema["$id"]] = name
            self.pending[name] = (schema, stem)
            for key, value in schema.get("$defs", {}).items():
                name = DEFS.get((stem, key), NAMES[stem] + pascal(key))
                self.refs[schema["$id"] + "#/$defs/" + key] = name
                self.pending[name] = (value, stem)
        self.used = set(self.pending)

    def named(self, value, name, stem):
        if name in self.used:
            suffix = hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()[:8]
            name += suffix
        self.used.add(name)
        self.emit(name, value, stem)
        return name, name

    def kind(self, value, name, stem):
        if "$ref" in value:
            ref = value["$ref"]
            if ref.startswith("#"):
                ref = self.schemas[stem]["$id"] + ref
            if ref not in self.refs:
                raise ValueError("unsupported type reference: " + ref)
            return self.refs[ref], self.refs[ref]
        if "const" in value or "enum" in value or "oneOf" in value or "anyOf" in value:
            branches = value.get("anyOf", [])
            if len(branches) == 2 and any(x.get("type") == "null" for x in branches):
                branch = next(x for x in branches if x.get("type") != "null")
                r, t = self.kind(branch, name + "Value", stem)
                return "Option<" + r + ">", t + " | null"
            return self.named(value, name, stem)
        kind = value.get("type")
        if isinstance(kind, list):
            if "null" not in kind or len(kind) != 2:
                raise ValueError("unsupported primitive type union: " + repr(kind))
            r, t = self.kind({**value, "type": next(x for x in kind if x != "null")}, name + "Value", stem)
            return "Option<" + r + ">", t + " | null"
        if kind == "array":
            r, t = self.kind(value["items"], name + "Item", stem)
            return "Vec<" + r + ">", "Array<" + t + ">"
        if kind == "object":
            if "properties" in value or value.get("additionalProperties") is False:
                return self.named(value, name, stem)
            return "BTreeMap<String, Value>", "Record<string, unknown>"
        integer = "i64"
        if value.get("minimum", -1) >= 0:
            integer = "u64"
            maximum = value.get("maximum")
            if isinstance(maximum, int):
                integer = next(("u"+str(bits) for bits in (8,16,32,64) if maximum <= 2**bits-1), "u64")
        primitives = {"string": ("String", "string"), "boolean": ("bool", "boolean"),
                      "integer": (integer, "number"),
                      "number": ("f64", "number"), "null": ("()", "null"),
                      None: ("Value", "unknown")}
        if kind not in primitives:
            raise ValueError("unsupported type: " + repr(kind))
        return primitives[kind]

    def emit(self, name, value, stem):
        self.shapes[name] = value
        derive = "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n"
        if "const" in value and not isinstance(value["const"], str):
            constant = value["const"]
            assert isinstance(constant, (int, bool))
            primitive = "bool" if isinstance(constant, bool) else "u64"
            constant = json.dumps(constant)
            self.rs[name] = f'''#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct {name};
impl Serialize for {name} {{
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {{ serializer.serialize_{primitive}({constant}) }}
}}
impl<'de> Deserialize<'de> for {name} {{
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {{
        let value = {primitive}::deserialize(deserializer)?;
        if value == {constant} {{ Ok(Self) }} else {{ Err(serde::de::Error::custom("invalid protocol constant")) }}
    }}
}}
'''
            self.ts[name] = f"export type {name} = {constant};\n"
        elif "enum" in value or "const" in value:
            choices = value.get("enum", [value.get("const")])
            if not all(isinstance(x, str) for x in choices):
                raise ValueError("non-string enum requires an explicit supported representation")
            variants = [f"    #[serde(rename = {quoted(choice)})]\n    V{index}," for index, choice in enumerate(choices)]
            self.rs[name] = derive + "pub enum " + name + " {\n" + "\n".join(variants) + "\n}\n"
            self.ts[name] = "export type " + name + " = " + " | ".join(map(quoted, choices)) + ";\n"
        elif "oneOf" in value or "anyOf" in value:
            branches = value.get("oneOf", value.get("anyOf"))
            types = [self.kind(branch, name + "Branch" + str(index), stem) for index, branch in enumerate(branches)]
            self.rs[name] = derive + "#[serde(untagged)]\npub enum " + name + " {\n" + "\n".join(
                f"    V{index}({r})," for index, (r, _) in enumerate(types)) + "\n}\n"
            self.ts[name] = "export type " + name + " = " + " | ".join(t for _, t in types) + ";\n"
        elif value.get("type") == "object" and ("properties" in value or value.get("additionalProperties") is False):
            rs, ts = [], []
            required = value.get("required", [])
            for key, field in value.get("properties", {}).items():
                r, t = self.kind(field, name + pascal(key), stem)
                attributes = [f"    #[serde(rename = {quoted(key)})]"]
                if key not in required:
                    r = "Option<" + r + ">"
                    attributes.append('    #[serde(default, skip_serializing_if = "Option::is_none")]')
                elif r.startswith("Option<"):
                    attributes.append('    #[serde(deserialize_with = "crate::required_nullable")]')
                rs += attributes + ["    pub r#" + snake(key) + ": " + r + ","]
                ts.append("    " + quoted(key) + ("" if key in required else "?") + ": " + t + ";")
            self.rs[name] = derive + "#[serde(deny_unknown_fields)]\npub struct " + name + " {\n" + "\n".join(rs) + "\n}\n"
            self.ts[name] = "export interface " + name + " {\n" + "\n".join(ts) + "\n}\n"
        else:
            r, t = self.kind(value, name + "Value", stem)
            self.rs[name] = "pub type " + name + " = " + r + ";\n"
            self.ts[name] = "export type " + name + " = " + t + ";\n"

    def render(self):
        for name, (value, stem) in self.pending.items():
            self.emit(name, value, stem)
        dispatch = json.loads((ROOT / "operation-contracts.json").read_text(encoding="utf-8"))
        submit_results = dispatch["operations"]["submit"]["result"]
        self.emit("SubmitResult", {"oneOf": [{"$ref": BASE + target} for target in submit_results]}, "result")
        aliases = {"Output": "SubmitCompletedV2OutputsItem",
                   "TerminalFailure": "QueryResultTerminalFailureValue"}
        # These are aliases to generated shapes, never manually repeated fields.
        for alias, target in aliases.items():
            if target not in self.rs:
                raise ValueError("expected generated public shape absent: " + target)
            self.rs[alias] = f"pub type {alias} = {target};\n"
            self.ts[alias] = f"export type {alias} = {target};\n"
        digest = hashlib.sha256(b"".join(p.read_bytes() for p in wire_schema_paths())).hexdigest()
        header = "// Generated by tools/generate_contracts.py. Do not edit. Schema SHA-256: " + digest + "\n"
        rust = header + "use alloc::{collections::BTreeMap, string::String, vec::Vec};\nuse serde::{Deserialize, Serialize};\nuse serde_json::Value;\n\n"
        return {ROOT / "sdk/rust/src/generated.rs": rust + "\n".join(self.rs[k] for k in sorted(self.rs)) + guest_abi(self),
                ROOT / "sdk/typescript/src/generated.ts": header + "\n".join(self.ts[k] for k in sorted(self.ts))}


def guest_abi(generator):
    """Typed, exhaustive conversion; never infer a union branch from text/keys."""
    ports = generator.schemas["host-ports"]
    methods = ports["x-wit-functions"]
    roots = ["Invocation", "Response", generator.refs[ports["$id"] + "#/$defs/hostError"]]
    for method in methods:
        roots.append(generator.refs[method["arguments"]["$ref"]])
        result = generator.refs[method["returns"]["$ref"]]
        if " = ();" not in generator.rs[result]: roots.append(result)
    primitives = {"String", "()", "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64", "bool"}
    done, definitions, model = set(), [], {}

    def expression(kind):
        for container in ["Vec", "Option"]:
            if kind.startswith(container+"<") and kind.endswith(">"):
                return {"kind":container,"item":expression(kind[len(container)+1:-1])}
        return {"kind":"primitive" if kind in primitives else "named","name":kind}

    def binding_field(key):
        return key+"_" if key in {"type","ref","async","await","self","return","match","loop","use","mod","const","static","fn","impl","trait","enum","struct","move","where","in","for","as"} else key

    def convert(kind, expression, direction):
        for container in ["Vec", "Option"]:
            if kind.startswith(container + "<") and kind.endswith(">"):
                inner = kind[len(container)+1:-1]
                value = convert(inner, "item", direction)
                if container == "Vec":
                    return expression + ".into_iter().map(|item| -> Result<_, HostError> { Ok(" + value + ") }).collect::<Result<Vec<_>, HostError>>()?"
                return expression + ".map(|item| -> Result<_, HostError> { Ok(" + value + ") }).transpose()?"
        if kind in primitives: return expression
        emit(kind)
        return direction + "_" + snake(kind) + "(" + expression + ")?"

    def emit(name):
        if name in done: return
        done.add(name)
        source = generator.rs[name]
        shape = generator.shapes[name]
        if "pub struct " + name + ";" in source:
            model[name] = {"kind":"constant","value":shape["const"]}
        elif "pub struct " + name + " {" in source:
            fields = re.findall(r"pub r#([a-z0-9_]+): (.+),",source)
            wire_names = {snake(key):key for key in shape.get("properties",{})}
            model[name] = {"kind":"record","fields":[{"wire":wire_names[key],"binding":binding_field(key),"wit":key.replace("_","-"),"type":expression(kind)} for key,kind in fields]}
        elif "pub enum " + name + " {" in source:
            variants = re.findall(r"    V([0-9]+)(?:\(([^\n]+)\))?,",source)
            if "enum" in shape or "const" in shape:
                model[name] = {"kind":"enum","values":shape.get("enum",[shape.get("const")])}
            else:
                model[name] = {"kind":"variant","cases":[expression(kind) if kind else expression("()") for _,kind in variants]}
        else:
            alias=re.fullmatch(r"pub type "+name+r" = (.+);\s*",source)
            model[name]={"kind":"alias","type":expression(alias.group(1))}
        model[name]["witName"]=re.sub(r"([a-z0-9])([A-Z])",r"\1-\2",name).replace("_","-").lower()
        functions = []
        for direction in ["to", "from"]:
            incoming, outgoing = (name, "wit::"+name) if direction == "to" else ("wit::"+name, name)
            if "pub struct " + name + ";" in source:
                constant = re.search(r"serializer\.serialize_u64\(([0-9]+)\)", source).group(1)
                body = "{ let _ = value; " + constant + " }" if direction == "to" else "{ if value != " + constant + " { return Err(crate::host::invalid_wire(\"Invalid ABI protocol constant\")); } " + name + " }"
            elif "pub struct " + name + " {" in source:
                fields = re.findall(r"pub r#([a-z0-9_]+): (.+),", source)
                assignments = []
                for key, kind in fields:
                    native = "r#" + key
                    binding = binding_field(key)
                    field_in, field_out = (native, binding) if direction == "to" else (binding, native)
                    assignments.append(field_out + ": " + convert(kind, "value."+field_in, direction))
                body = outgoing + " { " + ", ".join(assignments) + " }"
            elif "pub enum " + name + " {" in source:
                variants = re.findall(r"    V([0-9]+)(?:\(([^\n]+)\))?,", source)
                branches = []
                for index, kind in variants:
                    if kind == "()":
                        pattern = incoming + "::V" + index + ("(_)" if direction == "to" else "")
                        value = outgoing + "::V" + index + ("" if direction == "to" else "(())")
                    elif kind:
                        pattern = incoming + "::V" + index + "(item)"
                        value = outgoing + "::V" + index + "(" + convert(kind, "item", direction) + ")"
                    else:
                        pattern, value = incoming + "::V" + index, outgoing + "::V" + index
                    branches.append(pattern + " => " + value)
                body = "match value { " + ", ".join(branches) + " }"
            else:
                alias = re.fullmatch(r"pub type " + name + r" = (.+);\s*", source)
                if not alias: raise ValueError("unsupported ABI conversion type: " + name)
                body = convert(alias.group(1), "value", direction)
            functions.append("pub fn " + direction + "_" + snake(name) + "(value: " + incoming + ") -> Result<" + outgoing + ", HostError> { Ok(" + body + ") }\n")
        definitions.extend(functions)

    for name in roots: emit(name)
    rust = '\n#[cfg(feature = "component-guest")]\n#[doc(hidden)]\n#[allow(dead_code)]\npub mod wire_abi {\nuse super::*;\nuse crate::host::HostError;\nuse crate::__bindings::bgxiong::vendor_adapter::types as wit;\n'
    rust += "\n".join(definitions)
    rust += "pub const BINDING_MODEL_JSON: &str = " + quoted(json.dumps(model,ensure_ascii=False,separators=(",",":"))) + ";\n"
    signatures = {method["interface"]+"."+method["name"]:{"args":generator.refs[method["arguments"]["$ref"]],"result":None if " = ();" in generator.rs[generator.refs[method["returns"]["$ref"]]] else generator.refs[method["returns"]["$ref"]],"error":roots[2]} for method in methods}
    rust += "pub const PORT_SIGNATURES_JSON: &str = " + quoted(json.dumps(signatures,separators=(",",":"))) + ";\n"
    rust += r'''
pub fn decode_binding(model:&Value,name:&str,value:&Value)->Result<Value,HostError>{
    fn walk(model:&Value,ty:&Value,value:&Value)->Result<Value,HostError>{
        let bad=||crate::host::invalid_wire("Component value does not match the generated ABI");
        match ty["kind"].as_str(){
            Some("Vec")=>Ok(Value::Array(value.as_array().ok_or_else(bad)?.iter().map(|v|walk(model,&ty["item"],v)).collect::<Result<_,_>>()?)),
            Some("Option")=>if value.is_null(){Ok(Value::Null)}else{walk(model,&ty["item"],value)},
            Some("primitive")=>if ty["name"]=="()"&&!value.is_null(){Err(bad())}else{Ok(value.clone())},
            Some("named")=>decode_binding(model,ty["name"].as_str().ok_or_else(bad)?,value),
            _=>Err(bad()),
        }
    }
    let bad=||crate::host::invalid_wire("Component value does not match the generated ABI");
    let shape=model.get(name).ok_or_else(bad)?;
    match shape["kind"].as_str(){
        Some("constant")=>if value==&shape["value"]{Ok(value.clone())}else{Err(bad())},
        Some("enum")=>shape["values"].as_array().ok_or_else(bad)?.iter().enumerate().find(|(i,_)|value.as_str()==Some(alloc::format!("V{i}").as_str())).map(|(_,v)|v.clone()).ok_or_else(bad),
        Some("variant")=>{
            for (i,ty) in shape["cases"].as_array().ok_or_else(bad)?.iter().enumerate(){
                let key=alloc::format!("V{i}");
                if value.as_str()==Some(&key)&&ty["name"]=="()"{return Ok(Value::Null);}
                if value.as_object().is_some_and(|o|o.len()==1){if let Some(payload)=value.get(&key){return walk(model,ty,payload);}}
            }Err(bad())
        },
        Some("record")=>{
            let fields=shape["fields"].as_array().ok_or_else(bad)?;let object=value.as_object().ok_or_else(bad)?;
            if object.len()!=fields.len(){return Err(bad());}
            let mut result=serde_json::Map::new();
            for field in fields{let binding=field["binding"].as_str().ok_or_else(bad)?;let input=object.get(binding).ok_or_else(bad)?;
                result.insert(field["wire"].as_str().ok_or_else(bad)?.into(),walk(model,&field["type"],input)?);}
            Ok(Value::Object(result))
        },
        Some("alias")=>walk(model,&shape["type"],value),
        _=>Err(bad()),
    }
}
pub fn port_arguments_to_wire(model:&Value,method:&str,args:&Value)->Result<HostPortCallV2,HostError>{
    let signatures:Value=serde_json::from_str(PORT_SIGNATURES_JSON).expect("generated port signatures");
    let name=signatures.get(method).and_then(|s|s["args"].as_str()).ok_or_else(||crate::host::invalid_wire("Unknown component port"))?;
    let args=decode_binding(model,name,args)?;
    serde_json::from_value(serde_json::json!({"method":method,"args":args})).map_err(|_|crate::host::invalid_wire("Invalid component port arguments"))
}
pub fn port_result_to_binding(method:&str,value:Value)->Result<Value,HostError>{match method{
'''
    for method in methods:
        key=method["interface"]+"."+method["name"]
        result=generator.refs[method["returns"]["$ref"]]
        if " = ();" in generator.rs[result]:
            body="let typed:()=serde_json::from_value(value).map_err(|_|crate::host::invalid_wire(\"Invalid null host result\"))?;serde_json::to_value(typed)"
        else:
            body="let typed:"+result+"=serde_json::from_value(value).map_err(|_|crate::host::invalid_wire(\"Invalid typed host result\"))?;serde_json::to_value(to_"+snake(result)+"(typed)?)"
        rust += quoted(key)+"=>{ "+body+".map_err(|_|crate::host::invalid_wire(\"Invalid component host result\")) },\n"
    rust += '_=>Err(crate::host::invalid_wire("Unknown component port")),\n}}\n'
    rust += "pub fn call_component(call: HostPortCallV2) -> Result<Value, HostError> {\nmatch call {\n"
    host_error = roots[2]
    for index, method in enumerate(methods):
        args = generator.refs[method["arguments"]["$ref"]]
        result = generator.refs[method["returns"]["$ref"]]
        path = "crate::__bindings::bgxiong::vendor_adapter::" + method["interface"].replace("-", "_") + "::" + method["name"].replace("-", "_")
        converted = "()" if " = ();" in generator.rs[result] else "from_" + snake(result) + "(result)?"
        rust += "HostPortCallV2::V" + str(index) + "(call) => { let args = to_" + snake(args) + "(call.r#args)?;\n"
        pattern = "()" if " = ();" in generator.rs[result] else "result"
        rust += "match " + path + "(&args) { Ok(" + pattern + ") => serde_json::to_value(" + converted + ").map_err(|_|crate::host::invalid_wire(\"Invalid host result\")), Err(error) => Err(from_" + snake(host_error) + "(error)?) } },\n"
    return rust + "}\n}\n}\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generated = Generator().render()
    assert generated == Generator().render(), "nondeterministic generator"
    profile = ROOT / "profiles/speech-v1"
    fixture = json.loads((profile / "fixtures/capability.json").read_text(encoding="utf-8"))
    resources = []
    for path in sorted(profile.glob("*.schema.json")):
        if path.name == "host.schema.json":
            continue
        source = path.read_bytes()
        resources.append({"id": json.loads(source)["$id"], "digest": hashlib.sha256(source).hexdigest(), "schemaJson": source.decode("utf-8")})
    refs = {r["id"]: {"id": r["id"], "digest": r["digest"]} for r in resources}
    registry = json.loads((ROOT / "protocol/vendor-adapter-protocol.json").read_text(encoding="utf-8"))
    for entry in registry["standardProfiles"]:
        for key in ["inputSchema", "outputSchema", "resumeSchema"]:
            assert entry[key] == refs[entry[key]["id"]], "frozen speech schema digest drift; revise the version before changing published bytes"
    fixture["schemas"] = resources
    def encoded(value):
        return json.dumps(value, ensure_ascii=False, indent=2) + "\n"
    generated[profile / "fixtures/capability.json"] = encoded(fixture)
    fixture = json.loads(encoded(fixture))
    fixture["modelIds"] = ["author-model"]
    fixture["operations"] = fixture["operations"][:1]
    caps = {"schemaVersion":2,"pluginId":"local.sdk.speech","pluginVersion":"1.0.0","capabilityContracts":[fixture]}
    generated[ROOT / "templates/tts-adapter/src/capabilities.json"] = encoded(caps)
    for path, source in generated.items():
        if args.check:
            if not path.is_file() or path.read_bytes() != source.encode("utf-8"):
                raise SystemExit("generated type drift: " + str(path))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(source.encode("utf-8"))
    print("GENERATED_CONTRACTS=PASS deterministic=PASS files=" + str(len(generated)))


if __name__ == "__main__":
    main()
