//! Properties of the GetGrokCreditsConfig gRPC-web decoder.
//!
//! Fixtures are encoded here. A 0..=1 ratio becomes a 0..=100 percent;
//! a 0..=100 float stays a percent; non-zero grpc-status yields no snapshot.

use super::decode_credits_frame;

fn encode_varint(mut value: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    while value > 0x7f {
        bytes.push(u8::try_from((value & 0x7f) | 0x80).expect("byte"));
        value >>= 7;
    }
    bytes.push(u8::try_from(value).expect("byte"));
    bytes
}

fn proto_tag(field: u32, wire: u32) -> Vec<u8> {
    encode_varint((field << 3) | wire)
}

fn proto_len(field: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = proto_tag(field, 2);
    out.extend(encode_varint(u32::try_from(payload.len()).expect("len")));
    out.extend_from_slice(payload);
    out
}

fn proto_fixed32(field: u32, value: f32) -> Vec<u8> {
    let mut out = proto_tag(field, 5);
    out.extend_from_slice(&value.to_le_bytes());
    out
}

fn grpc_frame(payload: &[u8], flags: u8) -> Vec<u8> {
    let mut header = vec![flags, 0, 0, 0, 0];
    let len = u32::try_from(payload.len()).expect("len").to_be_bytes();
    header[1..5].copy_from_slice(&len);
    header.extend_from_slice(payload);
    header
}

fn credits_payload(usage: Option<f32>, seconds: Option<u32>) -> Vec<u8> {
    let mut inner = Vec::new();
    if let Some(usage) = usage {
        inner.extend(proto_fixed32(1, usage));
    }
    if let Some(seconds) = seconds {
        let mut ts = proto_tag(1, 0);
        ts.extend(encode_varint(seconds));
        inner.extend(proto_len(5, &ts));
    }
    proto_len(1, &inner)
}

#[test]
fn a_unit_interval_ratio_becomes_a_percent() {
    let seconds = 1_788_307_200_u32;
    let framed = grpc_frame(&credits_payload(Some(0.5), Some(seconds)), 0);
    let decoded = decode_credits_frame(&framed).expect("snapshot");
    assert_eq!(decoded.used_percent, Some(50));
    assert_eq!(decoded.reset_at_ms, Some(i64::from(seconds) * 1000));
}

#[test]
fn a_zero_to_hundred_float_stays_a_percent() {
    let decoded = decode_credits_frame(&credits_payload(Some(42.4), None)).expect("snapshot");
    assert_eq!(decoded.used_percent, Some(42));
    assert!(decoded.reset_at_ms.is_none());
}

#[test]
fn values_above_one_hundred_are_not_percents() {
    assert!(decode_credits_frame(&credits_payload(Some(150.0), None)).is_none());
}

#[test]
fn non_zero_grpc_status_trailers_yield_no_snapshot() {
    let trailer = grpc_frame(b"grpc-status: 16\r\ngrpc-message: no-credentials\r\n", 0x80);
    assert_eq!(decode_credits_frame(&trailer), None);
}

#[test]
fn empty_buffer_is_not_a_snapshot() {
    assert_eq!(decode_credits_frame(&[]), None);
    assert!(decode_credits_frame(&super::EMPTY_FRAME).is_none());
}
