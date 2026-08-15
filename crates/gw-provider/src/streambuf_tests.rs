//! Unit tests for [`crate::streambuf`].
//!
//! 每条都在测**不写死在源码里的性质**（规范 2.11）：跨帧续行、驻留内存与流量脱钩、
//! 拆分策略与结果无关、以及 Anthropic 那种「tally 分两半」的合并方向。
//! 断言里没有一个来自实现的常量。

use super::*;
use crate::claude::parse_claude_stream_usage;
use crate::usage::parse_openai_stream_usage;

/// 把 `s` 按 `chunk` 大小切碎喂进去，模拟 executor 的读循环。
fn feed(probe: &mut StreamUsageProbe, s: &[u8], chunk: usize) {
    for part in s.chunks(chunk) {
        probe.observe(part);
    }
}

/// 一条带终局 usage 事件的 OpenAI SSE 流。`filler` 决定它有多长。
fn openai_stream(filler: usize, prompt: i64, completion: i64) -> String {
    let mut body = String::from("data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n");
    for _ in 0..filler {
        body.push_str("data: {\"choices\":[{\"delta\":{\"content\":\"some streamed text\"}}]}\n\n");
    }
    body.push_str(&format!(
        "data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":{prompt},\"completion_tokens\":{completion}}}}}\n\ndata: [DONE]\n\n"
    ));
    body
}

/// **切法不该影响结果。** 同一条流按任意 chunk 大小喂进去都要得到同一个 tally ——
/// 这正是「跨帧半行」那半个不变量，逐字节喂是最狠的一档。
#[test]
fn the_tally_is_independent_of_how_the_stream_is_chopped() {
    let body = openai_stream(3, 1234, 567);
    let mut tallies = Vec::new();
    for chunk in [1, 3, 17, 256, body.len()] {
        let mut probe = StreamUsageProbe::new(parse_openai_stream_usage);
        feed(&mut probe, body.as_bytes(), chunk);
        tallies.push(probe.finish().expect("usage must survive any chunking"));
    }
    let first = tallies[0];
    assert!(
        tallies.iter().all(|t| *t == first),
        "chunking changed the tally: {tallies:?}"
    );
    assert_eq!(first.total(), Some(1234 + 567));
}

/// 计费回归守护：终局 usage 事件必须在**任意长度**的流里活下来。
///
/// 老实现靠 head+tail 窗口，长流会把中间挤掉；这里测的是「长度不影响结果」——
/// 短流与长流的 tally 必须相等，而不是去核对某个窗口尺寸。
#[test]
fn the_terminal_usage_event_survives_however_long_the_stream_is() {
    let short = openai_stream(1, 4321, 765);
    let long = openai_stream(20_000, 4321, 765);
    assert!(
        long.len() > 100 * short.len(),
        "fixture must actually be long enough to matter"
    );

    let mut probe = StreamUsageProbe::new(parse_openai_stream_usage);
    feed(&mut probe, long.as_bytes(), 16 * 1024);
    let from_long = probe.finish().expect("usage lost in a long stream");

    let mut probe = StreamUsageProbe::new(parse_openai_stream_usage);
    feed(&mut probe, short.as_bytes(), 16 * 1024);
    let from_short = probe.finish().expect("usage lost in a short stream");

    assert_eq!(from_long, from_short);
}

/// **热点 #2 的判据**：驻留内存与流量脱钩。
///
/// 老实现每 chunk 全量复制进 tail，驻留会一路涨到 head+tail 并常驻；
/// 增量解析下驻留只在「一行」的量级上。这里不核对任何常量，只断言
/// 「流量翻了三个数量级，驻留没有跟着涨」。
#[test]
fn retained_memory_does_not_grow_with_the_stream() {
    let short = openai_stream(1, 1, 1);
    let long = openai_stream(20_000, 1, 1);

    let mut small = StreamUsageProbe::new(parse_openai_stream_usage);
    feed(&mut small, short.as_bytes(), 4096);
    let mut big = StreamUsageProbe::new(parse_openai_stream_usage);
    feed(&mut big, long.as_bytes(), 4096);

    assert!(
        big.total() > 100 * small.total(),
        "precondition: the long stream must dwarf the short one"
    );
    assert!(
        big.buffered_len() <= small.buffered_len().max(longest_line(&long)),
        "retained {} bytes for {} streamed — memory tracked the traffic",
        big.buffered_len(),
        big.total()
    );
}

fn longest_line(body: &str) -> usize {
    body.split('\n').map(str::len).max().unwrap_or_default()
}

/// 终局帧被读边界切成两半 —— 老的「按 chunk 解析」看不见它，
/// Vertex 为此写过一个专用累加器。跨帧续行让它变成普通情况。
#[test]
fn a_frame_split_across_two_reads_is_still_counted() {
    let whole =
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":900}}\n";
    let cut = whole.len() / 2;

    let mut probe = StreamUsageProbe::new(parse_openai_stream_usage);
    probe.observe(&whole.as_bytes()[..cut]);
    // 半个帧单独看不是合法 JSON，任何按 chunk 的解析都拿不到它。
    assert!(
        parse_openai_stream_usage(&whole.as_bytes()[..cut]).is_none(),
        "precondition: neither half parses on its own"
    );
    probe.observe(&whole.as_bytes()[cut..]);

    let tokens = probe.finish().expect("a split frame must still be counted");
    assert_eq!(tokens.total(), Some(1000));
}

/// Anthropic 把 tally 拆成两半：`message_start` 带 input、末尾的 `message_delta`
/// 带 output。「后者胜」会把 input 抹掉，所以合并方向必须是按列取最大值。
#[test]
fn anthropic_input_and_output_arrive_in_different_frames_and_both_survive() {
    let body = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":77}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"usage\":{\"output_tokens\":5}}}\n\n",
    );
    let mut probe = StreamUsageProbe::new(parse_claude_stream_usage);
    feed(&mut probe, body.as_bytes(), 13);
    let tokens = probe.finish().expect("usage");
    assert_eq!(tokens.input, Some(77), "message_start 的 input 被抹掉了");
    assert_eq!(tokens.output, Some(5), "message_delta 的 output 丢了");
}

/// 「缺失」与「零」是两件事：没有任何 usage 帧时必须是 `None`，
/// 而不是一个全零的 tally —— `billing.strict_usage_metadata_mode` 靠这个判据。
#[test]
fn a_stream_without_any_usage_frame_reports_absent_not_zero() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\ndata: [DONE]\n\n";
    let mut probe = StreamUsageProbe::new(parse_openai_stream_usage);
    feed(&mut probe, body.as_bytes(), 7);
    assert!(probe.finish().is_none());
}

/// 上游用非流式信封回应一个流式请求：整个 body 是一行紧凑 JSON，
/// 而且没有换行收尾 —— 只有 `finish()` 的冲洗能救它。
#[test]
fn a_whole_json_body_with_no_trailing_newline_is_still_parsed() {
    let body = br#"{"usage":{"prompt_tokens":11,"completion_tokens":22}}"#;
    let mut probe = StreamUsageProbe::new(parse_openai_stream_usage);
    feed(&mut probe, body, 5);
    assert_eq!(probe.finish().and_then(|t| t.total()), Some(33));
}

/// 信任边界外的输入：一个永不发换行的上游不能把跨帧缓冲撑爆。
/// 超长行被丢弃到下一个换行为止，其后的正常帧照常计入。
#[test]
fn an_upstream_that_never_sends_a_newline_cannot_grow_the_buffer_without_bound() {
    let mut probe = StreamUsageProbe::new(parse_openai_stream_usage);
    let flood = vec![b'x'; 64 * 1024];
    for _ in 0..64 {
        probe.observe(&flood);
    }
    let streamed = probe.total();
    assert!(
        (probe.buffered_len() as u64) < streamed / 4,
        "buffered {} of {streamed} streamed bytes",
        probe.buffered_len()
    );

    // 闸门关上之后仍然要能恢复：下一个换行结束丢弃，再下一帧正常计入。
    probe.observe(
        b"\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4}}\n",
    );
    assert_eq!(probe.finish().and_then(|t| t.total()), Some(7));
}
