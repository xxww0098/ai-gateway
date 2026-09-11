//! gRPC-web decoder for grok.com `GetGrokCreditsConfig`.
//!
//! Unified-billing SuperGrok / X Premium+ often omit `creditUsagePercent` on
//! CLI JSON. This frame still carries the weekly pool (field 1 = usage
//! ratio-or-percent, field 5 = reset timestamp).

use std::collections::HashMap;

#[cfg(test)]
mod tests;

/// Empty gRPC-web data frame (5 zero bytes) used as the POST body.
pub const EMPTY_FRAME: [u8; 5] = [0, 0, 0, 0, 0];

const WIRE_VARINT: u64 = 0;
const WIRE_FIXED64: u64 = 1;
const WIRE_LEN: u64 = 2;
const WIRE_FIXED32: u64 = 5;
const TRAILER_FLAG: u8 = 0x80;

/// Weekly-pool snapshot decoded from a gRPC-web credits frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditsSnapshot {
    pub used_percent: Option<i64>,
    pub reset_at_ms: Option<i64>,
}

enum Wire {
    Varint(u64),
    Fixed32([u8; 4]),
    Fixed64,
    Len(Vec<u8>),
}

/// Decode a gRPC-web (or raw protobuf) GetGrokCreditsConfig body.
///
/// Returns `None` when the buffer is empty, trailers report a non-zero
/// `grpc-status`, or neither usage nor reset is present.
#[must_use]
pub fn decode_credits_frame(buffer: &[u8]) -> Option<CreditsSnapshot> {
    if buffer.is_empty() {
        return None;
    }
    let frames = grpc_frames(buffer);
    if let Some(status) = frames.trailers.get("grpc-status")
        && status != "0"
    {
        return None;
    }
    let payload = frames.data.into_iter().next().or_else(|| {
        if frames.framed {
            None
        } else if looks_like_protobuf(buffer) {
            Some(buffer.to_vec())
        } else {
            None
        }
    })?;
    if payload.is_empty() {
        return None;
    }
    let top = decode_fields(&payload)?;
    let nested = match top.get(&1) {
        Some(Wire::Len(bytes)) => decode_fields(bytes),
        _ => None,
    };
    let credits = nested.as_ref().unwrap_or(&top);
    let used_percent = credits
        .get(&1)
        .and_then(float32_le)
        .and_then(as_used_percent);
    let reset_at_ms = credits
        .get(&5)
        .and_then(timestamp_ms)
        .or_else(|| credits.get(&4).and_then(timestamp_ms));
    if used_percent.is_none() && reset_at_ms.is_none() {
        return None;
    }
    Some(CreditsSnapshot {
        used_percent,
        reset_at_ms,
    })
}

fn read_varint(bytes: &[u8], offset: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut index = offset;
    while index < bytes.len() {
        let byte = bytes[index];
        index += 1;
        let part = u64::from(byte & 0x7f);
        if shift >= 64 {
            return None;
        }
        value |= part.checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some((value, index));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

fn read_length(bytes: &[u8], offset: usize, size: usize) -> Option<(&[u8], usize)> {
    let end = offset.checked_add(size)?;
    if end > bytes.len() {
        return None;
    }
    Some((&bytes[offset..end], end))
}

fn decode_fields(bytes: &[u8]) -> Option<HashMap<u64, Wire>> {
    let mut fields = HashMap::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (tag, next) = read_varint(bytes, offset)?;
        offset = next;
        let field_number = tag / 8;
        let wire_type = tag % 8;
        if field_number == 0 {
            return None;
        }
        let value = match wire_type {
            WIRE_VARINT => {
                let (v, next) = read_varint(bytes, offset)?;
                offset = next;
                Wire::Varint(v)
            }
            WIRE_FIXED32 => {
                let (slice, next) = read_length(bytes, offset, 4)?;
                offset = next;
                let mut arr = [0u8; 4];
                arr.copy_from_slice(slice);
                Wire::Fixed32(arr)
            }
            WIRE_FIXED64 => {
                let (_slice, next) = read_length(bytes, offset, 8)?;
                offset = next;
                Wire::Fixed64
            }
            WIRE_LEN => {
                let (len, next) = read_varint(bytes, offset)?;
                let (slice, end) = read_length(bytes, next, usize::try_from(len).ok()?)?;
                offset = end;
                Wire::Len(slice.to_vec())
            }
            _ => return None,
        };
        fields.entry(field_number).or_insert(value);
    }
    Some(fields)
}

fn float32_le(field: &Wire) -> Option<f32> {
    let Wire::Fixed32(bytes) = field else {
        return None;
    };
    let value = f32::from_le_bytes(*bytes);
    value.is_finite().then_some(value)
}

fn timestamp_ms(field: &Wire) -> Option<i64> {
    let Wire::Len(bytes) = field else {
        return None;
    };
    let nested = decode_fields(bytes)?;
    let seconds = match nested.get(&1) {
        Some(Wire::Varint(v)) => *v,
        _ => 0,
    };
    let nanos = match nested.get(&2) {
        Some(Wire::Varint(v)) => *v,
        _ => 0,
    };
    if seconds == 0 && nanos == 0 {
        return None;
    }
    let extra = (nanos + 500_000) / 1_000_000;
    let stamp = seconds.saturating_mul(1000).saturating_add(extra);
    let stamp = i64::try_from(stamp).ok()?;
    (stamp > 0).then_some(stamp)
}

fn as_used_percent(value: f32) -> Option<i64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let rounded = if value <= 1.0 {
        f64::from(value) * 100.0
    } else if value <= 100.0 {
        f64::from(value)
    } else {
        return None;
    };
    Some(rounded.round().clamp(0.0, 100.0) as i64)
}

struct Frames {
    data: Vec<Vec<u8>>,
    trailers: HashMap<String, String>,
    framed: bool,
}

fn grpc_frames(bytes: &[u8]) -> Frames {
    let mut data = Vec::new();
    let mut trailers = HashMap::new();
    let mut offset = 0;
    while offset + 5 <= bytes.len() {
        let flags = bytes[offset];
        let length = u32::from_be_bytes([
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
        ]) as usize;
        let start = offset + 5;
        let end = start.saturating_add(length);
        if end > bytes.len() {
            break;
        }
        let payload = &bytes[start..end];
        if flags & TRAILER_FLAG != 0 {
            if let Ok(text) = std::str::from_utf8(payload) {
                for line in text.split(['\r', '\n']) {
                    let Some((key, value)) = line.split_once(':') else {
                        continue;
                    };
                    let key = key.trim().to_ascii_lowercase();
                    if key.is_empty() {
                        continue;
                    }
                    trailers.insert(key, decode_uri(value.trim()));
                }
            }
        } else {
            data.push(payload.to_vec());
        }
        offset = end;
    }
    Frames {
        data,
        trailers,
        framed: offset > 0,
    }
}

fn looks_like_protobuf(bytes: &[u8]) -> bool {
    let Some(first) = bytes.first() else {
        return false;
    };
    let field_number = first >> 3;
    let wire_type = first & 0x07;
    field_number > 0 && matches!(wire_type, 0 | 1 | 2 | 5)
}

fn decode_uri(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_owned())
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
