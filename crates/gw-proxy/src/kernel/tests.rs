//! 状态机边与内核不变量。测的是「哪些跳转合法」，不抄源码里的阶段名去对拍
//! 某一条固定路径的字面量。

use super::{Phase, RelayCtx};
use crate::ports::AccessMetadata;

fn meta() -> AccessMetadata {
    AccessMetadata {
        user_id: 7,
        api_key_id: 3,
        group_id: None,
        rate_mult: 1.0,
        subscription: None,
    }
}

/// 每条合法边都能走；终态没有出边。
#[test]
fn the_happy_path_is_a_single_forward_chain() {
    let chain = [
        Phase::Received,
        Phase::Authenticated,
        Phase::Inspected,
        Phase::Gated,
        Phase::Reserved,
        Phase::Routed,
        Phase::Attempting,
        Phase::Relaying,
        Phase::Settled,
    ];
    for window in chain.windows(2) {
        assert!(
            window[0].can_transition_to(window[1]),
            "{:?} should reach {:?}",
            window[0],
            window[1],
        );
    }
    assert!(Phase::Settled.is_terminal());
}

/// 鉴权之前不能预扣：这是 B1 在状态机上的形状。
#[test]
fn a_hold_cannot_precede_authentication() {
    assert!(!Phase::Received.can_transition_to(Phase::Reserved));
    assert!(!Phase::Received.can_transition_to(Phase::Inspected));
    assert!(Phase::Received.can_transition_to(Phase::Authenticated));
    assert!(Phase::Received.can_transition_to(Phase::Failed));
}

/// 预扣之前的拒绝必须落 Failed，不能偷偷进 Reserved。
#[test]
fn a_rejection_before_the_hold_never_enters_reserved() {
    for from in [Phase::Authenticated, Phase::Inspected, Phase::Gated] {
        assert!(from.can_transition_to(Phase::Failed));
        if from != Phase::Gated {
            assert!(!from.can_transition_to(Phase::Reserved));
        }
    }
}

/// 不计费路径从 Authenticated 直接 Skipped，不经过 hold。
#[test]
fn a_zero_cost_path_skips_the_reservation() {
    assert!(Phase::Authenticated.can_transition_to(Phase::Skipped));
    assert!(!Phase::Authenticated.can_transition_to(Phase::Reserved));
    assert!(Phase::Skipped.is_terminal());
}

/// 跨账号重试停在 Attempting，不重新预扣。
#[test]
fn failover_retries_stay_on_the_same_reservation() {
    assert!(Phase::Attempting.can_transition_to(Phase::Attempting));
    assert!(!Phase::Attempting.can_transition_to(Phase::Reserved));
    assert!(Phase::Attempting.can_transition_to(Phase::Relaying));
}

/// Settled / StrictHeld 只能从 Relaying 进入；Released 还可以从 Reserved
/// 进入（幂等抢锁失败、预扣后被拒），这是「释放而不是结算」。
#[test]
fn settlement_terminals_are_reached_only_after_relay() {
    for terminal in [Phase::Settled, Phase::StrictHeld] {
        assert!(Phase::Relaying.can_transition_to(terminal));
        assert!(!Phase::Reserved.can_transition_to(terminal));
        assert!(terminal.is_terminal());
        assert!(!terminal.can_transition_to(Phase::Reserved));
    }
    assert!(Phase::Relaying.can_transition_to(Phase::Released));
    assert!(Phase::Reserved.can_transition_to(Phase::Released));
    assert!(Phase::Released.is_terminal());
}

/// 非法跳转不改状态：一次写错不能把后续结算带偏。
#[test]
fn an_illegal_jump_leaves_the_context_untouched() {
    let mut ctx = RelayCtx::authenticated(meta());
    ctx.advance(Phase::Reserved);
    assert_eq!(ctx.phase, Phase::Authenticated);
    ctx.advance(Phase::Inspected);
    assert_eq!(ctx.phase, Phase::Inspected);
}

/// 终态集合是闭合的：任何终态都不能再走进计费边。
#[test]
fn terminal_phases_have_no_outbound_billing_edge() {
    for phase in [
        Phase::Skipped,
        Phase::Settled,
        Phase::Released,
        Phase::StrictHeld,
        Phase::Failed,
    ] {
        for next in [
            Phase::Received,
            Phase::Authenticated,
            Phase::Inspected,
            Phase::Gated,
            Phase::Reserved,
            Phase::Routed,
            Phase::Attempting,
            Phase::Relaying,
            Phase::Settled,
            Phase::Released,
        ] {
            assert!(
                !phase.can_transition_to(next),
                "{phase:?} must not re-enter {next:?}",
            );
        }
    }
}
