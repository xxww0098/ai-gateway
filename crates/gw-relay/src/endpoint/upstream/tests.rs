//! OWNER: worker `relay-endpoints`。
//!
//! 规范 2.11：这里**不抄**前缀表里的任何一条规则，也不抄 `channel_providers`
//! 的默认映射。测的是四级链必须满足的性质：目录为空时与今天等价、目录说话时前缀
//! 闭嘴、带斜杠的模型名不被误剥、候选列表保序去重、每次落到猜测都被记账。
//!
//! 注意 [`prefix_guess_hits`] 是**进程级**计数器，`cargo test` 多线程并发跑，
//! 所以断言只用「单调增」这类在并发下仍然成立的形式，绝不断言绝对值或精确增量。

use super::*;

/// 一批真实会出现的模型名，覆盖前缀表的每一支以及它一支都不命中的情形。
/// 它们是**输入**，不是对实现的复述 —— 断言里没有任何一条依赖某个具体名字
/// 该落到哪个 provider。
const MODELS: [&str; 12] = [
    "gpt-5",
    "gpt-4o-mini",
    "o3-mini",
    "o4",
    "claude-opus-4.5",
    "gemini-2.5-pro",
    "gpt-5-codex",
    "text-embedding-3-small",
    "deepseek-chat",
    "qwen-max",
    "",
    "Claude-Opus-4.5",
];

fn all_surfaces() -> [Surface; 3] {
    [
        Surface::OpenAiCompletions,
        Surface::OpenAiResponses,
        Surface::AnthropicMessages,
    ]
}

/// **灰度保证**：目录为空时，四级链的结果与纯前缀猜测逐条相同。
///
/// 这是整套方案能安全上线的承重点。全新部署、或者管理员从没点过
/// 「从上游获取模型列表」时，路由行为必须与今天一模一样 ——
/// 灰度期出的任何问题都能确定不是路由改动引起的。
///
/// 守护的 bug：在 L1/L2 短路的分支里顺手「优化」候选顺序，或者在没有目录时
/// 也去掉某个候选。任何一处偏差都会让灰度期的对照实验失去意义。
#[test]
fn an_empty_catalog_routes_exactly_like_the_prefix_table_does_today() {
    let empty = InMemoryChannelResolver::new();
    for surface in all_surfaces() {
        for model in MODELS {
            let expected = prefix_guess(surface, model);
            for resolver in [None, Some(&empty as &dyn ChannelResolver)] {
                let got = select(surface, Some(model), resolver);
                assert_eq!(
                    got.candidates, expected,
                    "{surface:?} / {model:?} 的候选与前缀表不一致"
                );
                assert_eq!(got.level, SelectionLevel::PrefixGuess);
                assert!(got.upstream_model.is_none(), "兜底路径不该改写模型名");
            }
        }
    }
}

/// 前缀表永远给得出至少一个候选。
///
/// 守护的 bug：删掉「把入口默认 provider 补进候选」那一步。今天任何模型名
/// 都至少有端点默认值兜着；删掉之后，一个新命名的模型会拿到空候选，
/// 直接变成 `UnknownModel` 错误 —— 一批今天能用的模型突然全部不可用。
#[test]
fn guessing_never_gives_up_entirely() {
    for surface in all_surfaces() {
        for model in MODELS {
            assert!(
                !prefix_guess(surface, model).is_empty(),
                "{surface:?} / {model:?} 兜底候选为空"
            );
        }
        assert!(
            prefix_guess(surface, "").contains(&default_provider(surface)),
            "空模型名时必须落到入口默认上游"
        );
    }
}

/// 目录说话时，前缀表闭嘴。
///
/// 守护的 bug：把 L2 的结果与 L4 的结果合并（「多一个候选总没坏处」）。
/// 那样一个名字里带 `claude-` 但实际配在某个 OpenAI 兼容渠道上的模型，
/// 仍然会先去打 Anthropic 账号池 —— 四级链就白做了，
/// 「新模型名静默走错上游」这个根因原样保留。
#[test]
fn the_catalogue_outranks_the_name_it_happens_to_have() {
    let resolver = InMemoryChannelResolver::new()
        .with_model("claude-opus-4.5", ["my-vertex-channel"])
        .with_channel("my-vertex-channel", Provider::Vertex);

    let got = select(
        Surface::AnthropicMessages,
        Some("claude-opus-4.5"),
        Some(&resolver),
    );
    assert_eq!(got.level, SelectionLevel::Catalog);
    assert_eq!(got.candidates, vec![Provider::Vertex]);
    assert!(
        !got.candidates
            .contains(&default_provider(Surface::AnthropicMessages)),
        "目录已经给出答案，端点默认值不该再挤进来"
    );
}

/// 显式渠道前缀命中时，上游拿到的是**去掉前缀**的模型名。
///
/// 守护的 bug：命中 L1 却把整个 `channel/model` 当模型名传下去。
/// 上游收到一个它不认识的模型名，返回 404 / 400，而客户端明明写对了。
#[test]
fn an_explicit_channel_prefix_selects_the_channel_and_hands_back_the_bare_model() {
    let resolver = InMemoryChannelResolver::new().with_channel("codexchan", Provider::Codex);
    let got = select(
        Surface::OpenAiCompletions,
        Some("codexchan/gpt-5-pro"),
        Some(&resolver),
    );
    assert_eq!(got.level, SelectionLevel::ExplicitPrefix);
    assert_eq!(got.candidates, vec![Provider::Codex]);
    assert_eq!(got.upstream_model.as_deref(), Some("gpt-5-pro"));
}

/// 斜杠不等于渠道前缀：不认识的前缀属于**模型名本身**。
///
/// 这是四级链里最容易写错、也最贵的一条。`meta-llama/Llama-3-70b`、
/// `anthropic/claude-3.5-sonnet` 这类名字在 OpenAI 兼容上游（vLLM / OpenRouter 风格）
/// 里是完整的模型 ID。
///
/// 守护的 bug：把任何斜杠都当渠道前缀剥掉。上游会收到 `Llama-3-70b`
/// 而客户端要的是 `meta-llama/Llama-3-70b` —— 一个静默的模型替换，
/// 计费按一个模型、推理用另一个模型。
#[test]
fn a_slash_that_is_not_a_known_channel_belongs_to_the_model_name() {
    let resolver = InMemoryChannelResolver::new()
        .with_channel("codexchan", Provider::Codex)
        .with_model("meta-llama/Llama-3-70b", ["vllm-box"]);

    let got = select(
        Surface::OpenAiCompletions,
        Some("meta-llama/Llama-3-70b"),
        Some(&resolver),
    );
    assert_ne!(
        got.level,
        SelectionLevel::ExplicitPrefix,
        "不认识的前缀被当成渠道剥掉了"
    );
    assert!(got.upstream_model.is_none(), "模型名被静默改写了");

    // 前缀里恰好写了一个已知渠道名、但后半段是空的，也不算显式前缀。
    let empty_tail = select(
        Surface::OpenAiCompletions,
        Some("codexchan/"),
        Some(&resolver),
    );
    assert_ne!(empty_tail.level, SelectionLevel::ExplicitPrefix);
}

/// 同一模型多渠道时，候选列表**保序**且**去重**。
///
/// 顺序即优先级，由数据源决定 —— 这正是今天写死的 `["gemini","vertex"]`
/// 该被取代的地方：管理员无法通过面板调整跨 provider 的优先级。
///
/// 守护的 bug：用 `HashSet` 去重。顺序一丢，管理员在面板上调的优先级就失效了，
/// 而且每次进程重启的候选顺序还可能不同 —— 一个不可复现的故障。
#[test]
fn multiple_channels_become_an_ordered_deduplicated_candidate_list() {
    let resolver = InMemoryChannelResolver::new()
        .with_model("shared", ["b-chan", "a-chan", "b-again", "c-chan"])
        .with_channel("a-chan", Provider::Gemini)
        .with_channel("b-chan", Provider::Vertex)
        .with_channel("b-again", Provider::Vertex)
        .with_channel("c-chan", Provider::Claude);

    let got = select(Surface::OpenAiCompletions, Some("shared"), Some(&resolver));
    assert_eq!(
        got.candidates,
        vec![Provider::Vertex, Provider::Gemini, Provider::Claude],
        "候选顺序或去重不对"
    );
}

/// 目录命中但渠道没有显式映射时，仍然由目录负责，**不回落到猜测**。
///
/// 守护的 bug：把「L3 查不到」当成「L2 没命中」再往下走 L4。
/// 那样一个管理员刚在面板上新建、还没写进 `channel_providers` 的渠道，
/// 会静默退回按模型名猜 —— 而管理员刚刚做的那次配置完全没生效。
#[test]
fn a_channel_without_an_explicit_mapping_is_still_the_catalogues_answer() {
    let resolver = InMemoryChannelResolver::new().with_model("house-model", ["brand-new-channel"]);
    let got = select(
        Surface::AnthropicMessages,
        Some("house-model"),
        Some(&resolver),
    );
    assert_eq!(
        got.level,
        SelectionLevel::Catalog,
        "目录已经命中，不该退回前缀猜测"
    );
    assert_eq!(got.candidates.len(), 1, "没有映射也必须落到一个确定的上游");
}

/// 每一次落到前缀猜测都被记账。
///
/// 打点是把「要不要删掉前缀表」从一次赌博变成一次测量的唯一办法：
/// 灰度期这个数降到 0，前缀表才能真正删掉。
///
/// 守护的 bug：只打 `warn!` 不计数。日志会被采样、被丢、被限流，
/// 而「还有多少流量在靠猜」这个问题需要一个不会丢的数。
#[test]
fn every_fallback_to_guessing_is_counted() {
    let before = prefix_guess_hits();
    let got = select(Surface::OpenAiCompletions, Some("never-seen-model"), None);
    assert_eq!(got.level, SelectionLevel::PrefixGuess);
    assert!(prefix_guess_hits() > before, "落到 L4 却没有被记账");
}

/// 看不见模型名（流式请求体）时不会 panic，且行为与空模型名一致。
///
/// 守护的 bug：在 `select` 里对 `model` 做 `expect()`。缺陷 #2 的解药就是
/// 「计费降级、转发不降级」—— 一个超大的流式请求体看不见 `model` 是**正常路径**，
/// 不是异常路径。
#[test]
fn an_invisible_model_name_degrades_instead_of_panicking() {
    for surface in all_surfaces() {
        let none = select(surface, None, None);
        let empty = select(surface, Some(""), None);
        assert_eq!(none.candidates, empty.candidates);
        assert!(!none.candidates.is_empty(), "降级路径也必须给得出上游");
    }
}
