# migrations

`gw-model` 用 `sqlx::migrate!("../../migrations")` 在**编译期**把这里的 `.sql`
嵌进二进制，启动时由 `gw_model::run_migrations(&pool)` 应用。

| 文件 | 内容 | 对应关系 |
| --- | --- | --- |
| `0001_init.sql` | 22 张表的 `CREATE TABLE IF NOT EXISTS` | 既有库建表脚本 |
| `0002_columns.sql` | 每张表每个非主键列的 `ADD COLUMN IF NOT EXISTS` | 既有库补列行为 |
| `0003_indexes.sql` | 自动索引 + 7 个手写索引 | 既有库索引全集 |

## 两条不可违反的规矩

1. **每条语句都必须带 `IF NOT EXISTS`**。这份迁移的头号使用场景是「指向一个已有
   既有 schema 的现有库」，那时 `_sqlx_migrations` 是空的，0001 会从头
   跑一遍 —— 不幂等就直接起不来。
   单元测试 `migrate::tests::migrations_are_idempotent` 逐行卡这条，
   同一个测试还禁止出现 `DROP` / `TRUNCATE`。
2. **列名以既有 schema 为准，不"修正"**。几个反直觉但必须保留的：

   | 字段（历史命名） | 列名 |
   | --- | --- |
   | `OAuthSession`（模型名） | `o_auth_sessions` |
   | `InputPricePer1M` | `input_price_per1_m` |
   | `ApiKeyID` | `api_key_id` |
   | `IPAddress` | `ip_address` |

   另外历史建库脚本有若干列定义写成了 `size=32`（等号而非冒号），解析
   不到，于是那些列建出来是 `text` 而不是 `varchar(n)`。这里照抄 `text`。

## 怎么验证它和既有 schema 一致

需要一个能跑的 Postgres。下面这套是这份迁移当初的验收方式，改动迁移后照做一遍：

```bash
# 1) 准备一个按既有 schema 建出来的基线库
pg_dump --schema-only --no-owner --no-privileges base_schema > /tmp/base_schema.sql

# 2) 让本目录的迁移建另一个库
psql -d rust_schema -v ON_ERROR_STOP=1 -f 0001_init.sql -f 0002_columns.sql -f 0003_indexes.sql
pg_dump --schema-only --no-owner --no-privileges rust_schema > /tmp/rust_schema.sql

# 3) 逐字比较（不含注释行）
diff <(grep -v '^--' /tmp/base_schema.sql) <(grep -v '^--' /tmp/rust_schema.sql)
```

当前版本 diff 为空：表、列、类型、DEFAULT、NOT NULL、索引（含 GIN 与 `DESC` 复合
索引）、序列、主键约束名全部一致。

跑 `gw-model` 里那几个连库测试可以一次性覆盖同样的场景（含「老库缺列」）：

```bash
DATABASE_URL=postgres://postgres@127.0.0.1:5432/postgres \
  cargo test -p gw-model -- --ignored
```
