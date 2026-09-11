//! AWS eventstream framing and header codec for Kiro responses.

use serde_json::{Map, Value, json};

use super::StreamEvent;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn encode_one_header(name: &str, type_id: u8, value: &[u8]) -> Vec<u8> {
    let name_buf = name.as_bytes();
    let mut row = vec![0u8; 1 + name_buf.len() + 1 + value.len()];
    row[0] = name_buf.len() as u8;
    row[1..1 + name_buf.len()].copy_from_slice(name_buf);
    row[1 + name_buf.len()] = type_id;
    row[1 + name_buf.len() + 1..].copy_from_slice(value);
    row
}

/// Encode one eventstream frame. Header types other than string are preserved.
#[must_use]
pub fn encode_event_frame(type_name: &str, payload: &Value, message_type: &str) -> Vec<u8> {
    let mut headers = Vec::new();
    headers.extend(encode_one_header(":compacted", 1, &[]));
    let mut msg = Vec::new();
    let msg_bytes = message_type.as_bytes();
    msg.extend_from_slice(&(msg_bytes.len() as u16).to_be_bytes());
    msg.extend_from_slice(msg_bytes);
    headers.extend(encode_one_header(":message-type", 7, &msg));
    if !type_name.is_empty() {
        let mut ev = Vec::new();
        let ev_bytes = type_name.as_bytes();
        ev.extend_from_slice(&(ev_bytes.len() as u16).to_be_bytes());
        ev.extend_from_slice(ev_bytes);
        headers.extend(encode_one_header(":event-type", 7, &ev));
    }
    let mut ct = Vec::new();
    let ct_bytes = b"application/json";
    ct.extend_from_slice(&(ct_bytes.len() as u16).to_be_bytes());
    ct.extend_from_slice(ct_bytes);
    headers.extend(encode_one_header(":content-type", 7, &ct));
    headers.extend(encode_one_header(":event-id", 9, &[0u8; 16]));
    headers.extend(encode_one_header("timestamp", 8, &[0u8; 8]));
    let payload_buf = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let mut prelude = vec![0u8; 12];
    let total_len = 12 + headers.len() + payload_buf.len() + 4;
    prelude[0..4].copy_from_slice(&(total_len as u32).to_be_bytes());
    prelude[4..8].copy_from_slice(&(headers.len() as u32).to_be_bytes());
    let prelude_crc = crc32(&prelude[0..8]).to_be_bytes();
    prelude[8..12].copy_from_slice(&prelude_crc);
    let mut head = prelude;
    head.extend_from_slice(&headers);
    head.extend_from_slice(&payload_buf);
    let tail = crc32(&head).to_be_bytes();
    head.extend_from_slice(&tail);
    head
}

#[must_use]
pub fn encode_event_stream(events: &[StreamEvent]) -> Vec<u8> {
    let mut out = Vec::new();
    for event in events {
        out.extend(encode_event_frame(
            &event.type_name,
            &event.payload,
            &event.message_type,
        ));
    }
    out
}

fn parse_event_headers(buffer: &[u8]) -> Map<String, Value> {
    let mut headers = Map::new();
    let mut offset = 0usize;
    while offset < buffer.len() {
        let name_len = buffer[offset] as usize;
        offset += 1;
        if offset + name_len + 1 > buffer.len() {
            break;
        }
        let name = String::from_utf8_lossy(&buffer[offset..offset + name_len]).into_owned();
        offset += name_len;
        let type_id = buffer[offset];
        offset += 1;
        match type_id {
            0 => {
                headers.insert(name, json!(true));
            }
            1 => {
                headers.insert(name, json!(false));
            }
            2 => {
                if offset + 1 > buffer.len() {
                    break;
                }
                headers.insert(name, json!(buffer[offset] as i8));
                offset += 1;
            }
            3 => {
                if offset + 2 > buffer.len() {
                    break;
                }
                let v = i16::from_be_bytes(buffer[offset..offset + 2].try_into().unwrap_or([0, 0]));
                headers.insert(name, json!(v));
                offset += 2;
            }
            4 => {
                if offset + 4 > buffer.len() {
                    break;
                }
                let v = i32::from_be_bytes(buffer[offset..offset + 4].try_into().unwrap_or([0; 4]));
                headers.insert(name, json!(v));
                offset += 4;
            }
            5 | 8 => {
                if offset + 8 > buffer.len() {
                    break;
                }
                offset += 8;
                headers.insert(name, json!(0));
            }
            6 | 7 => {
                if offset + 2 > buffer.len() {
                    break;
                }
                let value_len =
                    u16::from_be_bytes(buffer[offset..offset + 2].try_into().unwrap_or([0, 0]))
                        as usize;
                offset += 2;
                if offset + value_len > buffer.len() {
                    break;
                }
                if type_id == 7 {
                    headers.insert(
                        name,
                        json!(
                            String::from_utf8_lossy(&buffer[offset..offset + value_len])
                                .into_owned()
                        ),
                    );
                }
                offset += value_len;
            }
            9 => {
                if offset + 16 > buffer.len() {
                    break;
                }
                offset += 16;
                headers.insert(name, json!(true));
            }
            _ => break,
        }
    }
    headers
}

/// Parse AWS eventstream. Non-string header types do not abort the frame.
#[must_use]
pub fn parse_event_stream(buffer: &[u8]) -> Vec<StreamEvent> {
    if !buffer.is_empty()
        && (buffer[0] == b'{' || buffer[0] == b'[')
        && let Ok(payload) = serde_json::from_slice::<Value>(buffer)
    {
        return vec![StreamEvent {
            type_name: "exception".to_owned(),
            message_type: "exception".to_owned(),
            payload,
        }];
    }
    let mut events = Vec::new();
    let mut buf = buffer;
    while buf.len() >= 12 {
        let total_len = u32::from_be_bytes(buf[0..4].try_into().unwrap_or([0; 4])) as usize;
        if !(16..=16 * 1024 * 1024).contains(&total_len) || buf.len() < total_len {
            break;
        }
        let headers_len = u32::from_be_bytes(buf[4..8].try_into().unwrap_or([0; 4])) as usize;
        let header_end = 12 + headers_len;
        let payload_end = total_len.saturating_sub(4);
        if header_end > payload_end {
            buf = &buf[total_len..];
            continue;
        }
        let headers = parse_event_headers(&buf[12..header_end]);
        let payload =
            serde_json::from_slice(&buf[header_end..payload_end]).unwrap_or_else(|_| json!({}));
        let type_name = headers
            .get(":event-type")
            .or_else(|| headers.get(":exception-type"))
            .or_else(|| headers.get(":message-type"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let message_type = headers
            .get(":message-type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        events.push(StreamEvent {
            type_name,
            message_type,
            payload,
        });
        buf = &buf[total_len..];
    }
    events
}
