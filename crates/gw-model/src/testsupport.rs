//! 连库测试的公共脚手架。
//!
//! 所有用到它的测试都标了 `#[ignore]`（规范 2.9：要么 fail-loud，要么 ignore，
//! 绝不允许"读不到环境变量就 return"）。跑法：
//!
//! ```bash
//! DATABASE_URL=postgres://postgres@127.0.0.1:5432/postgres \
//!   cargo test -p gw-model -- --ignored
//! ```
//!
//! 每个测试用一个**独占的、以测试名命名的数据库**：先 DROP 再 CREATE，所以上一次
//! 跑失败留下的残骸不会影响下一次。

use std::str::FromStr as _;

use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;

const HOWTO: &str = "连库测试需要 DATABASE_URL，例如：\n  \
     DATABASE_URL=postgres://postgres@127.0.0.1:5432/postgres cargo test -p gw-model -- --ignored";

/// 建一个空库并连上去。`tag` 会成为库名的一部分，每个测试用不同的 tag。
pub(crate) async fn fresh_db(tag: &str) -> PgPool {
    let url = std::env::var("DATABASE_URL").expect(HOWTO);
    let opts = PgConnectOptions::from_str(&url).expect("DATABASE_URL 不是合法的 Postgres 连接串");

    let admin = PgPool::connect_with(opts.clone())
        .await
        .expect("连不上 DATABASE_URL 指向的库");
    let name = format!("gw_model_test_{tag}");
    // 库名来自测试里写死的 tag，不是外部输入；仍然只允许 [a-z0-9_]。
    assert!(
        tag.chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
        "tag 只能是小写字母/数字/下划线"
    );
    for stmt in [
        format!(r#"DROP DATABASE IF EXISTS "{name}""#),
        format!(r#"CREATE DATABASE "{name}""#),
    ] {
        sqlx::query(&stmt)
            .execute(&admin)
            .await
            .unwrap_or_else(|e| panic!("{stmt} 失败: {e}"));
    }
    admin.close().await;

    PgPool::connect_with(opts.database(&name))
        .await
        .expect("连不上刚建好的测试库")
}
