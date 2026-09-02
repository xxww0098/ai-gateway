//! Hold admission, expiry and lifecycle — everything that runs inside the Lua
//! scripts, plus the ordering rule that a reservation is cleared only after
//! the debit is durable.
//!
//! Every test here is `#[ignore]`d — see `tests/common/mod.rs` for how to run
//! them.

mod common;

use std::time::Duration;

use common::{FAULT_PREFIX, Fixture, Rng};
use gw_ledger::{BillingOperationId, LedgerError, NewOperation};

const FIVE_MINUTES: Duration = Duration::from_secs(300);
const EPSILON: f64 = 1e-9;

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPSILON
}

/// With a cached balance `B` and live holds `H₁..Hₙ`, a hold of `A` is
/// admitted **iff** `B - ΣHᵢ >= A`. This is the admission rule the whole
/// overdraft guarantee rests on.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn a_hold_is_admitted_exactly_when_the_available_balance_covers_it() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let mut rng = Rng::new(0x8010_0001);

    for i in 0..40 {
        let balance = (rng.f64_range(1.0, 1000.0) * 100.0).round() / 100.0;
        let user = fx.seed_user(balance).await;
        fx.ledger
            .refresh_balance_cache(user)
            .await
            .expect("prime cache");

        // Place a few holds, stopping at the first refusal, so `admitted` is
        // the real sum Redis is holding.
        let mut admitted = 0.0;
        for h in 0..rng.i64_range(0, 8) {
            let amount = (rng.f64_range(0.01, balance / 4.0) * 100.0).round() / 100.0;
            if fx
                .ledger
                .hold(user, amount, &format!("existing-{h}"), FIVE_MINUTES)
                .await
                .is_err()
            {
                break;
            }
            admitted += amount;
        }

        let available = balance - admitted;
        let amount = (rng.f64_range(0.01, balance) * 100.0).round() / 100.0;
        let outcome = fx.ledger.hold(user, amount, "new-hold", FIVE_MINUTES).await;

        // The Lua sum is float arithmetic, so treat the knife-edge as
        // admissible rather than asserting on an exact tie.
        let should_admit = available - amount >= -EPSILON;
        match (&outcome, should_admit) {
            (Ok(()), true) => {
                let score = fx
                    .ledger
                    .active_hold_amount(user, "new-hold")
                    .await
                    .expect("read hold")
                    .expect("an admitted hold must be readable");
                assert!(
                    approx(score, amount),
                    "iteration {i}: reserved {score}, asked for {amount}"
                );
            }
            (Err(LedgerError::InsufficientBalance), false) => {}
            _ => panic!(
                "iteration {i}: balance={balance} held={admitted} available={available} \
                 amount={amount} should_admit={should_admit} outcome={outcome:?}"
            ),
        }
    }

    fx.cleanup().await;
}

/// When the balance covers exactly one of two simultaneous holds, exactly one
/// is admitted. The Lua script is the only thing standing between this and a
/// double-spend.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn two_simultaneous_holds_for_the_whole_balance_admit_exactly_one() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;

    for i in 0..20 {
        let balance = 100.0 + f64::from(i);
        let user = fx.seed_user(balance).await;
        fx.ledger
            .refresh_balance_cache(user)
            .await
            .expect("prime cache");

        let a = fx.ledger.clone();
        let b = fx.ledger.clone();
        let (first, second) = tokio::join!(
            tokio::spawn(async move { a.hold(user, balance, "race-a", FIVE_MINUTES).await }),
            tokio::spawn(async move { b.hold(user, balance, "race-b", FIVE_MINUTES).await }),
        );
        let first = first.expect("task a");
        let second = second.expect("task b");

        let admitted = usize::from(first.is_ok()) + usize::from(second.is_ok());
        assert_eq!(
            admitted, 1,
            "iteration {i}: exactly one hold may win (a={first:?} b={second:?})"
        );
        assert!(
            matches!(first, Err(LedgerError::InsufficientBalance))
                || matches!(second, Err(LedgerError::InsufficientBalance)),
            "iteration {i}: the loser must be refused for the right reason \
             (a={first:?} b={second:?})"
        );
    }

    fx.cleanup().await;
}

/// A retried request must not reserve twice. The script returns success
/// without touching the score, so a client retry cannot quietly double the
/// user's exposure.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn re_holding_the_same_request_does_not_reserve_twice() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let user = fx.seed_user(10.0).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");

    fx.ledger
        .hold(user, 4.0, "req-retry", FIVE_MINUTES)
        .await
        .expect("hold");
    fx.ledger
        .hold(user, 4.0, "req-retry", FIVE_MINUTES)
        .await
        .expect("a repeated hold is idempotent, not a failure");

    assert_eq!(fx.hold_members(user).await.len(), 1);
    assert!(approx(
        fx.ledger.get_balance(user).await.expect("balance"),
        6.0
    ));

    fx.cleanup().await;
}

/// A hold appears with the reserved amount as its score, and after either
/// settle or release it is gone from *both* the sorted set and the timestamp
/// hash. A member left in either one would either freeze the balance forever
/// or resurface as an ageless hold.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn a_hold_round_trips_and_vanishes_after_settle_or_release() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let mut rng = Rng::new(0x8010_0002);

    for i in 0..40 {
        let balance = rng.f64_range(10.0, 10_000.0);
        let amount = rng.f64_range(0.01, balance * 0.5);
        let user = fx.seed_user(balance * 10.0 + 1.0).await;
        fx.ledger
            .refresh_balance_cache(user)
            .await
            .expect("prime cache");
        let request_id = format!("req-lifecycle-{i}");

        fx.ledger
            .hold(user, amount, &request_id, FIVE_MINUTES)
            .await
            .expect("hold");

        let reserved = fx
            .ledger
            .active_hold_amount(user, &request_id)
            .await
            .expect("read hold")
            .expect("the hold must be live");
        assert!(
            approx(reserved, amount),
            "iteration {i}: {reserved} != {amount}"
        );
        assert!(fx.hold_timestamp(user, &request_id).await.is_some());

        let via_settle = rng.bool();
        if via_settle {
            let actual = rng.f64_range(0.0, amount);
            fx.ledger
                .settle(user, &request_id, actual)
                .await
                .expect("settle");
        } else {
            fx.ledger.release(user, &request_id).await.expect("release");
        }

        assert_eq!(
            fx.ledger
                .active_hold_amount(user, &request_id)
                .await
                .expect("read"),
            None,
            "iteration {i}: hold survived {}",
            if via_settle { "settle" } else { "release" }
        );
        assert!(
            fx.hold_timestamp(user, &request_id).await.is_none(),
            "iteration {i}: the timestamp outlived the hold"
        );
    }

    fx.cleanup().await;
}

/// Reading the balance reclaims reservations whose owning request died, so a
/// crashed request cannot freeze a user's money past the TTL.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn reading_the_balance_reclaims_expired_holds() {
    let hold_ttl = Duration::from_secs(120);
    let mut fx = Fixture::with_redis(hold_ttl).await;
    let mut rng = Rng::new(0x8010_0003);

    let balance = 10_000.0;
    let user = fx.seed_user(balance).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;

    let mut fresh_total = 0.0;
    for i in 0..5 {
        let amount = rng.f64_range(0.01, 5.0);
        fx.plant_hold(
            user,
            &format!("expired-{i}"),
            amount,
            now - 120 - 60 * (i + 1),
        )
        .await;
    }
    for i in 0..5 {
        let amount = rng.f64_range(0.01, 5.0);
        fresh_total += amount;
        fx.plant_hold(user, &format!("fresh-{i}"), amount, now - i)
            .await;
    }

    let available = fx.ledger.get_balance(user).await.expect("get_balance");
    assert!(
        (available - (balance - fresh_total)).abs() <= 1e-4,
        "expired holds must not count against the balance: got {available}, \
         want {}",
        balance - fresh_total
    );

    let members = fx.hold_members(user).await;
    for i in 0..5 {
        assert!(
            !members.contains(&format!("expired-{i}")),
            "expired hold {i} survived the sweep: {members:?}"
        );
        assert!(
            fx.hold_timestamp(user, &format!("expired-{i}"))
                .await
                .is_none()
        );
        assert!(
            members.contains(&format!("fresh-{i}")),
            "fresh hold {i} was swept: {members:?}"
        );
    }

    fx.cleanup().await;
}

/// The sweep also runs on the admission path, so a new request is not
/// refused because of money frozen by a request that died.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn an_expired_hold_does_not_block_a_new_one() {
    let hold_ttl = Duration::from_secs(60);
    let mut fx = Fixture::with_redis(hold_ttl).await;
    let user = fx.seed_user(10.0).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;

    // The whole balance, frozen by a request that never settled.
    fx.plant_hold(user, "abandoned", 10.0, now - 3600).await;

    fx.ledger
        .hold(user, 9.0, "fresh-request", hold_ttl)
        .await
        .expect("the expired hold must be reclaimed before admission");

    let members = fx.hold_members(user).await;
    assert!(!members.contains(&"abandoned".to_string()), "{members:?}");
    assert!(
        members.contains(&"fresh-request".to_string()),
        "{members:?}"
    );

    fx.cleanup().await;
}

/// After any credit or debit the cached balance is dropped, so the next
/// admission decision sees the new number instead of a stale one.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn moving_money_drops_the_cached_balance() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;

    for credit in [true, false] {
        let user = fx.seed_user(100.0).await;
        fx.ledger
            .refresh_balance_cache(user)
            .await
            .expect("prime cache");
        assert!(fx.balance_cache_exists(user).await, "precondition");

        if credit {
            fx.ledger.credit(user, 10.0, "topup").await.expect("credit");
        } else {
            fx.ledger
                .debit(user, 10.0, "purchase")
                .await
                .expect("debit");
        }

        assert!(
            !fx.balance_cache_exists(user).await,
            "the cached balance must be dropped after a {} ",
            if credit { "credit" } else { "debit" }
        );

        // And the next read re-derives the right number.
        let expected = if credit { 110.0 } else { 90.0 };
        assert!(approx(
            fx.ledger.get_balance(user).await.expect("balance"),
            expected
        ));
    }

    fx.cleanup().await;
}

/// The key lifetime follows the ledger's *configured* hold TTL, not the
/// per-call argument. Both Lua scripts expire holds against that same
/// configured value, and a per-request cutoff would let a short request evict
/// a long one's reservation.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn the_hold_key_lifetime_follows_the_configured_ttl_not_the_argument() {
    let configured = Duration::from_secs(100);
    let mut fx = Fixture::with_redis(configured).await;
    let user = fx.seed_user(100.0).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");

    // A deliberately different argument, to prove it is ignored.
    fx.ledger
        .hold(user, 5.0, "req-ttl", Duration::from_secs(300))
        .await
        .expect("hold");

    let ttl = fx.hold_key_ttl(user).await;
    let want = configured.as_secs() as i64 + 60;
    assert!(
        (ttl - want).abs() <= 10,
        "hold key TTL is {ttl}s, want ~{want}s (configured {}s + margin); \
         a value near 360 would mean the per-call argument leaked through",
        configured.as_secs()
    );

    fx.cleanup().await;
}

/// When the settle transaction fails, the reservation must survive with its
/// original amount. Releasing it early would let the same money be spent
/// twice while the debit is still owed.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn a_failed_settle_keeps_the_reservation() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let user = fx.seed_user(100.0).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");
    let request_id = format!("{FAULT_PREFIX}settle-keeps-hold");

    fx.ledger
        .hold(user, 5.0, &request_id, FIVE_MINUTES)
        .await
        .expect("hold");

    let err = fx.ledger.settle(user, &request_id, 3.0).await.unwrap_err();
    assert!(matches!(err, LedgerError::Db(_)), "{err:?}");

    let still_held = fx
        .ledger
        .active_hold_amount(user, &request_id)
        .await
        .expect("read hold")
        .expect("a rolled-back settle must NOT release the reservation");
    assert!(
        approx(still_held, 5.0),
        "the reserved amount must be unchanged"
    );
    assert!(
        approx(fx.balance(user).await, 100.0),
        "no partial debit may survive"
    );

    fx.cleanup().await;
}

/// A settle that debits the balance also releases the reservation, so the
/// freed headroom is immediately visible to the next request.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn settling_frees_the_reserved_headroom() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let user = fx.seed_user(100.0).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");

    fx.ledger
        .hold(user, 40.0, "req-1", FIVE_MINUTES)
        .await
        .expect("hold");
    assert!(approx(
        fx.ledger.get_balance(user).await.expect("balance"),
        60.0
    ));

    fx.ledger.settle(user, "req-1", 1.0).await.expect("settle");

    // 99 persistent, nothing reserved.
    assert!(approx(
        fx.ledger.get_balance(user).await.expect("balance"),
        99.0
    ));

    fx.cleanup().await;
}

/// A hold placed against a cold cache still works: the miss is detected, the
/// balance is loaded from Postgres, and the script is retried. Without that
/// retry every process restart would refuse the first request of each user.
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn a_cold_cache_is_filled_and_the_hold_retried() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let user = fx.seed_user(50.0).await;

    // Deliberately no refresh_balance_cache: the cache is cold.
    assert!(!fx.balance_cache_exists(user).await, "precondition");

    fx.ledger
        .hold(user, 10.0, "req-cold", FIVE_MINUTES)
        .await
        .expect("a cold cache must be filled, not refused");

    assert!(approx(
        fx.ledger.get_balance(user).await.expect("balance"),
        40.0
    ));

    fx.cleanup().await;
}

// ---------------------------------------------------------------- 租约续期

/// 续租要有持久那一行才成立（首次预扣的时刻从它来），所以这里走
/// `admit_operation` 而不是裸 `hold`。
async fn admit(fx: &Fixture, user: i64, operation: &BillingOperationId, amount: f64) {
    fx.ledger
        .admit_operation(
            &NewOperation {
                operation_id: operation.clone(),
                user_id: user,
                reserved_amount: amount,
                admitted_liability: amount,
                request_fingerprint: "fingerprint".to_owned(),
                client_trace_id: "trace-the-client-saw".to_owned(),
            },
            Some(FIVE_MINUTES),
        )
        .await
        .expect("admit");
}

/// **续租只推时间，不动钱。**
///
/// 一条健康的长流每隔半片租约续一次；如果续租顺手改了 zset 的分数，
/// 那租户的在途负债就会随着流的长度漂移 —— 余额闸门看到的将是一个
/// 与准入时不同的数。
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn renewing_a_lease_moves_the_deadline_and_leaves_the_amount_alone() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let user = fx.seed_user(100.0).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");
    let operation = BillingOperationId::mint();
    admit(&fx, user, &operation, 7.5).await;

    let reserved = fx
        .ledger
        .active_hold_amount(user, operation.as_str())
        .await
        .expect("read hold")
        .expect("the hold must be live");

    // 把时间戳倒推到「快过期了」，续租之后它必须回到现在。
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_secs() as i64;
    let stale = now - FIVE_MINUTES.as_secs() as i64 + 5;
    fx.plant_hold(user, operation.as_str(), reserved, stale)
        .await;
    assert_eq!(
        fx.hold_timestamp(user, operation.as_str()).await.as_deref(),
        Some(stale.to_string().as_str()),
        "precondition",
    );

    for round in 0..2 {
        let renewal = fx
            .ledger
            .renew_lease(user, &operation)
            .await
            .expect("a live hold renews");
        assert!(
            approx(renewal.reserved_amount, reserved),
            "第 {round} 次续租改了金额：{} != {reserved}",
            renewal.reserved_amount,
        );
        assert!(
            approx(
                fx.ledger
                    .active_hold_amount(user, operation.as_str())
                    .await
                    .expect("read hold")
                    .expect("still live"),
                reserved,
            ),
            "第 {round} 次续租之后 zset 的分数变了",
        );
        let moved: i64 = fx
            .hold_timestamp(user, operation.as_str())
            .await
            .expect("timestamp survives")
            .parse()
            .expect("a numeric timestamp");
        assert!(
            moved > stale,
            "第 {round} 次续租没把到期时刻往后推：{moved} 不晚于 {stale}",
        );
    }

    // 续租不是第二次准入：可用余额还是「余额 - 那一笔预留」。
    assert!(approx(
        fx.ledger.get_balance(user).await.expect("balance"),
        100.0 - reserved,
    ));

    fx.cleanup().await;
}

/// 续租**绝不凭空造出一笔预留**。一个不存在（或已终结）的操作续租失败，
/// 而不是悄悄 `ZADD` 一个新成员 —— 那会是一笔谁也不认识的在途负债。
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn renewing_an_unknown_operation_does_not_conjure_a_hold() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let user = fx.seed_user(100.0).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");

    let stranger = BillingOperationId::mint();
    assert!(matches!(
        fx.ledger.renew_lease(user, &stranger).await,
        Err(LedgerError::HoldNotFound),
    ));
    assert!(fx.hold_members(user).await.is_empty(), "凭空多了一个成员");

    // 已经终结的操作同理：它的钱已经结清，续租没有任何意义。
    let settled = BillingOperationId::mint();
    admit(&fx, user, &settled, 3.0).await;
    fx.ledger
        .release_once(&settled, user)
        .await
        .expect("release");
    assert!(matches!(
        fx.ledger.renew_lease(user, &settled).await,
        Err(LedgerError::HoldNotFound),
    ));

    fx.cleanup().await;
}

/// 硬顶到了就不再续：一条永不结束的流不许永久冻结余额。
///
/// 被拒之后金额**仍然是原来那个**——预留照旧活到它剩下的 TTL，
/// 那一行留给对账，而不是当场被撕掉。
#[tokio::test]
#[ignore = "requires a local Redis and Postgres (set GW_TEST_REDIS_URL, GW_TEST_DATABASE_URL)"]
async fn a_lease_past_its_maximum_duration_is_refused_but_keeps_its_amount() {
    let mut fx = Fixture::with_redis(FIVE_MINUTES).await;
    let user = fx.seed_user(100.0).await;
    fx.ledger
        .refresh_balance_cache(user)
        .await
        .expect("prime cache");
    let operation = BillingOperationId::mint();
    admit(&fx, user, &operation, 4.25).await;

    // 一个短得离谱的硬顶，等价于「这笔预留已经活了很久」。
    let capped = fx
        .ledger
        .clone()
        .with_max_hold_duration(Duration::from_millis(20));
    tokio::time::sleep(Duration::from_millis(80)).await;

    assert!(matches!(
        capped.renew_lease(user, &operation).await,
        Err(LedgerError::LeaseExpired),
    ));
    assert!(approx(
        fx.ledger
            .active_hold_amount(user, operation.as_str())
            .await
            .expect("read hold")
            .expect("被拒的续租不该顺手撕掉预留"),
        4.25,
    ));

    fx.cleanup().await;
}
