//! 租约用尽的判定，以及 TTL 与最长时长是两个旋钮这件事。
//!
//! 续租本身要动 Redis 与 Postgres，所以它的性质在
//! `tests/redis_ledger.rs` 里对着真服务测（`#[ignore]`）。这里只留纯判定。

use super::*;

/// 到点就是到点：恰好等于上限的租约不再续一片。
///
/// `>` 而不是 `>=` 会让一条流每次都恰好卡在上限上再续一次 —— 上限就永远
/// 差一片生效不了。
#[test]
fn a_lease_is_exhausted_the_moment_it_reaches_the_cap_not_after() {
    let cap = Duration::from_secs(60);
    assert!(!lease_exhausted(cap - Duration::from_nanos(1), cap));
    assert!(lease_exhausted(cap, cap), "恰好到上限必须算用尽");
    assert!(lease_exhausted(cap + Duration::from_secs(1), cap));
}

/// 零龄的预留永远续得下去（只要上限为正）—— 否则第一次续租就会失败。
#[test]
fn a_brand_new_hold_is_never_exhausted() {
    for cap in [Duration::from_secs(1), DEFAULT_MAX_HOLD_DURATION] {
        assert!(!lease_exhausted(Duration::ZERO, cap));
    }
}

/// **最长时长必须远大于一片租约**，否则续租机制毫无意义：
/// 流还没跑到需要第二片就已经被硬顶拒绝了。
#[test]
fn the_maximum_duration_leaves_room_for_more_than_one_lease_slice() {
    assert!(
        DEFAULT_MAX_HOLD_DURATION > crate::DEFAULT_HOLD_TTL,
        "最长时长 {DEFAULT_MAX_HOLD_DURATION:?} 不该短于一片租约 {:?}",
        crate::DEFAULT_HOLD_TTL,
    );
    assert!(
        !lease_exhausted(crate::DEFAULT_HOLD_TTL, DEFAULT_MAX_HOLD_DURATION),
        "跑满一片租约的流必须还能续下去",
    );
}
