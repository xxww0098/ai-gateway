//! Header 复制与 hop-by-hop 过滤。**本 crate 里 header 复制只能有这一处实现。**
//!
//! OWNER: worker `relay-core`。
//!
//! 存在的理由是缺陷 #7：出站方向用 `keys()` + `get_all()` + `append`（对的），
//! 回写方向用 `iter()` + `insert`（错的，把同名多值折叠成最后一个）。
//! 同一个概念两处实现，就是这么漂的。
//!
//! 所以这里只有**一个**复制函数 [`copy_preserving_multivalue`]，两个方向共用：
//! 出站由 [`upstream_headers`] 在它上面再加凭证换绑与 `accept-encoding` 收敛，
//! 回写由 `gw-proxy` 直接调它。谁都不许再写第二个循环。
//!
//! # 这里根除的缺陷
//!
//! | 编号 | 内容 |
//! | --- | --- |
//! | #7  | 回写 `insert` 折叠多值 → 只有 `append` |
//! | #10 | `expect: 100-continue` 没被消费 → 进黑名单 |
//! | #17 | 本层读过的凭证载体被原样转给上游 → 出站先剥后设 |
//! | #18 | 流式强制覆盖 `Accept` → 撤销，一个字节都不碰 |
//! | #5  | `accept-encoding` 原样转发导致旁路 usage 必然读到 gzip 魔数 → 默认收敛成 `identity` |

use http::HeaderMap;
use http::header::{ACCEPT_ENCODING, AUTHORIZATION, CONTENT_LENGTH, HOST, HeaderName, HeaderValue};

use crate::contract::{Credential, RelayTransportError};

/// Anthropic 的凭证载体。
const X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");
/// Google AI Studio 的凭证载体。
const X_GOOG_API_KEY: HeaderName = HeaderName::from_static("x-goog-api-key");

/// 代理必须自己消费、绝不能转发的 header。
///
/// 前八个是 RFC 7230 §6.1 的逐跳集合；第九个 `expect` 是**缺陷 #10** 补上的：
/// `Expect: 100-continue` 是逐跳的连接控制头，hyper 的 client 不做 100-continue
/// 协商，转给上游只会多一次 RTT，严格的中间设备直接 417。
///
/// `host` 与 `content-length` 不在这里 —— 它们不是逐跳头，是**本地重算**的头
/// （审计 §4.1 #8 特意点了这个分类问题）。判定见 [`is_locally_rebuilt`]。
fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "expect"
    )
}

/// 两个方向都必须由传输层重新算出来的 header。
///
/// - `host`：请求换了 origin，必须跟着上游 URL 重建（审计 §4.1 #5）。
/// - `content-length`：请求体在 `Streaming` 态下压根没有确定长度，响应体的长度
///   由本地 body 决定（审计 §4.1 #6 / #8）。转发一个旧长度就是协议错误。
fn is_locally_rebuilt(name: &HeaderName) -> bool {
    name == HOST || name == CONTENT_LENGTH
}

/// 把 `src` 的 header 复制到 `dst`，**保留同名多值**，跳过逐跳头与本地重算头。
///
/// # 根除缺陷 #7
///
/// 必须 `append` 而不是 `insert`。OpenAI 经 Cloudflare 返回两条 `set-cookie`
/// （`__cf_bm` 与 `_cfuvid`），`insert` 会让客户端只收到最后一条，缺了 `__cf_bm`
/// 之后每个请求都被 Cloudflare 当新会话，bot 分数掉下去就开始吃 403 挑战页 ——
/// 表现为**间歇性 403 而不是稳定失败**，最难排查的那种。
///
/// # 根除缺陷 #10
///
/// 黑名单里带 `expect`，见 [`is_hop_by_hop`]。
///
/// # 两个方向共用
///
/// 出站（客户端 → 上游）与回写（上游 → 客户端）走的是同一个函数体。
/// 出站方向额外的凭证换绑在 [`upstream_headers`] 里做 —— 那是**请求方向独有**的
/// 语义，塞进这里会让回写方向平白无故丢 header。
pub fn copy_preserving_multivalue(dst: &mut HeaderMap, src: &HeaderMap) {
    for name in src.keys() {
        if is_hop_by_hop(name) || is_locally_rebuilt(name) {
            continue;
        }
        for value in src.get_all(name) {
            dst.append(name.clone(), value.clone());
        }
    }
}

/// 构造发往上游的 header。
///
/// 在 [`copy_preserving_multivalue`] 之上只多做三件事，**其余字节一律不碰**：
///
/// 1. **剥掉本层读过的凭证载体**（根除观察 #17）。`authorization` / `x-api-key` /
///    `x-goog-api-key` 三个都剥 —— 不管入口用的是哪一个，因为「凡是本层读过的
///    凭证载体，本层负责剥掉」。今天 `/v1/*` 上客户端的 `x-api-key` 会被原样转给
///    OpenAI：不是凭证泄漏（access 层只认 `Authorization`），但是无意义的信息外泄。
/// 2. **按 `cred` 的类型 set 对应的头**（审计 §4.1 #1–#3：租户凭证与上游凭证是
///    两个信任域，不能串）。值标记为 sensitive，不会进 hyper 的 debug 日志。
/// 3. **`accept-encoding` 按策略处理**（根除缺陷 #5），见下。
///
/// # 根除缺陷 #18：不碰 `Accept`
///
/// 流式时**不**强制覆盖成 `text/event-stream`，缺失时**不**补 `application/json`。
/// 上游是按 body 里的 `stream` 字段决定框架的，不看 `Accept` —— 这次覆盖没有任何
/// 正确性收益，属于「顺手改了」（审计 §4.2 #2 / #4）。`content-type` 同理。
///
/// # 根除缺陷 #5：`accept-encoding`
///
/// `preserve_client_encoding == false`（默认）时把它收敛成 `identity`：
/// 上游不压缩，旁路 [`crate::UsageProbe`] 读到的就是明文。否则上游按需 gzip，
/// probe 拿到的是 gzip 魔数 `1f 8b`，解析必然 `None`，于是**每一次非流式请求都
/// 按估算而不是真实 token 计费**；`billing.strict_usage_metadata_mode` 下更糟 ——
/// 请求成功但 `usage_logs.failed = true` 且完全不结算。
///
/// `preserve_client_encoding == true` 时原样透传客户端的 `accept-encoding`，
/// 并**显式接受 usage 落 fallback 结算**这个后果。要么正确，要么显式降级，
/// 不要留成现在这种「看起来能跑、计费静默失真」的状态。
///
/// # Errors
///
/// 凭证不是合法 header value（含 CR/LF 或非 ASCII）时返回
/// [`RelayTransportError::BadCredential`]。
pub(crate) fn upstream_headers(
    src: &HeaderMap,
    cred: &Credential,
    preserve_client_encoding: bool,
) -> Result<HeaderMap, RelayTransportError> {
    let mut out = HeaderMap::with_capacity(src.len() + 1);
    copy_preserving_multivalue(&mut out, src);

    // 1. 先剥 —— 三个载体全剥，不看入口用的是哪一个。
    out.remove(AUTHORIZATION);
    out.remove(X_API_KEY);
    out.remove(X_GOOG_API_KEY);

    // 2. 后设。
    let (name, raw) = match cred {
        Credential::Bearer(token) => (AUTHORIZATION, format!("Bearer {token}")),
        Credential::XApiKey(key) => (X_API_KEY, key.clone()),
        Credential::GoogleApiKey(key) => (X_GOOG_API_KEY, key.clone()),
    };
    let mut value = HeaderValue::try_from(raw).map_err(|_| RelayTransportError::BadCredential)?;
    value.set_sensitive(true);
    out.insert(name, value);

    // 3. accept-encoding。
    if !preserve_client_encoding {
        out.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    }

    Ok(out)
}

#[cfg(test)]
mod tests;
