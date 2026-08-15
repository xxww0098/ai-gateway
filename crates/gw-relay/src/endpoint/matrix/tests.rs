//! OWNER: worker `relay-endpoints`。
//!
//! 规范 2.11：这里**不抄**矩阵里的任何一格。测的是矩阵必须满足的**不变量** ——
//! 直通等价于方言相同、拒绝必须给得出替代入口、错误信封必须是客户端 SDK
//! 读得懂的那一套。每条 doc 写明它守护的 bug，那个 bug 都塞回去跑红过再还原。

use serde_json::Value;

use super::*;

/// 每个入口**原生**说哪种上游 wire 协议。
///
/// 这不是矩阵的副本 —— 它是从入口自身的协议身份推出来的：
/// `/v1/chat/completions` 说 Chat Completions，`/v1/responses` 说 Responses，
/// `/v1/messages` 说 Anthropic Messages。矩阵必须与它一致，
/// 一致性正是下面几条测试要钉的东西。
fn native_dialect(surface: Surface) -> UpstreamDialect {
    match surface {
        Surface::OpenAiCompletions => UpstreamDialect::OpenAiChat,
        Surface::OpenAiResponses => UpstreamDialect::OpenAiResponses,
        Surface::AnthropicMessages => UpstreamDialect::AnthropicMessages,
    }
}

fn all_cells() -> Vec<(Surface, Provider)> {
    ALL_SURFACES
        .into_iter()
        .flat_map(|s| Provider::ALL.into_iter().map(move |p| (s, p)))
        .collect()
}

/// 直通**当且仅当**入口方言与上游方言是同一个。
///
/// 这条把「直通」从一个人工填的格子变成一个可推导的性质：一格之所以能直通，
/// 唯一的理由就是两边说同一种话。
///
/// 守护的 bug：把某一格从转义误标成直通（复制粘贴 match 臂时最容易发生）。
/// 那一格会把 Anthropic 形状的 body 原样送进 OpenAI 端点，客户端拿到一个
/// 来自上游的、与网关无关的 400，排查方向直接被带偏。
#[test]
fn passthrough_is_exactly_where_the_two_dialects_coincide() {
    for (surface, provider) in all_cells() {
        let same_dialect = upstream_dialect(surface, provider) == native_dialect(surface);
        let is_passthrough = cell(surface, provider) == Cell::Passthrough;
        assert_eq!(
            is_passthrough,
            same_dialect,
            "{:?} × {} 的直通判定与方言是否相同不一致",
            surface,
            provider.as_str()
        );
    }
}

/// 15 格一格不少，且三种判定都非空。
///
/// 守护的 bug：给 `cell()` 加一条 `_ => Cell::Passthrough` 兜底。
/// 那样新增 provider 时编译器不再点名，新上游会静默按「直通」处理 ——
/// 也就是把客户端的 body 原样打给一个说另一种话的上游。
#[test]
fn the_matrix_is_complete_and_uses_all_three_verdicts() {
    let cells: Vec<Cell> = all_cells().into_iter().map(|(s, p)| cell(s, p)).collect();
    assert_eq!(
        cells.len(),
        ALL_SURFACES.len() * Provider::ALL.len(),
        "矩阵的格数与「入口数 × 上游数」对不上"
    );
    for verdict in [Cell::Passthrough, Cell::Translate, Cell::Reject] {
        assert!(
            cells.contains(&verdict),
            "{verdict:?} 一格都没有 —— 矩阵退化了"
        );
    }
}

/// 入口 A 与入口 B 打到**同一个** OpenAI 系上游时，上游方言必须不同。
///
/// 这是缺陷 #1（S1）的守护点：今天 `/v1/responses` 的 body 被 POST 到
/// `{base}/v1/chat/completions`，因为端点由 provider 猜、而 provider 只会构造
/// 一种端点。OpenAI Python SDK 的 `client.responses.create()` 因此 100% 拿到
/// `400 Unrecognized request argument supplied: input` —— 三个保留入口之一全废。
///
/// 守护的 bug：把入口 B 的上游方言写回 `OpenAiChat`。
#[test]
fn the_responses_entry_does_not_collapse_onto_chat_completions() {
    for provider in [Provider::OpenAi, Provider::Codex] {
        assert_ne!(
            upstream_dialect(Surface::OpenAiResponses, provider),
            upstream_dialect(Surface::OpenAiCompletions, provider),
            "{} 上，两个 OpenAI 入口被折叠成了同一个上游端点",
            provider.as_str()
        );
    }
}

/// 入口 → 路径的逆映射与 `contract.rs` 里的正映射一致。
///
/// `Surface::from_path` 住在协调者独占的 `contract.rs`，`path_of` 是它在本模块里
/// 唯一的逆。守护的 bug：改了其中一边忘了改另一边 —— 那会让 400 的
/// 「请改用 POST /v1/xxx」指向一个不存在的路径，客户端照着改还是 404。
#[test]
fn the_path_mapping_round_trips() {
    for surface in ALL_SURFACES {
        assert_eq!(
            Surface::from_path(path_of(surface)),
            Some(surface),
            "{surface:?} 的路径逆映射与 contract.rs 的正映射漂移了"
        );
    }
}

/// 400 用**入口自身方言**的错误信封，且两种方言的结构确实不同。
///
/// 守护的 bug：两个入口回同一个网关自有格式。客户端 SDK 只解析它自己那套结构 ——
/// `@anthropic-ai/sdk` 读不到 `error.message`、OpenAI SDK 同理，
/// 于是一次失败被渲染成一个**无字的红叉**，用户看得到失败却读不到原因。
#[test]
fn a_rejection_speaks_the_dialect_the_client_sdk_can_parse() {
    let openai: Value = serde_json::from_slice(&reject_body(
        Surface::OpenAiResponses,
        Provider::Gemini,
        Some("gemini-2.5-pro"),
        RejectReason::SemanticsWouldBeLost,
    ))
    .expect("错误体必须是合法 JSON");
    let anthropic: Value = serde_json::from_slice(&reject_body(
        Surface::AnthropicMessages,
        Provider::Gemini,
        Some("gemini-2.5-pro"),
        RejectReason::TranslatorUnavailable,
    ))
    .expect("错误体必须是合法 JSON");

    // OpenAI SDK 读的那三个位置
    assert!(
        openai["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "OpenAI 客户端读不到 error.message"
    );
    assert!(openai["error"]["type"].as_str().is_some());
    assert!(openai["error"]["code"].as_str().is_some());

    // Anthropic SDK 读的那两个位置
    assert_eq!(
        anthropic["type"].as_str(),
        Some("error"),
        "Anthropic 客户端靠顶层 type 判断这是不是错误"
    );
    assert!(
        anthropic["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "Anthropic 客户端读不到 error.message"
    );

    assert_ne!(
        openai.as_object().map(|o| o.keys().collect::<Vec<_>>()),
        anthropic.as_object().map(|o| o.keys().collect::<Vec<_>>()),
        "两种方言的信封结构必须不同，否则至少有一边的 SDK 解析不了"
    );
}

/// 拒绝信里的指引是**可执行**的：它给出的替代入口真的能承接那个 provider。
///
/// 守护的 bug：把替代入口写成硬编码字符串。矩阵一改（例如 P2 落地或某格被撤），
/// 那句指引就开始说谎，而说谎的错误提示比没有提示更浪费用户时间。
#[test]
fn the_rejection_tells_the_client_somewhere_that_actually_works() {
    for (surface, provider) in all_cells() {
        if cell(surface, provider) != Cell::Reject {
            continue;
        }
        let body = reject_body(
            surface,
            provider,
            Some("some-model"),
            RejectReason::SemanticsWouldBeLost,
        );
        let text = String::from_utf8(body.to_vec()).expect("信封是 UTF-8");

        let alternatives: Vec<Surface> = surfaces_serving(provider)
            .into_iter()
            .filter(|s| *s != surface)
            .collect();
        assert!(
            !alternatives.is_empty(),
            "{} 被这个入口拒了，却没有任何入口能承接它 —— 那它不该出现在矩阵里",
            provider.as_str()
        );
        for alt in alternatives {
            assert!(
                text.contains(path_of(alt)),
                "指引里没提到真正可用的入口 {}",
                path_of(alt)
            );
            assert_ne!(
                cell(alt, provider),
                Cell::Reject,
                "指引指向了一个同样会拒绝的入口"
            );
        }
        assert!(
            text.contains(provider.as_str()),
            "指引没说清模型属于哪个渠道"
        );
        assert!(text.contains("some-model"), "指引没说清是哪个模型被拒了");
    }
}

/// 模型名里带引号 / 换行 / 反斜杠时，错误体仍然是合法 JSON。
///
/// 守护的 bug：用 `format!` 手拼 JSON 而不是让 serde 转义。一个模型名叫
/// `a"b` 的请求会产出一段坏 JSON，客户端 SDK 解析失败 —— 又一个无字的红叉，
/// 而且这次连状态码之外的任何信息都没有。
#[test]
fn a_hostile_model_name_cannot_break_the_envelope() {
    for hostile in ["a\"b", "line\nbreak", "back\\slash", "\u{0}nul", "emoji🚀"] {
        for surface in ALL_SURFACES {
            let body = reject_body(
                surface,
                Provider::Vertex,
                Some(hostile),
                RejectReason::TranslatorUnavailable,
            );
            let parsed: Value = serde_json::from_slice(&body).expect("模型名不该能撑破错误信封");
            // 两种方言都把 message 挂在 `error.message` 下，只是外层不同。
            let message = &parsed["error"]["message"];
            assert!(
                message.as_str().is_some_and(|m| m.contains(hostile)),
                "模型名在转义后丢失了：{hostile:?}"
            );
        }
    }
}

/// `route()` 的三个分支与 `cell()` 的三个判定一一对应，且拒绝分支自带 400 的字节。
///
/// 守护的 bug：`route()` 里某个分支忘了跟着 `cell()` 走（例如把 Translate 也
/// 当成 Passthrough 转发出去）。那一格会把客户端的 body 原样送给一个说另一种话
/// 的上游，而不是回一个能读懂的 400。
#[test]
fn routing_follows_the_table_and_carries_its_own_error_bytes() {
    for (surface, provider) in all_cells() {
        let decision = route(surface, provider, Some("m"));
        match cell(surface, provider) {
            Cell::Passthrough => assert!(matches!(decision, Route::Passthrough { .. })),
            Cell::Translate => assert!(matches!(decision, Route::Translate { .. })),
            Cell::Reject => {
                let Route::Reject(bytes) = decision else {
                    panic!("{surface:?} × {} 该被拒却没被拒", provider.as_str())
                };
                assert!(
                    serde_json::from_slice::<Value>(&bytes).is_ok(),
                    "拒绝分支给出的不是合法 JSON"
                );
            }
        }
        if let Route::Passthrough { upstream } | Route::Translate { upstream } =
            route(surface, provider, Some("m"))
        {
            assert_eq!(
                upstream,
                upstream_dialect(surface, provider),
                "派发结论里的上游方言与查表结果不一致"
            );
        }
    }
    assert_eq!(REJECT_STATUS.as_u16(), 400);
}

/// 每个需要转义的格子都拿得到转义器，而且拿到的是**它自己承认**属于这一格的那个。
///
/// 判据不是我手写的对照表，而是转义器自述的 `surface()` / `to_dialect()`
/// —— 用被选中者自己的声明当预言机，接线接错了立刻发现。
///
/// 守护的 bug：把 `AnthropicToGoogle` 接到 openai 入口那一格上。转义会照跑，
/// 产出的 Google 请求体也合法，只是它按 Anthropic 的字段名去读一个 OpenAI 的 body
/// —— `messages` 读得到、`system` 读不到，客户端拿到一个丢了 system prompt
/// 却完全成功的回答。这是最难发现的一类错误：没有报错，只是答得不对。
#[test]
fn every_translating_cell_gets_the_translator_that_claims_it() {
    for (surface, provider) in all_cells() {
        let upstream = upstream_dialect(surface, provider);
        let picked = translator_for(surface, upstream);
        match cell(surface, provider) {
            Cell::Translate => {
                let t = picked.unwrap_or_else(|| {
                    panic!("{surface:?} × {} 需要转义却没有转义器", provider.as_str())
                });
                assert_eq!(t.surface(), surface, "转义器自述的入口对不上");
                assert_eq!(t.to_dialect(), upstream, "转义器自述的上游方言对不上");
            }
            Cell::Passthrough | Cell::Reject => assert!(
                picked.is_none(),
                "{surface:?} × {} 不需要转义，却选出了一个转义器",
                provider.as_str()
            ),
        }
    }
}

/// executor 名与 provider 之间是一一对应的双射。
///
/// 守护的 bug：两个 provider 复制粘贴出同一个 `as_str()`。那样
/// `Provider::from_name` 会把其中一个永远解析成另一个，配置里写 `vertex`
/// 的渠道会静默走到 gemini 的账号池上。
#[test]
fn provider_names_are_a_bijection() {
    for p in Provider::ALL {
        assert_eq!(Provider::from_name(p.as_str()), Some(p));
    }
    let names: Vec<&str> = Provider::ALL.iter().map(|p| p.as_str()).collect();
    let mut deduped = names.clone();
    deduped.sort_unstable();
    deduped.dedup();
    assert_eq!(deduped.len(), names.len(), "有两个 provider 共用了一个名字");
    assert!(
        Provider::from_name("nope").is_none(),
        "不认识的名字不许兜底"
    );
}
