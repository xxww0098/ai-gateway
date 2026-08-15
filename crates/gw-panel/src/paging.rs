//! 分页与路径参数的解析规则，以及两种分页信封。
//!
//! 对应 `queryInt` / `parseUintParam`，以及各 handler 手写的
//! `gin.H{"items":…, "page":…}`。
//!
//! 分页信封有**两种形状**，而且是按 handler 定的，不是按域定的：
//!
//! | 形状 | 键 | 用在 |
//! | --- | --- | --- |
//! | [`Page`] | `items` `page` `page_size` `total` `total_pages` | `/user/api-keys`、`/user/balance-history` |
//! | [`ListPage`] | `items` `total` `page` `page_size` | 管理员列表、工单、订单、退款、通知 |
//!
//! 前端不改，所以少一个 `total_pages` 或多一个都是破坏性变更。照 handler 原样，别统一。

use serde::Serialize;

#[cfg(test)]
mod tests;

/// 带 `total_pages` 的分页信封。
#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
    pub total_pages: i64,
}

impl<T> Page<T> {
    /// 按 `int(math.Ceil(float64(total) / float64(pageSize)))` 计算总页数。
    ///
    /// `page_size` 恒为正（[`query_int`] 的 min 至少是 1），所以不必防除零；
    /// `total = 0` 时这里也是 0。
    #[must_use]
    pub fn new(items: Vec<T>, page: i64, page_size: i64, total: i64) -> Self {
        Self {
            items,
            page,
            page_size,
            total,
            total_pages: total_pages(total, page_size),
        }
    }
}

/// 不带 `total_pages` 的分页信封。
#[derive(Debug, Clone, Serialize)]
pub struct ListPage<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

impl<T> ListPage<T> {
    #[must_use]
    pub fn new(items: Vec<T>, total: i64, page: i64, page_size: i64) -> Self {
        Self {
            items,
            total,
            page,
            page_size,
        }
    }
}

/// 总页数 = `int(math.Ceil(float64(total)/float64(pageSize)))`。
#[must_use]
pub fn total_pages(total: i64, page_size: i64) -> i64 {
    if page_size <= 0 || total <= 0 {
        return 0;
    }
    // `i64::div_ceil` 还没稳定（int_roundings），手写向上取整。total / size 的
    // 商加上「有余数就再进一页」，与 math.Ceil 逐值相同，且不经过 f64。
    total / page_size + i64::from(total % page_size != 0)
}

/// 解析并夹紧一个查询参数。对应 `queryInt`。
///
/// 有两处行为容易写漏，这里逐条保持：
///
/// * `strconv.Atoi` 失败（缺参、空串、`"abc"`、`"12x"`）→ 用 `def`，**然后仍然过夹紧**；
/// * 夹紧是先 `< min` 再 `> max`，所以 `def` 本身越界也会被夹回去。
#[must_use]
pub fn query_int(raw: Option<&str>, def: i64, min: i64, max: i64) -> i64 {
    let value = raw.and_then(|s| s.parse::<i64>().ok()).unwrap_or(def);
    value.clamp(min, max)
}

/// 解析路径上的实体 id。对应 `parseUintParam`。
///
/// 用 `ParseUint` 并**把 0 也当成错误**（`err != nil || value == 0`），因为
/// 自增主键从 1 开始，`0` 只可能来自伪造的 URL。负数同样落进 `None`。
#[must_use]
pub fn parse_id(raw: &str) -> Option<i64> {
    match raw.trim().parse::<u64>() {
        Ok(0) | Err(_) => None,
        Ok(v) => i64::try_from(v).ok(),
    }
}

/// `(page, page_size)` 的常见组合：`page ∈ [1, 1_000_000]`，`page_size ∈ [1, 100]`。
///
/// `page_size` 的默认值按 handler 不同（20 / 15 / 30 / 10），所以由调用方传入。
#[must_use]
pub fn page_params(
    page_raw: Option<&str>,
    size_raw: Option<&str>,
    default_size: i64,
) -> (i64, i64) {
    let page = query_int(page_raw, 1, 1, 1_000_000);
    let page_size = query_int(size_raw, default_size, 1, 100);
    (page, page_size)
}

/// `LIMIT`/`OFFSET` 里的 offset。
#[must_use]
pub fn offset(page: i64, page_size: i64) -> i64 {
    (page - 1) * page_size
}
