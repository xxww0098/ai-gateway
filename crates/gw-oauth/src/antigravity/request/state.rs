//! Session pins and thoughtSignature memory: one owner for the static caches.
//!
//! Request construction and response normalization share these stores through
//! narrow helpers; neither caller owns a second copy of the session state.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use serde_json::{Value, json};

use super::{is_plain_object, trimmed};
use crate::antigravity::cache::ANTIGRAVITY_STABLE_SESSION;

const PIN_CAP: usize = 64;
const THOUGHT_SIGNATURE_SESSION_CAP: usize = 64;
const THOUGHT_SIGNATURES_PER_SESSION: usize = 256;

static SESSION_PINS: LazyLock<Mutex<PinStore>> = LazyLock::new(|| Mutex::new(PinStore::default()));
static THOUGHT_SIGNATURES: LazyLock<Mutex<SigStore>> =
    LazyLock::new(|| Mutex::new(SigStore::default()));

#[derive(Default)]
struct PinStore {
    order: VecDeque<String>,
    map: HashMap<String, PinRecord>,
}

#[derive(Default)]
struct PinRecord {
    system: Option<String>,
    tools: Option<Option<Value>>,
    thinking: Option<Option<Value>>,
}

#[derive(Default)]
struct SigStore {
    order: VecDeque<String>,
    map: HashMap<String, SessionSigs>,
}

#[derive(Default)]
struct SessionSigs {
    order: VecDeque<String>,
    map: HashMap<String, String>,
}

/// Drop in-process system/tools/thinking pins. Tests only.
pub fn reset_pins() {
    let mut store = SESSION_PINS.lock().unwrap_or_else(|e| e.into_inner());
    store.order.clear();
    store.map.clear();
}

/// Drop in-process thoughtSignature memory. Tests only.
pub fn reset_thought_signatures() {
    let mut store = THOUGHT_SIGNATURES.lock().unwrap_or_else(|e| e.into_inner());
    store.order.clear();
    store.map.clear();
}

pub(super) struct PinnedSystem {
    pub(super) parts: Vec<Value>,
    pub(super) extra: Option<String>,
}

fn lock_pins() -> std::sync::MutexGuard<'static, PinStore> {
    SESSION_PINS.lock().unwrap_or_else(|e| e.into_inner())
}

fn lock_sigs() -> std::sync::MutexGuard<'static, SigStore> {
    THOUGHT_SIGNATURES.lock().unwrap_or_else(|e| e.into_inner())
}

fn can_pin(session_id: &str) -> bool {
    !session_id.is_empty() && session_id != ANTIGRAVITY_STABLE_SESSION
}

fn pin_record<'a>(store: &'a mut PinStore, session_id: &str) -> Option<&'a mut PinRecord> {
    if !can_pin(session_id) {
        return None;
    }
    if !store.map.contains_key(session_id) {
        if store.map.len() >= PIN_CAP
            && let Some(first) = store.order.pop_front()
        {
            store.map.remove(&first);
        }
        store.order.push_back(session_id.to_owned());
        store
            .map
            .insert(session_id.to_owned(), PinRecord::default());
    }
    store.map.get_mut(session_id)
}

pub(super) fn pin_system_instruction(session_id: &str, parts: &[Value]) -> PinnedSystem {
    let text = system_text(parts);
    if !can_pin(session_id) || text.is_empty() {
        return PinnedSystem {
            parts: parts.to_vec(),
            extra: None,
        };
    }
    let mut store = lock_pins();
    let Some(record) = pin_record(&mut store, session_id) else {
        return PinnedSystem {
            parts: parts.to_vec(),
            extra: None,
        };
    };
    if record.system.is_none() {
        record.system = Some(text);
        return PinnedSystem {
            parts: parts.to_vec(),
            extra: None,
        };
    }
    let pinned = record.system.clone().expect("just set or already some");
    if pinned == text {
        return PinnedSystem {
            parts: parts.to_vec(),
            extra: None,
        };
    }
    let extra = if text.starts_with(&pinned) {
        text[pinned.len()..]
            .trim_start_matches('\n')
            .trim()
            .to_owned()
    } else {
        text
    };
    PinnedSystem {
        parts: vec![json!({"text": pinned})],
        extra: if extra.is_empty() { None } else { Some(extra) },
    }
}

pub(super) fn pin_tools(session_id: &str, tools: Option<Value>) -> Option<Value> {
    let mut store = lock_pins();
    let Some(record) = pin_record(&mut store, session_id) else {
        return tools;
    };
    if record.tools.is_none() {
        record.tools = Some(tools.clone());
        return tools;
    }
    let first = record.tools.clone().flatten();
    if tools_fingerprint(first.as_ref()) == tools_fingerprint(tools.as_ref()) {
        return first;
    }
    record.tools = Some(tools.clone());
    tools
}

pub(super) fn pin_thinking(session_id: &str, thinking: Option<Value>) -> Option<Value> {
    let next = thinking.filter(is_plain_object);
    let mut store = lock_pins();
    let Some(record) = pin_record(&mut store, session_id) else {
        return next;
    };
    if record.thinking.is_none() {
        record.thinking = Some(next.clone());
        return next;
    }
    record.thinking.clone().flatten()
}

fn system_text(parts: &[Value]) -> String {
    parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn tools_fingerprint(tools: Option<&Value>) -> String {
    let Some(Value::Array(groups)) = tools else {
        return String::new();
    };
    if groups.is_empty() {
        return String::new();
    }
    let mut decls = Vec::new();
    for group in groups {
        let Some(list) = group.get("functionDeclarations").and_then(Value::as_array) else {
            continue;
        };
        for decl in list {
            let Some(name) = decl
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            decls.push(stable_json(&json!({
                "name": name,
                "parameters": decl.get("parameters").cloned().unwrap_or(Value::Null),
                "parametersJsonSchema": decl.get("parametersJsonSchema").cloned().unwrap_or(Value::Null),
            })));
        }
    }
    decls.sort();
    decls.join("\n")
}

fn stable_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => value.to_string(),
        Value::Array(items) => {
            let inner = items.iter().map(stable_json).collect::<Vec<_>>().join(",");
            format!("[{inner}]")
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner = keys
                .into_iter()
                .map(|key| format!("{}:{}", Value::String(key.clone()), stable_json(&map[key])))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
    }
}

pub(super) fn thought_signature_of(sources: &[&Value]) -> Option<String> {
    for source in sources {
        if !is_plain_object(source) {
            continue;
        }
        if let Some(direct) = source
            .get("thoughtSignature")
            .and_then(Value::as_str)
            .and_then(trimmed)
            .or_else(|| {
                source
                    .get("thought_signature")
                    .and_then(Value::as_str)
                    .and_then(trimmed)
            })
        {
            return Some(direct);
        }
        let extra = source
            .get("extra_content")
            .filter(|v| is_plain_object(v))
            .or_else(|| source.get("extra_body").filter(|v| is_plain_object(v)));
        if let Some(extra) = extra {
            let google = extra
                .get("google")
                .filter(|v| is_plain_object(v))
                .unwrap_or(extra);
            if let Some(from_extra) = google
                .get("thought_signature")
                .and_then(Value::as_str)
                .and_then(trimmed)
                .or_else(|| {
                    google
                        .get("thoughtSignature")
                        .and_then(Value::as_str)
                        .and_then(trimmed)
                })
            {
                return Some(from_extra);
            }
        }
        if let Some(nested) = thought_signature_of(&[
            source.get("function").unwrap_or(&Value::Null),
            source.get("functionCall").unwrap_or(&Value::Null),
        ]) {
            return Some(nested);
        }
    }
    None
}

fn signature_call_key(name: &str, args: &Value) -> Option<String> {
    let n = trimmed(name)?;
    Some(format!("{n}\0{}", stable_json(args)))
}

pub(super) fn remember_thought_signature(
    session_id: Option<&str>,
    id: Option<&str>,
    name: &str,
    args: &Value,
    signature: &str,
) {
    let Some(session_id) = session_id.filter(|s| !s.is_empty()) else {
        return;
    };
    if signature.is_empty() {
        return;
    }
    let mut store = lock_sigs();
    if !store.map.contains_key(session_id) {
        if store.map.len() >= THOUGHT_SIGNATURE_SESSION_CAP
            && let Some(first) = store.order.pop_front()
        {
            store.map.remove(&first);
        }
        store.order.push_back(session_id.to_owned());
        store
            .map
            .insert(session_id.to_owned(), SessionSigs::default());
    }
    let Some(bucket) = store.map.get_mut(session_id) else {
        return;
    };
    if let Some(ck) = signature_call_key(name, args) {
        put_sig(bucket, ck, signature);
    }
    if let Some(id) = id.and_then(trimmed) {
        put_sig(bucket, format!("id:{id}"), signature);
    }
}

fn put_sig(bucket: &mut SessionSigs, key: String, signature: &str) {
    if !bucket.map.contains_key(&key) {
        bucket.order.push_back(key.clone());
    }
    bucket.map.insert(key, signature.to_owned());
    while bucket.map.len() > THOUGHT_SIGNATURES_PER_SESSION {
        if let Some(first) = bucket.order.pop_front() {
            bucket.map.remove(&first);
        } else {
            break;
        }
    }
}

pub(super) fn lookup_thought_signature(
    session_id: &str,
    id: Option<&str>,
    name: &str,
    args: &Value,
) -> Option<String> {
    let store = lock_sigs();
    let bucket = store.map.get(session_id)?;
    if let Some(ck) = signature_call_key(name, args)
        && let Some(sig) = bucket.map.get(&ck)
    {
        return Some(sig.clone());
    }
    if let Some(id) = id.and_then(trimmed) {
        return bucket.map.get(&format!("id:{id}")).cloned();
    }
    None
}

pub(super) fn attach_thought_signature_fields(target: &mut Value, signature: &str) {
    target["thoughtSignature"] = json!(signature);
    target["thought_signature"] = json!(signature);
    target["extra_content"] = json!({"google": {"thought_signature": signature}});
}
