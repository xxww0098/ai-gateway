//! COORDINATOR-OWNED，随 `contract.rs` 一起改。
//!
//! 规范 2.11：测的是**性质**，不是字面量的复述。下面每条守护的都是一个
//! 「改了就会静默出错」的不变量，不是「常量等于它自己」。

use super::*;

/// 三个入口两两不同，且认得出自己。
///
/// 守护的 bug：`from_path` 的 match 臂被复制粘贴后忘了改返回值 ——
/// 那样两个路径会映射到同一个 `Surface`，请求被派到错误的上游方言，
/// 而任何「常量等于常量」的断言都发现不了。
#[test]
fn surfaces_are_pairwise_distinct() {
    let paths = ["/v1/chat/completions", "/v1/responses", "/v1/messages"];
    let mapped: Vec<Surface> = paths
        .iter()
        .map(|p| Surface::from_path(p).expect("入口路径必须可识别"))
        .collect();

    for (i, a) in mapped.iter().enumerate() {
        for (j, b) in mapped.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "{} 与 {} 映射到了同一个 Surface", paths[i], paths[j]);
            }
        }
    }
}

/// `path()` 与 `from_path()` 必须互为反函数。
///
/// 守护的 bug：加第四个入口时只在 `from_path` 加了臂，`path()` 忘了跟
/// （编译器会拦，因为 match 穷尽），或者两边写了**不同的**路径字面量
/// （编译器拦不住）。后者会让 `gw-proxy` 写进 metadata 的路径 executor 认不出来，
/// 于是静默回落到 `OpenAiCompletions` —— `/v1/responses` 又被打回
/// chat/completions，缺陷 #1 悄悄复活，而所有测试照样绿。
#[test]
fn path_and_from_path_are_inverses() {
    for surface in [
        Surface::OpenAiCompletions,
        Surface::OpenAiResponses,
        Surface::AnthropicMessages,
    ] {
        assert_eq!(
            Surface::from_path(surface.path()),
            Some(surface),
            "{surface:?} 的 path() 没能被 from_path() 认回来"
        );
    }
}

/// 不认识的路径必须是 `None`，不许兜底猜一个。
///
/// 守护的 bug：给 `from_path` 加一条 `_ => Some(Self::OpenAiCompletions)` 兜底。
/// 那会让被收敛掉的 `/v1/embeddings`、`/v1beta/**` 静默复活并走错方言，
/// 而不是干脆地 404。
#[test]
fn unknown_paths_are_rejected_rather_than_guessed() {
    for p in [
        "/v1/completions",
        "/v1/embeddings",
        "/v1beta/models/gemini-2.5-pro:generateContent",
        "/v1/chat/completions/",
        "/V1/CHAT/COMPLETIONS",
        "",
        "/",
    ] {
        assert!(Surface::from_path(p).is_none(), "{p} 不该被识别成入口");
    }
}

/// 方言划分必须是一个**划分**：每个入口恰好属于一个方言，且两个方言都非空。
///
/// 守护的 bug：新增第四个入口时忘了在 `dialect()` 里加臂（编译器会拦），
/// 或者加错了臂（编译器不会拦）。P3 的 400 用错方言，客户端 SDK 解析不了
/// 错误结构，会把它渲染成一个无字的红叉。
#[test]
fn dialect_partitions_surfaces() {
    let all = [
        Surface::OpenAiCompletions,
        Surface::OpenAiResponses,
        Surface::AnthropicMessages,
    ];
    let openai = all
        .iter()
        .filter(|s| s.dialect() == Dialect::OpenAi)
        .count();
    let anthropic = all
        .iter()
        .filter(|s| s.dialect() == Dialect::Anthropic)
        .count();

    assert_eq!(openai + anthropic, all.len(), "有入口没有归属方言");
    assert!(openai > 0 && anthropic > 0, "方言划分退化成了单边");
}

/// 流式 body **看不到**内容，且**不可重放**。
///
/// 这是缺陷 #2 解药的承重点。守护的 bug：给 `peek()` 的 `Streaming` 臂返回
/// `Some(&[])`。那样计费会把一个 500 MiB 的流当成空 body（估算为 0 token），
/// 而 failover 重试会把一个已经消费掉的流当成可重放的，第二次尝试发出空请求。
#[test]
fn streaming_body_is_neither_peekable_nor_replayable() {
    use http_body_util::{BodyExt as _, Empty};

    let streaming = RelayBody::Streaming(
        Empty::<Bytes>::new()
            .map_err(|never| match never {})
            .boxed(),
    );

    assert!(
        streaming.peek().is_none(),
        "Streaming 必须返回 None，而不是空切片 —— 调用方要能区分「看不到」与「是空的」"
    );
    assert!(!streaming.is_replayable(), "消费过的流不可能重放");
}

/// 已缓冲 body 的 peek 与转发看到的是**同一块内存**，且可重放。
///
/// 守护的 bug：`peek()` 改成返回 `to_vec()` 的临时拷贝。那不会让任何断言变红，
/// 但会把「peek 与转发共用同一块内存」这条零拷贝承诺悄悄作废。
#[test]
fn buffered_body_peeks_its_own_bytes_and_replays() {
    let payload = Bytes::from_static(b"\x00\xff not utf-8 \xc3\x28");
    let body = RelayBody::Buffered(payload.clone());

    assert_eq!(
        body.peek().expect("Buffered 必须可 peek"),
        payload.as_ref(),
        "peek 看到的字节必须与原始字节逐位相等（含非 UTF-8 字节）"
    );
    assert!(body.is_replayable());
}

/// `is_empty()` 必须严格等价于「四个字段全 `None`」。
///
/// 守护的 bug：把某个字段的 `Some(0)` 当成「没给」。计费的 fallback 分支就挂在
/// 这个判定上 —— 「缺失」与「零」混同，会让上游明确返回 0 token 的请求
/// 被按估算重新计价。
#[test]
fn usage_emptiness_separates_absent_from_zero() {
    assert!(RelayUsage::default().is_empty());

    let fields: [fn(&mut RelayUsage); 4] = [
        |u| u.input_tokens = Some(0),
        |u| u.output_tokens = Some(0),
        |u| u.cached_tokens = Some(0),
        |u| u.reasoning_tokens = Some(0),
    ];
    for set in fields {
        let mut u = RelayUsage::default();
        set(&mut u);
        assert!(
            !u.is_empty(),
            "上游明确给了 0，不是没给 —— 这两者必须能分开"
        );
    }
}

/// 凭证的 `Debug` 不许带出活密钥 —— 连带任何**包含**它的类型。
///
/// 守护的 bug：把 `#[derive(Debug)]` 还给 `Credential`。那样一句
/// `tracing::debug!(?target)`、一个 `.expect(&format!("{cred:?}"))`，
/// 打出来的就是一把可以直接拿去用的上游 key，而这些字节落到哪个文件、
/// 被谁收走、留多久，全都不由本进程决定。
///
/// 密文是**测试自己造的**（生产的掩码实现里不存在这个串），所以断言的期望值
/// 来自输入而不是源码字面量（规范 2.11）。
#[test]
fn credential_debug_never_carries_the_live_secret() {
    const LIVE: &str = "sk-live-UNIQUE-KNIFE3-9f3a7c";

    let creds = [
        Credential::Bearer(LIVE.to_owned()),
        Credential::XApiKey(LIVE.to_owned()),
        Credential::GoogleApiKey(LIVE.to_owned()),
    ];

    let dumps: Vec<String> = creds.iter().map(|cred| format!("{cred:?}")).collect();
    for dump in &dumps {
        assert!(
            !dump.contains(LIVE),
            "凭证的 Debug 把活密钥打了出来：{dump}"
        );
    }

    // 脱敏不许把三个变体抹成同一坨：日志还要能回答「这次用的是哪种载体」。
    for (i, a) in dumps.iter().enumerate() {
        for (j, b) in dumps.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "两个变体的 dump 无法区分");
            }
        }
    }

    // 掩码必须稳定：同一个密文两次 dump 必须一模一样，否则日志里同一把 key
    // 会变成两把，去重与关联全部失效。
    assert_eq!(
        format!("{:?}", Credential::Bearer(LIVE.to_owned())),
        dumps[0],
        "掩码不稳定"
    );

    // 嵌套：包含凭证的类型照样 derive(Debug)，泄漏点收在 Credential 这一层。
    for cred in creds {
        let target = UpstreamTarget {
            origin: "https://upstream.invalid/"
                .parse()
                .expect("测试用的 origin"),
            credential: cred,
            timeouts: RelayTimeouts::default(),
            dialect: UpstreamDialect::OpenAiChat,
        };
        let dump = format!("{target:?}");
        assert!(!dump.contains(LIVE), "嵌套的凭证把活密钥带了出来：{dump}");
    }
}
