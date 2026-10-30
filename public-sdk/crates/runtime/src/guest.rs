//! Generic component ABI mapping. Schema-generated metadata is the only field
//! source; unknown fields, enum cases and integer narrowing are hard failures.
use wasmtime::{bail, ensure, Result};
use serde_json::{json, Value};
use wasmtime::component::{types::Type, Val};

pub fn validate_type(ty: &Type, name: &str, model: &Value) -> Result<()> {
    let shape = model
        .get(name)
        .ok_or_else(|| wasmtime::format_err!("unknown ABI type"))?;
    match shape["kind"].as_str() {
        Some("constant") => ensure!(
            matches!(ty, Type::U64),
            "ABI constant representation differs"
        ),
        Some("enum") => {
            let Type::Enum(e) = ty else {
                bail!("ABI enum type differs")
            };
            let actual: Vec<_> = e.names().collect();
            let expected: Vec<_> = (0..shape["values"].as_array().unwrap().len())
                .map(|i| format!("v{i}"))
                .collect();
            ensure!(actual == expected, "ABI enum cases differ");
        }
        Some("record") => {
            let Type::Record(record) = ty else {
                bail!("ABI record type differs")
            };
            let fields = shape["fields"].as_array().unwrap();
            ensure!(
                record.fields().len() == fields.len(),
                "ABI field count differs"
            );
            for (actual, expected) in record.fields().zip(fields) {
                ensure!(
                    actual.name == expected["wit"].as_str().unwrap(),
                    "ABI field name differs"
                );
                validate_expression(&actual.ty, &expected["type"], model)?;
            }
        }
        Some("variant") => {
            let Type::Variant(variant) = ty else {
                bail!("ABI variant differs")
            };
            let expected = shape["cases"].as_array().unwrap();
            ensure!(
                variant.cases().len() == expected.len(),
                "ABI variant count differs"
            );
            for (index, (actual, expected)) in variant.cases().zip(expected).enumerate() {
                ensure!(
                    actual.name == format!("v{index}"),
                    "ABI variant name differs"
                );
                if expected["kind"] == "primitive" && expected["name"] == "()" {
                    ensure!(actual.ty.is_none(), "ABI unit case has payload");
                } else {
                    validate_expression(
                        &actual
                            .ty
                            .ok_or_else(|| wasmtime::format_err!("ABI case payload missing"))?,
                        expected,
                        model,
                    )?;
                }
            }
        }
        Some("alias") => validate_expression(ty, &shape["type"], model)?,
        _ => bail!("unknown generated ABI shape"),
    }
    Ok(())
}
fn validate_expression(ty: &Type, expression: &Value, model: &Value) -> Result<()> {
    match expression["kind"].as_str() {
        Some("named") => validate_type(ty, expression["name"].as_str().unwrap(), model),
        Some("Vec") => {
            let Type::List(list) = ty else {
                bail!("ABI list type differs")
            };
            validate_expression(&list.ty(), &expression["item"], model)
        }
        Some("Option") => {
            let Type::Option(option) = ty else {
                bail!("ABI option type differs")
            };
            validate_expression(&option.ty(), &expression["item"], model)
        }
        Some("primitive") => {
            let valid = match expression["name"].as_str().unwrap() {
                "String" => matches!(ty, Type::String),
                "u8" => matches!(ty, Type::U8),
                "u16" => matches!(ty, Type::U16),
                "u32" => matches!(ty, Type::U32),
                "u64" => matches!(ty, Type::U64),
                "i64" => matches!(ty, Type::S64),
                "f64" => matches!(ty, Type::Float64),
                "bool" => matches!(ty, Type::Bool),
                _ => false,
            };
            ensure!(valid, "ABI primitive differs");
            Ok(())
        }
        _ => bail!("unknown generated ABI expression"),
    }
}
fn binding_name(name: &str) -> String {
    let name = name.replace('-', "_");
    if matches!(
        name.as_str(),
        "type"
            | "ref"
            | "async"
            | "await"
            | "self"
            | "return"
            | "match"
            | "loop"
            | "use"
            | "mod"
            | "const"
            | "static"
            | "fn"
            | "impl"
            | "trait"
            | "enum"
            | "struct"
            | "move"
            | "where"
            | "in"
            | "for"
            | "as"
    ) {
        name + "_"
    } else {
        name
    }
}
fn rust_case(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

pub fn from_val(value: &Val) -> Result<Value> {
    Ok(match value {
        Val::Bool(v) => json!(v),
        Val::U8(v) => json!(v),
        Val::U16(v) => json!(v),
        Val::U32(v) => json!(v),
        Val::U64(v) => json!(v),
        Val::S8(v) => json!(v),
        Val::S16(v) => json!(v),
        Val::S32(v) => json!(v),
        Val::S64(v) => json!(v),
        Val::Float32(v) => {
            ensure!(v.is_finite(), "nonfinite component number");
            json!(v)
        }
        Val::Float64(v) => {
            ensure!(v.is_finite(), "nonfinite component number");
            json!(v)
        }
        Val::String(v) => json!(v),
        Val::Enum(v) => json!(rust_case(v)),
        Val::List(v) => Value::Array(v.iter().map(from_val).collect::<Result<_>>()?),
        Val::Record(v) => {
            let mut result = serde_json::Map::new();
            for (name, value) in v {
                ensure!(
                    result
                        .insert(binding_name(name), from_val(value)?)
                        .is_none(),
                    "duplicate component field"
                );
            }
            Value::Object(result)
        }
        Val::Variant(name, Some(v)) => {
            let mut result = serde_json::Map::new();
            result.insert(rust_case(name), from_val(v)?);
            Value::Object(result)
        }
        Val::Variant(name, None) => json!(rust_case(name)),
        Val::Option(Some(v)) => from_val(v)?,
        Val::Option(None) => Value::Null,
        _ => bail!("component value is outside the public ABI"),
    })
}

pub fn to_val(ty: &Type, value: &Value) -> Result<Val> {
    let invalid = || wasmtime::format_err!("component value has wrong shape or integer range");
    Ok(match ty {
        Type::Bool => Val::Bool(value.as_bool().ok_or_else(invalid)?),
        Type::String => Val::String(value.as_str().ok_or_else(invalid)?.into()),
        Type::U8 => Val::U8(u8::try_from(value.as_u64().ok_or_else(invalid)?)?),
        Type::U16 => Val::U16(u16::try_from(value.as_u64().ok_or_else(invalid)?)?),
        Type::U32 => Val::U32(u32::try_from(value.as_u64().ok_or_else(invalid)?)?),
        Type::U64 => Val::U64(value.as_u64().ok_or_else(invalid)?),
        Type::S64 => Val::S64(value.as_i64().ok_or_else(invalid)?),
        Type::Float64 => {
            let v = value.as_f64().ok_or_else(invalid)?;
            ensure!(v.is_finite(), "nonfinite number");
            Val::Float64(v)
        }
        Type::List(list) => Val::List(
            value
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(|v| to_val(&list.ty(), v))
                .collect::<Result<_>>()?,
        ),
        Type::Record(record) => {
            let fields = record.fields().collect::<Vec<_>>();
            ensure!(
                value.as_object().ok_or_else(invalid)?.len() == fields.len(),
                "component record has unknown or missing fields"
            );
            Val::Record(
                fields
                    .into_iter()
                    .map(|field| {
                        let input = value.get(binding_name(field.name)).ok_or_else(invalid)?;
                        Ok((field.name.into(), to_val(&field.ty, input)?))
                    })
                    .collect::<Result<_>>()?,
            )
        }
        Type::Enum(enumeration) => Val::Enum(
            enumeration
                .names()
                .find(|name| value.as_str() == Some(rust_case(name).as_str()))
                .ok_or_else(invalid)?
                .into(),
        ),
        Type::Option(option) => Val::Option(if value.is_null() {
            None
        } else {
            Some(Box::new(to_val(&option.ty(), value)?))
        }),
        Type::Variant(variant) => {
            let mut selected = None;
            for case in variant.cases() {
                let key = rust_case(case.name);
                if case.ty.is_none() && value.as_str() == Some(&key) {
                    selected = Some(Val::Variant(case.name.into(), None));
                    break;
                }
                if value.as_object().is_some_and(|o| o.len() == 1) {
                    if let (Some(payload), Some(ty)) = (value.get(&key), case.ty) {
                        selected = Some(Val::Variant(
                            case.name.into(),
                            Some(Box::new(to_val(&ty, payload)?)),
                        ));
                        break;
                    }
                }
            }
            selected.ok_or_else(invalid)?
        }
        _ => bail!("unsupported component ABI primitive"),
    })
}
