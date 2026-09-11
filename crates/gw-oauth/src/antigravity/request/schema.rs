//! Tool schema normalization: $ref dereference, meta-schema strip, and the
//! legacy custom-tool allowlist for Claude / GPT-OSS bridges.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{is_plain_object, trimmed, uses_legacy_tool_parameters};

const CUSTOM_TOOL_SCHEMA_ALLOW: &[&str] = &[
    "type",
    "description",
    "properties",
    "required",
    "items",
    "enum",
];

fn dereference_schema(
    schema: &Value,
    root_defs: &Map<String, Value>,
    visited: &mut HashSet<String>,
) -> Value {
    match schema {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| dereference_schema(item, root_defs, visited))
                .collect(),
        ),
        Value::Object(map) => {
            let mut defs = root_defs.clone();
            if let Some(Value::Object(more)) = map.get("$defs") {
                defs.extend(more.clone());
            }
            if let Some(Value::Object(more)) = map.get("definitions") {
                defs.extend(more.clone());
            }
            if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                if !visited.insert(r.to_owned()) {
                    return schema.clone();
                }
                if let Some(name) = r
                    .strip_prefix("#/$defs/")
                    .or_else(|| r.strip_prefix("#/definitions/"))
                    && let Some(resolved) = defs.get(name)
                {
                    let resolved = dereference_schema(resolved, &defs, visited);
                    let mut rest = map.clone();
                    rest.remove("$ref");
                    let rest_cleaned = dereference_schema(&Value::Object(rest), &defs, visited);
                    return if is_plain_object(&resolved) && is_plain_object(&rest_cleaned) {
                        let mut merged = resolved.as_object().cloned().unwrap_or_default();
                        if let Some(obj) = rest_cleaned.as_object() {
                            merged.extend(obj.clone());
                        }
                        Value::Object(merged)
                    } else {
                        resolved
                    };
                }
            }
            let mut out = Map::new();
            for (key, value) in map {
                out.insert(key.clone(), dereference_schema(value, &defs, visited));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn ensure_root_object_schema(schema: Value) -> Value {
    if !is_plain_object(&schema) {
        return json!({"type": "object", "properties": {}});
    }
    if schema.get("type").is_none() {
        let mut obj = schema.as_object().cloned().unwrap_or_default();
        obj.insert("type".into(), json!("object"));
        if !obj.contains_key("properties") {
            obj.insert("properties".into(), json!({}));
        }
        return Value::Object(obj);
    }
    schema
}

fn strip_meta_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "$schema"
                        | "$id"
                        | "$anchor"
                        | "$dynamicAnchor"
                        | "$vocabulary"
                        | "$comment"
                        | "$defs"
                        | "definitions"
                ) {
                    continue;
                }
                out.insert(key.clone(), strip_meta_schema(value));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(strip_meta_schema).collect()),
        other => other.clone(),
    }
}

fn normalize_custom_tool_type(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_owned());
    }
    value.as_array().and_then(|arr| {
        arr.iter()
            .filter_map(Value::as_str)
            .find(|entry| *entry != "null")
            .map(str::to_owned)
    })
}

fn normalize_custom_tool_schema(schema: &Value) -> Value {
    match schema {
        Value::Array(items) => {
            Value::Array(items.iter().map(normalize_custom_tool_schema).collect())
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                if !CUSTOM_TOOL_SCHEMA_ALLOW.contains(&key.as_str()) {
                    continue;
                }
                if key == "type" {
                    if let Some(normalized) = normalize_custom_tool_type(value) {
                        out.insert("type".into(), json!(normalized));
                    }
                    continue;
                }
                if key == "properties" && is_plain_object(value) {
                    let mut props = Map::new();
                    if let Some(obj) = value.as_object() {
                        for (prop_name, prop_schema) in obj {
                            props.insert(
                                prop_name.clone(),
                                normalize_custom_tool_schema(prop_schema),
                            );
                        }
                    }
                    out.insert("properties".into(), Value::Object(props));
                    continue;
                }
                if key == "enum"
                    && let Some(arr) = value.as_array()
                    && !arr.iter().all(Value::is_string)
                {
                    continue;
                }
                out.insert(key.clone(), normalize_custom_tool_schema(value));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn json_schema_of(parameters: Option<&Value>) -> Option<Value> {
    let parameters = parameters?;
    if parameters.is_null() {
        return None;
    }
    Some(strip_meta_schema(&ensure_root_object_schema(
        dereference_schema(parameters, &Map::new(), &mut HashSet::new()),
    )))
}

pub(super) fn tool_declarations(tools: Option<&Value>, model: &str) -> Option<Value> {
    let tools = tools.and_then(Value::as_array)?;
    if tools.is_empty() {
        return None;
    }
    let legacy = uses_legacy_tool_parameters(model);
    let mut declarations = Vec::new();
    for tool in tools {
        let fn_obj = tool.get("function").unwrap_or(tool);
        let Some(name) = fn_obj.get("name").and_then(Value::as_str).and_then(trimmed) else {
            continue;
        };
        let mut decl = json!({"name": name});
        if let Some(desc) = fn_obj
            .get("description")
            .and_then(Value::as_str)
            .and_then(trimmed)
        {
            decl["description"] = json!(desc);
        }
        if let Some(schema) = json_schema_of(fn_obj.get("parameters")) {
            if legacy {
                decl["parameters"] = normalize_custom_tool_schema(&schema);
            } else {
                decl["parametersJsonSchema"] = schema;
            }
        }
        declarations.push(decl);
    }
    if declarations.is_empty() {
        None
    } else {
        Some(json!([{"functionDeclarations": declarations}]))
    }
}
