//! 四级上游选择链 —— 取代今天的字符串前缀猜测。
//!
//! OWNER: worker `relay-endpoints`。
//!
//! # 根除的东西
//!
//! `routes::provider_candidates`（`routes.rs:73-97`）靠 `lower.contains("codex")` /
//! `starts_with("claude-")` / `starts_with("gpt-")` 猜上游。根因一句话：
//! **模型名是一个由上游厂商随时变更的字符串，而路由决策把它当成了一个稳定的枚举。**
//! 四个具体的静默失效场景见 `docs/relay-surface-plan.md` §3.5.1 ——
//! 它们全是静默的：不报错，只是打到错误的上游然后拿一个上游 4xx。
//!
//! # 四级链，第一个命中即止
//!
//! | 级 | 规则 | 数据源 |
//! | --- | --- | --- |
//! | **L1** | 显式渠道前缀 `<channel_key>/<model_id>` | 请求体的 `model` 字段本身 |
//! | **L2** | 模型目录查表 `model_id → channel_key[]` | `model_catalog_entries`，经缓存 |
//! | **L3** | `channel_key → provider` 映射 | `config.yaml` 的 `routing.channel_providers` |
//! | **L4** | 前缀猜测**降级为兜底**，命中必 `warn!` + 打点 | 硬编码前缀表 |
//!
//! L3 不是独立的一级 —— 它是 L1 与 L2 共用的**映射器**：两者拿到的都是
//! `channel_key`（管理员自填的自由文本），都必须经它落到五个 executor 之一。
//!
//! # 为什么 L4 必须逐字节等同于今天
//!
//! 目录为空（全新部署、或管理员从没点过「从上游获取模型列表」）时，
//! [`select`] 的结果必须与今天**逐字节相同**，这样才能安全灰度：
//! 灰度期出的任何问题都能确定不是路由改动引起的。
//! [`prefix_guess`] 是这条保证的承重点，`tests.rs` 里有一条对拍测试钉它。
//!
//! # 数据源为什么是注入的
//!
//! `gw-relay` 不依赖工作区任何其他 crate（依赖了就不叫内核了），所以 L2/L3 的
//! 数据源是一个注入的 [`ChannelResolver`]。真实实现（读 `model_catalog_entries`
//! 与 `config.yaml`，带缓存）由 wave 3 的 `gw-proxy` 提供；本模块给出 trait
//! 与一个内存实现 [`InMemoryChannelResolver`]。
//!
//! **`resolver: None` 就是一键回滚开关**（`docs/relay-surface-plan.md` §3.5.2 的
//! `catalog_routing_enabled: false`）：不传 resolver，L1/L2/L3 全部短路，直接落 L4。
//! 不需要第二个配置项。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::contract::Surface;
use crate::endpoint::matrix::Provider;

/// L2/L3 的数据源。真实实现在 wave 3 由 `gw-proxy` 提供（读
/// `model_catalog_entries` + `config.yaml`，走缓存）。
///
/// # 实现方必须遵守的两条
///
/// 1. **绝不复用 `list_models()` 的 SQL。** 那条查询带 `WHERE visible = TRUE`，
///    而 `visible` 是「对租户展示」开关，不是「允许调用」开关 —— 今天一个
///    `visible=false` 的模型照样能被调用。路由查询若继承了它，会静默地把所有
///    隐藏模型变成不可调用，表现为「某些模型突然 503」，极难归因。
/// 2. **不许阻塞。** 这两个方法在推理热路径上，实现必须查缓存。缓存未命中时
///    返回空（回落 L4），**不要等 DB** —— DB 抖动不该变成推理延迟。
pub trait ChannelResolver: Send + Sync {
    /// **L2**：模型 ID → 有序的 `channel_key` 候选。
    ///
    /// 返回多个是正常的（同一模型多渠道，例如 `gemini-2.5-pro` 同时可从
    /// gemini 与 vertex 出）—— 顺序即优先级，由数据源决定，不由这里硬编码。
    /// 这正是今天写死的 `["gemini", "vertex"]` 该被取代的地方。
    fn channels_for_model(&self, model_id: &str) -> Vec<String>;

    /// **L3**：`channel_key` → 上游 executor。
    ///
    /// `None` 表示这个 `channel_key` 没有显式映射。调用方据此落到
    /// [`WILDCARD_PROVIDER`]，因为 `openai` executor 就是那个通配的
    /// OpenAI 兼容 executor。
    fn provider_for_channel(&self, channel_key: &str) -> Option<Provider>;
}

/// 没有显式映射的 `channel_key` 落到哪里。
///
/// `openai` executor 是那个**通配的** OpenAI 兼容 executor
/// （DeepSeek / Qwen / Kimi / 自建 vLLM 都从它出网），所以「不认识的渠道」
/// 落它是唯一有意义的选择。注意这不是猜 —— L1/L2 已经确定了渠道，
/// 只是这个渠道没有被显式映射到某个专用 executor。
pub const WILDCARD_PROVIDER: Provider = Provider::OpenAi;

/// 一个可注入的内存实现。测试用，也是 wave 3 真实实现的行为参照。
#[derive(Debug, Default, Clone)]
pub struct InMemoryChannelResolver {
    models: HashMap<String, Vec<String>>,
    channels: HashMap<String, Provider>,
}

impl InMemoryChannelResolver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一条 `model_id → channel_key[]`（对应 `model_catalog_entries` 的若干行）。
    #[must_use]
    pub fn with_model(
        mut self,
        model_id: impl Into<String>,
        channel_keys: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.models.insert(
            model_id.into(),
            channel_keys.into_iter().map(Into::into).collect(),
        );
        self
    }

    /// 登记一条 `channel_key → provider`（对应 `config.yaml` 的
    /// `routing.channel_providers` 一行）。
    #[must_use]
    pub fn with_channel(mut self, channel_key: impl Into<String>, provider: Provider) -> Self {
        self.channels.insert(channel_key.into(), provider);
        self
    }
}

impl ChannelResolver for InMemoryChannelResolver {
    fn channels_for_model(&self, model_id: &str) -> Vec<String> {
        self.models.get(model_id).cloned().unwrap_or_default()
    }

    fn provider_for_channel(&self, channel_key: &str) -> Option<Provider> {
        self.channels.get(channel_key).copied()
    }
}

/// 哪一级给出了答案。进日志与指标，用来回答「灰度期到底有多少流量还在靠猜」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionLevel {
    /// L1：客户端在 model 里显式指定了渠道。
    ExplicitPrefix,
    /// L2（+L3）：模型目录说了算。
    Catalog,
    /// L4：前缀猜测兜底。**每一次命中都是一条 `warn!` 和一次计数**。
    PrefixGuess,
}

/// 选择结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// 候选上游，按优先级排序，已去重。
    ///
    /// 空表示「无法确定上游」—— 调用方回一个明确的错误，**不要**随便挑一个试。
    /// 实践中只有 L4 在 surface 默认 provider 也拿不到时才会为空。
    pub candidates: Vec<Provider>,
    /// 答案来自哪一级。
    pub level: SelectionLevel,
    /// L1 剥掉渠道前缀之后的模型名。`None` = 模型名未被改写。
    ///
    /// **本层不改 body。** 客户端写的 `codex/gpt-5` 里，`codex/` 是给网关看的，
    /// 上游只认识 `gpt-5` —— 但改写 body 的 `model` 字段是一次 body 变异，
    /// 而本 crate 的合同是「唯一被授权的 body 变异是 `stream_options` 定点注入」。
    /// 所以这里只把「去前缀后的名字」算出来交给上层，**由 wave 3 决定**是改写
    /// body 还是要求管理员配置成不带前缀的渠道。这个张力是真实的，不要静默解决它。
    pub upstream_model: Option<String>,
}

/// L4 兜底命中的累计次数。**打点**（`docs/relay-surface-plan.md` §3.5.2 L4）。
///
/// 灰度期看这个数：它降到 0 意味着目录已经覆盖了全部真实流量，
/// 前缀猜测可以真正删掉；它一直不降，就说明有一批模型从没被写进目录 ——
/// 这是把「要不要删前缀表」从一次赌博变成一次测量的唯一办法。
static PREFIX_GUESS_HITS: AtomicU64 = AtomicU64::new(0);

/// 读取 [`SelectionLevel::PrefixGuess`] 的累计命中次数。
#[must_use]
pub fn prefix_guess_hits() -> u64 {
    PREFIX_GUESS_HITS.load(Ordering::Relaxed)
}

/// 四级链，第一个命中即止。
///
/// `model` 为 `None`（请求体是流式的、或者客户端没写 `model`）时直接落 L4，
/// 得到的就是 surface 的默认 provider —— 与今天 `provider_candidates("")` 的
/// 结果逐字节相同。
///
/// `resolver` 为 `None` 时 L1/L2/L3 全部短路：这是一键回滚开关，
/// 打开后行为与今天完全一致。
#[must_use]
pub fn select(
    surface: Surface,
    model: Option<&str>,
    resolver: Option<&dyn ChannelResolver>,
) -> Selection {
    let model = model.unwrap_or_default();

    if let Some(resolver) = resolver {
        // ---- L1：显式渠道前缀 `<channel_key>/<model_id>`
        //
        // **只在前缀是一个已知 channel_key 时才生效。** 这条限制不是保守，是必需：
        // `meta-llama/Llama-3-70b`、`anthropic/claude-3.5-sonnet` 这类带斜杠的模型名
        // 在 OpenAI 兼容上游（vLLM / OpenRouter 风格）里是**模型名的一部分**。
        // 把任何斜杠都当渠道前缀剥掉，会静默改掉客户端要的模型。
        if let Some((prefix, rest)) = model.split_once('/')
            && !rest.is_empty()
            && let Some(provider) = resolver.provider_for_channel(prefix)
        {
            return Selection {
                candidates: vec![provider],
                level: SelectionLevel::ExplicitPrefix,
                upstream_model: Some(rest.to_owned()),
            };
        }

        // ---- L2（+L3）：模型目录查表
        let channels = resolver.channels_for_model(model);
        if !channels.is_empty() {
            let mut candidates = Vec::with_capacity(channels.len());
            for key in &channels {
                let provider = resolver
                    .provider_for_channel(key)
                    .unwrap_or(WILDCARD_PROVIDER);
                if !candidates.contains(&provider) {
                    candidates.push(provider);
                }
            }
            return Selection {
                candidates,
                level: SelectionLevel::Catalog,
                upstream_model: None,
            };
        }
    }

    // ---- L4：前缀猜测兜底
    PREFIX_GUESS_HITS.fetch_add(1, Ordering::Relaxed);
    let candidates = prefix_guess(surface, model);
    tracing::warn!(
        model,
        surface = ?surface,
        candidates = ?candidates.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
        "上游选择落到 L4 前缀猜测：模型不在目录里，路由结果只是猜的"
    );
    Selection {
        candidates,
        level: SelectionLevel::PrefixGuess,
        upstream_model: None,
    }
}

/// 入口的默认上游 —— L4 兜底**内部**的最后一环。
///
/// 语义不是「这个端点默认走谁」，而是「前缀表和目录都没话说时，
/// 按客户端说的方言猜一个」。收敛后只剩两种方言，所以只剩两个默认值
/// （今天的 `ApiFamily::{Embeddings, Gemini}` 随被删的路由一起消失）。
#[must_use]
pub fn default_provider(surface: Surface) -> Provider {
    match surface {
        Surface::OpenAiCompletions | Surface::OpenAiResponses => Provider::OpenAi,
        Surface::AnthropicMessages => Provider::Claude,
    }
}

/// L4：今天的 `provider_candidates()`（`routes.rs:73-97`）**原样搬过来**。
///
/// 逐条对齐今天的行为，包括那条 `text-embedding` 分支 —— `/v1/embeddings`
/// 入口虽然被删了，但模型名这条轴上的判定必须一字不改，否则「目录为空时行为
/// 与今天逐字节相同」这条灰度保证就破了。
///
/// 它**不是**一个好的路由规则（它就是被 L1/L2/L3 取代的那个东西），
/// 保留它只是为了让灰度可回滚。
#[must_use]
pub fn prefix_guess(surface: Surface, model: &str) -> Vec<Provider> {
    let lower = model.to_ascii_lowercase();
    let mut candidates: Vec<Provider> = if lower.contains("codex") {
        vec![Provider::Codex, Provider::OpenAi]
    } else if lower.starts_with("claude-") {
        vec![Provider::Claude]
    } else if lower.starts_with("gemini-") {
        vec![Provider::Gemini, Provider::Vertex]
    } else if lower.starts_with("gpt-")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.starts_with("text-embedding")
    {
        vec![Provider::OpenAi]
    } else {
        Vec::new()
    };
    let default = default_provider(surface);
    if !candidates.contains(&default) {
        candidates.push(default);
    }
    candidates
}

#[cfg(test)]
mod tests;
