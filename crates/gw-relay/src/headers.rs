//! Header 复制与 hop-by-hop 过滤。**本 crate 里 header 复制只能有这一处实现。**
//!
//! OWNER: worker `relay-core`。
//!
//! 存在的理由是缺陷 #7：出站方向用 `keys()` + `get_all()` + `append`（对的），
//! 回写方向用 `iter()` + `insert`（错的，把同名多值折叠成最后一个）。
//! 同一个概念两处实现，就是这么漂的。

use http::HeaderMap;

/// 把 `src` 的 header 复制到 `dst`，**保留同名多值**。
///
/// 缺陷 #7 的解药：必须 `append` 而不是 `insert`。OpenAI 经 Cloudflare 返回
/// 两条 `set-cookie`（`__cf_bm` 与 `_cfuvid`），`insert` 会让客户端只收到最后
/// 一条，缺了 `__cf_bm` 之后每个请求都被 Cloudflare 当新会话，bot 分数掉下去
/// 就开始吃 403 挑战页 —— 表现为**间歇性 403**，最难排查的那种。
pub fn copy_preserving_multivalue(dst: &mut HeaderMap, src: &HeaderMap) {
    let _ = (dst, src);
    todo!("relay-core: 见 docs/relay-passthrough-audit.md 缺陷 #7")
}

#[cfg(test)]
mod tests;
