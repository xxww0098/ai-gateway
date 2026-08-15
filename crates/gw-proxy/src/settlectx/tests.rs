//! The single-settlement guarantee. Everything downstream — the hold
//! finalizer, the dispatcher, the stream settler — relies on exactly one of
//! them winning, which is what keeps cross-account failover from double-billing.

use std::sync::Arc;

use super::*;

fn billing() -> Arc<RequestBilling> {
    Arc::new(RequestBilling::new(
        SettleCtx {
            request_id: "req-1".to_owned(),
            user_id: 1,
            rate_mult: 1.0,
            ..SettleCtx::default()
        },
        0.5,
        false,
    ))
}

#[test]
fn the_first_claim_wins_and_later_ones_lose() {
    let billing = billing();
    assert!(!billing.is_finalized());
    assert!(billing.claim_finalize());
    assert!(billing.is_finalized());
    assert!(!billing.claim_finalize());
    assert!(!billing.claim_finalize());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exactly_one_of_many_concurrent_claimants_settles() {
    // The dispatcher and the hold finalizer race on every request; a second
    // winner here would be a double charge.
    for _ in 0..64 {
        let billing = billing();
        let winners = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let billing = billing.clone();
            let winners = winners.clone();
            tasks.push(tokio::spawn(async move {
                if billing.claim_finalize() {
                    winners.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }));
        }
        for task in tasks {
            task.await.expect("task joins");
        }
        assert_eq!(winners.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[test]
fn the_reservation_details_the_finalizer_needs_survive_on_the_handle() {
    let billing = RequestBilling::new(SettleCtx::default(), 1.25, true);
    assert_eq!(billing.hold_amount, 1.25);
    assert!(
        billing.used_budget_token,
        "a budget-token reservation must be distinguishable, or the finalizer \
         would Release a Redis hold that was never taken",
    );
}
