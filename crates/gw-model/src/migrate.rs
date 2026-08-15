//! 迁移入口 —— 把既有 schema 推到最新版本。
//!
//! SQL 放在 `rust/migrations/`，由 [`MIGRATOR`] 在**编译期**嵌进二进制
//! （`sqlx::migrate!`），所以部署时不需要额外带 .sql 文件，文件名/版本号写错也是
//! 编译错误而不是运行时错误。
//!
//! 迁移必须对「已经建过表的现有库」幂等 —— 全部语句都是
//! `CREATE TABLE IF NOT EXISTS` / `ADD COLUMN IF NOT EXISTS` /
//! `CREATE INDEX IF NOT EXISTS`。这条性质由 `tests.rs` 里的 `migrations_are_idempotent`
//! 卡住，别在新迁移里写裸的 `CREATE TABLE`。

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

/// `rust/migrations/*.sql` 的编译期快照。
///
/// 版本号即文件名前缀（0001、0002…），已应用的版本记在 `_sqlx_migrations` 表里，
/// 重复启动只会跑没跑过的那些。
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// 把 schema 推到最新。
pub async fn run(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}

#[cfg(test)]
mod tests;
