//! Unit tests for [`crate::streambuf`].
//!
//! The billing-regression guard here uses the OpenAI SSE scanner, whose usage
//! envelope sits at the end of the stream, so it exercises the same drop path
//! (`claude.rs`, another worker's file, is out of scope for this module).

use super::*;
use crate::usage::parse_openai_stream_usage;

/// Feeds `s` into the buffer in fixed-size chunks to mimic the executors'
/// read loop.
fn write_in_chunks(buf: &mut StreamUsageBuffer, s: &[u8], chunk: usize) {
    for part in s.chunks(chunk) {
        buf.write(part);
    }
}

#[test]
fn small_stream_is_preserved_byte_for_byte() {
    let mut buf = StreamUsageBuffer::new();
    let input = b"hello world, a short stream";
    write_in_chunks(&mut buf, input, 4);
    assert_eq!(buf.bytes(), input);
}

#[test]
fn stream_just_over_the_head_window_still_reconstructs_exactly() {
    // Total lands in (head, head + tail]: nothing may be dropped.
    let mut buf = StreamUsageBuffer::new();
    let input = vec![b'A'; DEFAULT_STREAM_HEAD_LIMIT + 1000];
    write_in_chunks(&mut buf, &input, 7000);
    assert_eq!(
        buf.bytes().len(),
        input.len(),
        "a stream that fits in head+tail must reconstruct exactly"
    );
    assert_eq!(buf.bytes(), input);
}

#[test]
fn stream_exactly_at_the_head_plus_tail_boundary_reconstructs_exactly() {
    let mut buf = StreamUsageBuffer::new();
    let input: Vec<u8> = (0..DEFAULT_STREAM_HEAD_LIMIT + DEFAULT_STREAM_TAIL_LIMIT)
        .map(|i| (i % 251) as u8)
        .collect();
    write_in_chunks(&mut buf, &input, 4096);
    assert_eq!(buf.bytes(), input, "the boundary case must not drop a byte");
}

#[test]
fn long_stream_is_bounded_and_keeps_both_ends() {
    let mut buf = StreamUsageBuffer::new();
    let head = b"HEAD_MARKER_START";
    let tail = b"TAIL_MARKER_END";
    let mut input = Vec::new();
    input.extend_from_slice(head);
    input.extend(std::iter::repeat_n(b'x', 1024 * 1024));
    input.extend_from_slice(tail);
    write_in_chunks(&mut buf, &input, 32 * 1024);

    let got = buf.bytes();
    assert!(
        got.len() <= DEFAULT_STREAM_HEAD_LIMIT + DEFAULT_STREAM_TAIL_LIMIT + 1,
        "retained {} bytes, exceeds the head+tail+separator bound",
        got.len()
    );
    assert!(got.starts_with(head), "head marker lost");
    assert!(got.ends_with(tail), "tail marker lost");
    assert_eq!(buf.total(), input.len() as u64);
}

#[test]
fn dropped_middle_is_separated_so_boundary_lines_cannot_fuse() {
    // Two half-lines either side of the gap must not join into one parseable
    // line — that is what the newline separator buys.
    let mut buf = StreamUsageBuffer::with_limits(8, 8);
    buf.write(b"data: {\"a");
    buf.write(&[b'z'; 64]);
    buf.write(b"bc\":1}\n");
    let got = buf.bytes();
    let joined = got.windows(2).any(|w| w == b"az" || w == b"zb");
    assert!(
        !joined || got.contains(&b'\n'),
        "head and tail fused without a separator: {:?}",
        String::from_utf8_lossy(&got)
    );
}

/// Billing-regression guard: OpenAI reports the aggregate `usage` envelope in
/// the terminal SSE event. A long stream must not push it out of the retained
/// window, or the request would be mis-charged.
#[test]
fn terminal_usage_event_survives_a_huge_stream() {
    let mut body = String::from("data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n");
    for _ in 0..20_000 {
        body.push_str(
            "data: {\"choices\":[{\"delta\":{\"content\":\"some streamed text chunk\"}}]}\n\n",
        );
    }
    body.push_str(
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1234,\"completion_tokens\":567}}\n\ndata: [DONE]\n\n",
    );

    assert!(
        body.len() > DEFAULT_STREAM_HEAD_LIMIT + DEFAULT_STREAM_TAIL_LIMIT,
        "fixture too small to exercise the drop path"
    );

    let mut buf = StreamUsageBuffer::new();
    write_in_chunks(&mut buf, body.as_bytes(), 16 * 1024);

    let tokens = parse_openai_stream_usage(&buf.bytes())
        .expect("terminal usage event must survive the bounded buffer");
    assert_eq!(tokens.input, Some(1234));
    assert_eq!(tokens.output, Some(567));
}
