//! 余额流水：翻页之后累计余额仍然对得上。
//!
//! 对应原实现的 balance_history 测试。这条测试的价值在于：累计值是在
//! **全集**上算的，而返回的只有一页 —— 一个把窗口函数写成"先 LIMIT 再累加"的
//! 实现在第一页上完全正确，只在第二页开始出错。

use crate::common::{fresh_db, seed_balance_log, seed_user};
use chrono::{Duration, Utc};
use gw_panel::commerce::balance::history_page;

/// 铺 `n` 条金额为 +1 的流水，时间严格递增。
async fn seed_series(pool: &sqlx::PgPool, user: i64, n: i64) {
    let base = Utc::now() - Duration::seconds(n * 10);
    for i in 0..n {
        seed_balance_log(
            pool,
            user,
            1.0,
            "credit",
            &format!("seed:{i}"),
            base + Duration::seconds(i * 10),
        )
        .await;
    }
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_running_balance_stays_correct_across_pages() {
    let pool = fresh_db("balance_history_pages").await;
    let user = seed_user(&pool, "history@example.com", 0.0).await;
    let total_rows = 25_i64;
    seed_series(&pool, user, total_rows).await;

    // 列表是倒序的，所以第一页最上面那条的累计值应当等于总条数（每条 +1）。
    let (page1, total) = history_page(&pool, user, "", 1, 10).await.expect("page 1");
    assert_eq!(total, total_rows);
    assert_eq!(page1.len(), 10);
    assert!((page1[0].balance_after - total_rows as f64).abs() < 1e-9);

    // 第二页接着往下：第 11 条的累计值是 15。
    let (page2, _) = history_page(&pool, user, "", 2, 10).await.expect("page 2");
    assert_eq!(page2.len(), 10);
    assert!(
        (page2[0].balance_after - (total_rows as f64 - 10.0)).abs() < 1e-9,
        "第二页的累计值必须接着第一页，而不是从头再算：{}",
        page2[0].balance_after
    );

    // 最后一页是短的，最底下那条的累计值就是它自己。
    let (page3, _) = history_page(&pool, user, "", 3, 10).await.expect("page 3");
    assert_eq!(page3.len(), 5);
    let last = page3.last().expect("last row");
    assert!((last.balance_after - 1.0).abs() < 1e-9);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn before_plus_amount_equals_after_on_every_row() {
    // 前端画的是"变动前 → 变动后"，这条不变量比具体数值更重要。
    let pool = fresh_db("balance_history_invariant").await;
    let user = seed_user(&pool, "history@example.com", 0.0).await;
    let base = Utc::now() - Duration::seconds(100);
    for (i, amount) in [5.0_f64, -2.0, 7.5, -0.5, 1.0].iter().enumerate() {
        seed_balance_log(
            &pool,
            user,
            *amount,
            "credit",
            "seed",
            base + Duration::seconds(i as i64 * 10),
        )
        .await;
    }

    let (rows, _) = history_page(&pool, user, "", 1, 100).await.expect("page");
    for row in &rows {
        assert!(
            (row.balance_before + row.amount - row.balance_after).abs() < 1e-9,
            "行 {} 破坏了 before + amount == after",
            row.id
        );
    }
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_kind_filter_narrows_both_the_page_and_the_total() {
    let pool = fresh_db("balance_history_filter").await;
    let user = seed_user(&pool, "history@example.com", 0.0).await;
    let base = Utc::now() - Duration::seconds(100);
    seed_balance_log(&pool, user, 5.0, "credit", "a", base).await;
    seed_balance_log(
        &pool,
        user,
        -2.0,
        "debit",
        "b",
        base + Duration::seconds(10),
    )
    .await;
    seed_balance_log(
        &pool,
        user,
        3.0,
        "credit",
        "c",
        base + Duration::seconds(20),
    )
    .await;

    let (all, total_all) = history_page(&pool, user, "", 1, 100).await.expect("all");
    assert_eq!(total_all, 3);
    assert_eq!(all.len(), 3);

    let (credits, total_credits) = history_page(&pool, user, "credit", 1, 100)
        .await
        .expect("credits");
    assert_eq!(total_credits, 2, "total 必须跟着过滤走");
    assert_eq!(credits.len(), 2);
    assert!(credits.iter().all(|row| row.kind == "credit"));
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_user_never_sees_another_users_ledger() {
    let pool = fresh_db("balance_history_isolation").await;
    let mine = seed_user(&pool, "mine@example.com", 0.0).await;
    let theirs = seed_user(&pool, "theirs@example.com", 0.0).await;
    seed_balance_log(&pool, theirs, 100.0, "credit", "theirs", Utc::now()).await;

    let (rows, total) = history_page(&pool, mine, "", 1, 100).await.expect("page");
    assert_eq!(total, 0);
    assert!(rows.is_empty());
}
