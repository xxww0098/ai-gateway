//! OWNER: worker `relay-core`。
//!
//! 规范 2.11：期望值全部来自测试自己写的上游帧，或是「跨帧切法不影响结果」
//! 「缓冲不随流量线性增长」「缺失与零可分」这类不写死在源码里的性质。

use bytes::Bytes;

use super::{MAX_JSON, SseUsageProbe, UsageShape};
use crate::contract::{RelayUsage, UsageProbe};

/// 把 `text` 按 `chunk` 字节切成帧喂进去 —— 切法故意与行边界无关。
fn feed(shape: UsageShape, text: &str, chunk: usize) -> Option<RelayUsage> {
    let (probe, handle) = SseUsageProbe::new(shape);
    let mut probe = Box::new(probe);
    let bytes = Bytes::from(text.to_owned());
    let mut at = 0;
    while at < bytes.len() {
        let end = (at + chunk).min(bytes.len());
        probe.observe(&bytes.slice(at..end));
        at = end;
    }
    let usage = probe.finish();
    assert_eq!(handle.get(), Some(usage.clone()), "句柄与返回值必须一致");
    usage
}

/// 同一段字节，无论按几字节切帧，解析结果都必须相同。
fn stable_across_framings(shape: UsageShape, text: &str) -> RelayUsage {
    let baseline = feed(shape, text, text.len()).expect("整块喂必须能解析出 usage");
    for chunk in 1..=17 {
        assert_eq!(
            feed(shape, text, chunk).as_ref(),
            Some(&baseline),
            "按 {chunk} 字节切帧时结果变了"
        );
    }
    baseline
}

const OPENAI_STREAM: &str = concat!(
    ": keep-alive\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":31,\"completion_tokens\":17,",
    "\"prompt_tokens_details\":{\"cached_tokens\":8},",
    "\"completion_tokens_details\":{\"reasoning_tokens\":5}}}\n\n",
    "data: [DONE]\n\n",
);

const RESPONSES_STREAM: &str = concat!(
    "event: response.output_text.delta\n",
    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
    "event: response.completed\n",
    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",",
    "\"usage\":{\"input_tokens\":19,\"output_tokens\":7,",
    "\"input_tokens_details\":{\"cached_tokens\":6},",
    "\"output_tokens_details\":{\"reasoning_tokens\":2}}}}\n\n",
);

const ANTHROPIC_STREAM: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"usage\":",
    "{\"input_tokens\":25,\"cache_creation_input_tokens\":4,",
    "\"cache_read_input_tokens\":10}}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},",
    "\"usage\":{\"output_tokens\":42}}\n\n",
    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
);

const GOOGLE_STREAM: &str = concat!(
    "data: {\"candidates\":[{\"content\":{}}],\"usageMetadata\":",
    "{\"promptTokenCount\":10,\"candidatesTokenCount\":5}}\n\n",
    "data: {\"candidates\":[{\"content\":{}}],\"usageMetadata\":",
    "{\"promptTokenCount\":10,\"candidatesTokenCount\":20,",
    "\"thoughtsTokenCount\":7,\"cachedContentTokenCount\":3}}\n\n",
);

#[test]
fn the_openai_terminal_usage_frame_is_parsed() {
    let usage = stable_across_framings(UsageShape::OpenAi, OPENAI_STREAM);
    assert_eq!(
        usage,
        RelayUsage {
            input_tokens: Some(31),
            output_tokens: Some(17),
            cached_tokens: Some(8),
            reasoning_tokens: Some(5),
        }
    );
}

/// Responses API 的流式终局不是顶层 `usage`，而是
/// `response.completed.response.usage`。
#[test]
fn the_responses_terminal_event_usage_is_parsed() {
    let usage = stable_across_framings(UsageShape::OpenAi, RESPONSES_STREAM);
    assert_eq!(
        usage,
        RelayUsage {
            input_tokens: Some(19),
            output_tokens: Some(7),
            cached_tokens: Some(6),
            reasoning_tokens: Some(2),
        }
    );
}

/// Anthropic 的 `input_tokens` 是未缓存输入；网关内部的 input 列必须恢复成
/// 全输入，且只有 cache read 是 cached 子集。cache creation 留在普通输入里。
#[test]
fn the_anthropic_head_and_tail_frames_are_merged_without_double_subtracting_cache() {
    let usage = stable_across_framings(UsageShape::Anthropic, ANTHROPIC_STREAM);
    assert_eq!(
        usage,
        RelayUsage {
            input_tokens: Some(39),
            output_tokens: Some(42),
            cached_tokens: Some(10),
            reasoning_tokens: None,
        }
    );
}

#[test]
fn the_google_cumulative_usage_takes_the_last_frame() {
    let usage = stable_across_framings(UsageShape::Google, GOOGLE_STREAM);
    assert_eq!(
        usage,
        RelayUsage {
            input_tokens: Some(10),
            output_tokens: Some(20),
            cached_tokens: Some(3),
            reasoning_tokens: Some(7),
        }
    );
}

/// **「缺失」与「零」必须能分开** —— 计费的 fallback / strict 分支全挂在这上面。
#[test]
fn an_explicit_zero_is_not_the_same_as_a_missing_usage() {
    let absent = feed(
        UsageShape::OpenAi,
        "data: {\"choices\":[{\"delta\":{}}]}\n\ndata: [DONE]\n\n",
        7,
    );
    assert_eq!(absent, None, "上游没给 usage 必须是 None");

    let zeros = feed(
        UsageShape::OpenAi,
        "data: {\"usage\":{\"prompt_tokens\":0,\"completion_tokens\":0}}\n\n",
        7,
    )
    .expect("上游明确给了 0，不是缺失");
    assert_eq!(zeros.input_tokens, Some(0));
    assert_eq!(zeros.output_tokens, Some(0));
    assert_eq!(zeros.cached_tokens, None, "没给的列必须还是 None");
    assert!(!zeros.is_empty());
}

#[test]
fn a_complete_non_streaming_body_is_parsed_by_the_same_probe() {
    let body = concat!(
        "{\"id\":\"chatcmpl-1\",\"choices\":[{\"message\":{\"content\":\"hi\"}}],",
        "\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":22,",
        "\"prompt_tokens_details\":{\"cached_tokens\":3},",
        "\"completion_tokens_details\":{\"reasoning_tokens\":4}}}"
    );
    let (probe, _) = SseUsageProbe::new(UsageShape::OpenAi);
    let mut probe = Box::new(probe);
    probe.observe(&Bytes::from_static(body.as_bytes()));

    assert_eq!(
        probe.finish(),
        Some(RelayUsage {
            input_tokens: Some(11),
            output_tokens: Some(22),
            cached_tokens: Some(3),
            reasoning_tokens: Some(4),
        })
    );
}

#[test]
fn the_responses_api_usage_shape_is_recognised() {
    let body = concat!(
        "{\"usage\":{\"input_tokens\":9,\"output_tokens\":8,",
        "\"input_tokens_details\":{\"cached_tokens\":2},",
        "\"output_tokens_details\":{\"reasoning_tokens\":1}}}"
    );
    let (probe, _) = SseUsageProbe::new(UsageShape::OpenAi);
    let mut probe = Box::new(probe);
    probe.observe(&Bytes::from_static(body.as_bytes()));

    assert_eq!(
        probe.finish(),
        Some(RelayUsage {
            input_tokens: Some(9),
            output_tokens: Some(8),
            cached_tokens: Some(2),
            reasoning_tokens: Some(1),
        })
    );
}

/// 反向代理可能先吐一个空白数据帧。判型必须等到第一个非空白帧，不能因此把
/// 后续 JSON 当 SSE。
#[test]
fn a_whitespace_only_first_frame_does_not_hide_non_streaming_usage() {
    let (probe, handle) = SseUsageProbe::new(UsageShape::OpenAi);
    let mut probe = Box::new(probe);
    probe.observe(&Bytes::from_static(b"\r\n\t"));
    probe.observe(&Bytes::from_static(
        b"{\"usage\":{\"prompt_tokens\":13,\"completion_tokens\":5}}",
    ));
    let _ = probe.finish();

    let usage = handle.get().expect("已结束").expect("有 usage");
    assert_eq!(usage.input_tokens, Some(13));
    assert_eq!(usage.output_tokens, Some(5));
}

/// 非流式旁路累积的上限也必须覆盖**第一帧**。此前第一帧可以一次性复制任意
/// 大小，恶意上游只需单个巨帧就能绕过 8 MiB 闸门。
#[test]
fn an_oversized_first_json_frame_is_discarded_without_allocating_unbounded_state() {
    let (probe, handle) = SseUsageProbe::new(UsageShape::OpenAi);
    let mut probe = Box::new(probe);
    let mut huge = vec![b'x'; MAX_JSON + 1];
    huge[0] = b'{';
    probe.observe(&Bytes::from(huge));
    assert_eq!(probe.buffered_len(), 0);
    assert_eq!(probe.finish(), None);
    assert_eq!(handle.get(), Some(None));
}

#[test]
fn the_cross_frame_buffer_stays_at_line_scale() {
    let (mut probe, _) = SseUsageProbe::new(UsageShape::OpenAi);
    let line = Bytes::from(format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
        "x".repeat(400)
    ));
    let (head, tail) = (line.slice(..37), line.slice(37..));

    for _ in 0..2000 {
        probe.observe(&head);
        assert!(probe.buffered_len() <= line.len());
        probe.observe(&tail);
        assert_eq!(probe.buffered_len(), 0, "整行走完后不该还留着东西");
    }
}

#[test]
fn an_endless_line_cannot_grow_the_buffer_without_bound() {
    let (probe, _) = SseUsageProbe::new(UsageShape::OpenAi);
    let mut probe = Box::new(probe);
    let junk = Bytes::from(vec![b'x'; 64 * 1024]);
    let rounds = 128;

    for _ in 0..rounds {
        probe.observe(&junk);
    }
    let flowed = junk.len() * rounds;
    assert!(
        probe.buffered_len() * 2 < flowed,
        "缓冲随流量线性增长了：{} / {flowed}",
        probe.buffered_len()
    );

    probe.observe(&Bytes::from_static(
        b"\ndata: {\"usage\":{\"prompt_tokens\":7}}\n",
    ));
    assert_eq!(
        probe.finish().and_then(|u| u.input_tokens),
        Some(7),
        "越过超长行之后必须能继续解析"
    );
}

#[test]
fn only_the_anthropic_shape_asks_for_the_head_window() {
    for shape in [UsageShape::OpenAi, UsageShape::Google] {
        assert!(!SseUsageProbe::new(shape).0.needs_head(), "{shape:?}");
    }
    assert!(SseUsageProbe::new(UsageShape::Anthropic).0.needs_head());
}

#[test]
fn the_handle_separates_pending_from_absent() {
    let (probe, handle) = SseUsageProbe::new(UsageShape::OpenAi);
    assert_eq!(handle.get(), None, "流还没结束");
    assert_eq!(Box::new(probe).finish(), None);
    assert_eq!(handle.get(), Some(None), "结束了，但上游没给");
}

#[test]
fn non_data_sse_lines_are_ignored() {
    let noisy = concat!(
        ":\n",
        ": a comment mentioning usage\n",
        "id: 42\n",
        "retry: 3000\n",
        "event: usage\n",
        "\r\n",
        "data: {\"usage\":{\"prompt_tokens\":5}}\r\n",
        "\r\n",
    );
    assert_eq!(
        feed(UsageShape::OpenAi, noisy, 3).and_then(|u| u.input_tokens),
        Some(5)
    );
}

#[test]
fn a_non_streaming_json_body_survives_being_split_across_frames() {
    let whole = br#"{"id":"x","usage":{"prompt_tokens":11,"completion_tokens":22}}"#;

    for cut in 1..whole.len() {
        let (mut probe, handle) = SseUsageProbe::new(UsageShape::OpenAi);
        probe.observe(&Bytes::copy_from_slice(&whole[..cut]));
        probe.observe(&Bytes::copy_from_slice(&whole[cut..]));
        Box::new(probe).finish();

        let usage = handle
            .get()
            .expect("流已结束")
            .unwrap_or_else(|| panic!("切点 {cut} 处丢了 usage"));
        assert_eq!(usage.input_tokens, Some(11), "切点 {cut}");
        assert_eq!(usage.output_tokens, Some(22), "切点 {cut}");
    }
}
