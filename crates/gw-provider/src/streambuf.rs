//! 流式 usage 的**增量行探测器**。
//!
//! OWNER: worker `provider-bytes`（wave 3）。
//!
//! # 根除的缺陷：热点 #2（`docs/relay-perf-baseline.md` §3 第 2 条）
//!
//! 这里原来是 `StreamUsageBuffer`：一个 head 32 KiB + tail 64 KiB 的字节窗口。
//! 它对**每一个 chunk 全量 memcpy 进 tail**（`extend_from_slice`），tail 涨到
//! 128 KiB 还要再 memmove 掉 64 KiB。实测代价：
//!
//! | 口径 | 下界 | 当时 |
//! | --- | ---: | ---: |
//! | SSE 分配字节 ÷ 流量字节 | 0.107× | **0.937×** |
//! | 每 chunk 额外开销 | — | **+0.585 µs** |
//!
//! 「0.937×」的意思是**每一个流字节都被完整复制了一遍**，500 chunk 的流为此多花
//! 约 200 µs，并常驻 96 KiB × 并发流数。
//!
//! # 现在怎么做
//!
//! SSE 是**行式**的：usage 永远落在某一行 `data: {…}` 里，没有任何一行需要它前面
//! 那 32 KiB 的上下文。所以不再留窗口，改成**逐行增量解析**：
//!
//! - 整行落在同一帧里 → **零拷贝**，直接把切片借给解析器；
//! - 只有跨帧的那半行会被复制一次，稳态额外内存 = `O(单行)`，**与流量无关**；
//! - 解析结果按列取最大值（[`crate::usage::max_usage_tokens`]）滚动合并。
//!
//! 思路来自已交付的 `gw_relay::probe::SseUsageProbe`（只读参考）。这里没有复用它
//! 的类型，因为本 crate 的四个 provider 解析器返回 [`UsageTokens`]，而 gw-relay 的
//! 返回 `RelayUsage`，两边的「缺失 vs 零」判据（[`UsageTokens::has_values`] 要求
//! **非零**）也不同 —— 换过去等于顺手改了计费语义。
//!
//! # 顺带解决的正确性问题
//!
//! 老窗口有两个只能靠外部代码兜的洞，增量解析下**结构上不存在**：
//!
//! 1. **终局帧被读边界切成两半**时，没有任何一次「按 chunk」解析看得见它。
//!    `vertex.rs` 为此专门写了 `VertexUsageTally`（per-chunk latch + 收尾时对整个
//!    窗口再解析一遍 + 列最大值）—— 那 60 行现在可以删掉，跨帧半行由 `pending` 天然接上。
//! 2. **中间被丢弃**时窗口用一个 `\n` 把 head 与 tail 隔开，靠「拼不出可解析的行」
//!    来避免误判。现在没有中间，也就没有这个隐患。
//!
//! # 已知且有意的收窄
//!
//! 上游若用**多行 pretty-print 的单个 JSON 对象**回应一个流式请求，且它跨了帧，
//! 那么逐行解析看不到它 —— usage 缺失，计费落 fallback（`AGENTS.md` §计费流程）。
//! **转发不受影响**：每一个字节仍然原样转出去（计费降级，转发不降级）。
//! 真实上游（OpenAI / Anthropic / Google）在流式方向一律是行式 SSE 或
//! 单行 JSON chunk，紧凑单行 body 由 [`StreamUsageProbe::finish`] 的收尾冲洗覆盖。

use crate::usage::{UsageTokens, max_usage_tokens};

/// 单行上限：超过就丢弃这一行直到下一个 `\n`。
///
/// 上游字节是信任边界外的输入。没有这个闸门，一个永不发 `\n` 的上游就能把
/// `pending` 撑到 OOM —— 那正是老窗口用固定 tail 尺寸换来的性质，
/// 增量解析必须自己把它挣回来。usage 所在的 `data:` 行实测都在几百字节量级。
const MAX_LINE: usize = 256 * 1024;

/// 判「这一行有没有可能带 usage」的字节标记。
///
/// 四个解析器要的顶层键分别是 `usage`（OpenAI / Codex / Claude）与
/// `usageMetadata`（Gemini / Vertex），都含这五个字节。这是一道**只会放过、
/// 不会误杀**的前置闸门：没有这个子串的行，解析器必然返回 `None`。
///
/// 它买到的是每 chunk 一次约 200 字节的子串扫描（~50 ns），换掉每 chunk 一次
/// `serde_json` 词法分析（一个 200 字节的 delta 帧就要几百 ns）——
/// 验收目标 T8（每 chunk ≤ 0.15 µs）没有这道闸门够不到。
///
/// 唯一的漏网之鱼是把键名写成 JSON 转义（`"usage"`）的上游：合法但没有真实
/// 上游这么干，代价也只是 usage 缺失并落 fallback 计费，不影响转发。
const USAGE_MARKER: &[u8] = b"usage";

/// 一条流的增量 usage 探测器。
///
/// 用法是三段式：[`Self::new`] 给定 provider 的帧解析器 → 每收到一帧调
/// [`Self::observe`] → 流结束时 [`Self::finish`] 恰好一次。
///
/// `parse` 收到的是**一行**（含 `data:` 前缀，原样），不是整个 body。四个
/// provider 的既有扫描器（`parse_openai_stream_usage` / `parse_claude_stream_usage`
/// / `parse_gemini_stream_usage` / `extract_latest_vertex_usage`）在单行输入上
/// 与在整 body 输入上语义一致，所以它们一行都不用改。
#[derive(Debug, Clone)]
pub struct StreamUsageProbe {
    parse: fn(&[u8]) -> Option<UsageTokens>,
    /// 跨帧的半行。稳态长度 = `O(单行)`，**不随流量增长** —— 这是本次改动的可观测量。
    pending: Vec<u8>,
    /// 当前这行已超 [`MAX_LINE`]，丢弃到下一个 `\n` 为止。
    discarding: bool,
    latest: UsageTokens,
    /// 有没有解析成功过**任何**一帧。`false` → [`Self::finish`] 返回 `None`。
    ///
    /// 「缺失」与「零」必须分得开：`None` 是上游没给（计费走 fallback / strict），
    /// `Some(全零)` 走不到这里 —— [`UsageTokens::has_values`] 已经把它挡在解析器内。
    found: bool,
    total: u64,
}

impl StreamUsageProbe {
    /// 绑定一个 provider 的帧解析器。
    #[must_use]
    pub fn new(parse: fn(&[u8]) -> Option<UsageTokens>) -> Self {
        Self {
            parse,
            pending: Vec::new(),
            discarding: false,
            latest: UsageTokens::default(),
            found: false,
            total: 0,
        }
    }

    /// 流过的总字节数（不是驻留字节数）。
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// 当前跨帧缓冲的字节数。
    ///
    /// 热点 #2 的**可观测量**：不管流过多少字节，它只在一行的量级上。
    /// 老窗口这个数会一路涨到 96 KiB 并常驻。
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.pending.len()
    }

    /// 吃进一帧上游字节。整行零拷贝，只有跨帧的半行被复制一次。
    pub fn observe(&mut self, frame: &[u8]) {
        self.total += frame.len() as u64;
        let mut rest = frame;
        while let Some(idx) = rest.iter().position(|&b| b == b'\n') {
            let (line, tail) = rest.split_at(idx);
            rest = &tail[1..];
            self.end_line(line);
        }
        self.hold(rest);
    }

    /// 流结束。冲洗最后一段没有换行收尾的字节 —— 紧凑单行 JSON body
    /// （上游用非流式信封回应流式请求）走的就是这条路。
    ///
    /// 幂等：`pending` 冲洗后即被清空，重复调用返回同一个值。
    #[must_use]
    pub fn finish(&mut self) -> Option<UsageTokens> {
        if !self.discarding && !self.pending.is_empty() {
            let buf = std::mem::take(&mut self.pending);
            self.take_line(&buf);
        }
        self.found.then_some(self.latest)
    }

    /// 一行走完了（遇到 `\n`）。
    fn end_line(&mut self, line: &[u8]) {
        if self.discarding {
            self.discarding = false;
            self.pending.clear();
            return;
        }
        if self.pending.is_empty() {
            // 整行都在这一帧里 —— 零拷贝，直接借切片。
            self.take_line(line);
            return;
        }
        self.pending.extend_from_slice(line);
        let buf = std::mem::take(&mut self.pending);
        self.take_line(&buf);
        // 把容量还回去：下一次跨帧不用重新分配。
        self.pending = buf;
        self.pending.clear();
    }

    /// 帧尾的半行，留到下一帧接上。
    fn hold(&mut self, tail: &[u8]) {
        if tail.is_empty() || self.discarding {
            return;
        }
        if self.pending.len().saturating_add(tail.len()) > MAX_LINE {
            self.pending.clear();
            self.discarding = true;
            return;
        }
        self.pending.extend_from_slice(tail);
    }

    /// 把一整行交给 provider 解析器，命中就按列取最大值合并。
    ///
    /// **为什么是最大值而不是「后者胜」**：Anthropic 把 tally 拆成两半 ——
    /// `message_start` 带 `input_tokens`、末尾的 `message_delta` 带
    /// `output_tokens`，后者胜会把 input 抹掉。而 Google 的 `usageMetadata`
    /// 是累计值、OpenAI 只在末帧报一次 usage，两者「最大值」与「后者胜」等价。
    /// 一条规则覆盖四个 provider，且方向是**不会少收**。
    fn take_line(&mut self, line: &[u8]) {
        if !carries_usage(line) {
            return;
        }
        let Some(tokens) = (self.parse)(line) else {
            return;
        };
        self.latest = max_usage_tokens(self.latest, tokens);
        self.found = true;
    }
}

/// 见 [`USAGE_MARKER`]。
fn carries_usage(line: &[u8]) -> bool {
    line.len() >= USAGE_MARKER.len()
        && line
            .windows(USAGE_MARKER.len())
            .any(|window| window == USAGE_MARKER)
}

#[cfg(test)]
#[path = "streambuf_tests.rs"]
mod tests;
