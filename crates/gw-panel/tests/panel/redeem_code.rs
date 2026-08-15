//! 兑换码：一张码只能被兑一次，且失败的兑换要把码放回去。
//!
//! 对应原实现的 redeem_persistence 测试。

use crate::common::{fresh_db, redeem_code_state, seed_redeem_code, seed_user};
use gw_panel::commerce::redeem::{Claim, claim_code, release_claim};

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_first_claim_wins_and_the_second_is_told_it_is_used() {
    let pool = fresh_db("redeem_single_use").await;
    let user = seed_user(&pool, "a@example.com", 0.0).await;
    let other = seed_user(&pool, "b@example.com", 0.0).await;
    let code = seed_redeem_code(&pool, "RDM-TESTCODE", 10.0).await;

    assert_eq!(
        claim_code(&pool, code, user, "a@example.com")
            .await
            .expect("claim"),
        Claim::Won
    );
    assert_eq!(
        claim_code(&pool, code, other, "b@example.com")
            .await
            .expect("claim"),
        Claim::AlreadyUsed,
        "第二个人必须被告知已被使用"
    );

    let (status, used_by_id) = redeem_code_state(&pool, code).await;
    assert_eq!(status, "used");
    assert_eq!(used_by_id, Some(user), "认领人必须是第一个赢家");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn concurrent_claims_produce_exactly_one_winner() {
    let pool = fresh_db("redeem_concurrent").await;
    let code = seed_redeem_code(&pool, "RDM-RACECODE", 10.0).await;
    let mut users = Vec::new();
    for i in 0..8 {
        users.push(seed_user(&pool, &format!("u{i}@example.com"), 0.0).await);
    }

    let mut handles = Vec::new();
    for user in users {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            claim_code(&pool, code, user, "x").await
        }));
    }
    let mut winners = 0;
    for handle in handles {
        if handle.await.expect("join").expect("claim") == Claim::Won {
            winners += 1;
        }
    }
    assert_eq!(winners, 1, "八个人抢一张码，只能有一个赢");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn releasing_my_own_claim_makes_the_code_redeemable_again() {
    // 入账失败时走的就是这条路：码必须回到 unused，否则用户白丢一张码。
    let pool = fresh_db("redeem_release").await;
    let user = seed_user(&pool, "a@example.com", 0.0).await;
    let code = seed_redeem_code(&pool, "RDM-RELEASE", 10.0).await;

    claim_code(&pool, code, user, "a@example.com")
        .await
        .expect("claim");
    release_claim(&pool, code, user).await.expect("release");

    let (status, used_by_id) = redeem_code_state(&pool, code).await;
    assert_eq!(status, "unused");
    assert_eq!(used_by_id, None);
    assert_eq!(
        claim_code(&pool, code, user, "a@example.com")
            .await
            .expect("claim"),
        Claim::Won,
        "放回去之后必须能再兑"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn releasing_never_steals_back_someone_elses_claim() {
    // `used_by_id` 进 WHERE 的全部理由：没有它，A 的失败回滚会把 B 刚抢到的
    // 认领擦掉，同一张码就被两个人各兑一次。
    let pool = fresh_db("redeem_release_scoped").await;
    let winner = seed_user(&pool, "winner@example.com", 0.0).await;
    let loser = seed_user(&pool, "loser@example.com", 0.0).await;
    let code = seed_redeem_code(&pool, "RDM-SCOPED", 10.0).await;

    claim_code(&pool, code, winner, "winner@example.com")
        .await
        .expect("claim");
    // 败者试图回滚 —— 必须什么都不发生。
    release_claim(&pool, code, loser).await.expect("release");

    let (status, used_by_id) = redeem_code_state(&pool, code).await;
    assert_eq!(status, "used", "别人的认领不能被撤销");
    assert_eq!(used_by_id, Some(winner));
}
