# 05 — 兑换、订阅购买、注册入账

## 完成状态（工作树已验证，未部署）

- 兑换：`claim` + `credit_tx("redeem:{CODE}")` 同一事务；失败码仍 unused。used 无流水时对本人补入一次。
- 订阅购买：`debit_tx` + INSERT 同一事务；创建失败回滚，无补偿流水。
- 注册：INSERT users + `credit_tx("initial_register_credit")` 同一事务；邮箱冲突不入账。
- 迁移 `0013_control_plane_sibling_credits.sql`。未 unique 全部 debit。
- 支付 02 的 paid-无-credit 补入分支保留（未部署，历史修复测试仍绿）。退款 approve 仍不动余额。

长期合同如下。

依赖：02 的 `credit_tx` / `debit_tx`。两者可参加调用方事务；当前只有 `payment_order:` credit 自动去重，普通 credit/debit 的 reference 仍为审计描述。05 必须在业务行状态与稳定身份上完成重复请求处理，不能假定 debit_tx 已提供幂等。

## 合同

与支付同一类：业务行状态与资金在同一 SQL 事务提交。进程死在两事务之间时，重试不得造成“码已用但没钱 / 钱扣了没订阅 / 用户建了没送注册金”，也不得双入账。

## 接缝

| 路径 | 现状 | 目标 |
|---|---|---|
| 兑换 | 先 claim `used` 再 `credit`；仅 Err 时 `release_claim` | `BEGIN; claim; credit_tx(..., "redeem:{CODE}"); COMMIT` |
| 订阅购买 | 先 `debit` 再 INSERT；失败才补偿 credit；HTTP 重试用新 UUID | `BEGIN; debit_tx(..., 稳定 reference); INSERT subscriptions; COMMIT` |
| 注册送 $1 | INSERT user 后独立 credit；email 冲突则永远拿不到 | `BEGIN; INSERT users; credit_tx(..., "initial_register_credit"); COMMIT` |

再加分区唯一（`IF NOT EXISTS`）：

```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_redeem_credit
  ON balance_logs (reference)
  WHERE type = 'credit' AND reference LIKE 'redeem:%';

CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_initial_register_credit
  ON balance_logs (user_id, reference)
  WHERE type = 'credit' AND reference = 'initial_register_credit';
```

订阅 debit 的 reference 用已有 UUID 即可；真正防双购的是**同一事务里的订阅行 + 用户锁**，不是全局 unique debit。不要 unique 全部 debit。

删除成功路径上的补偿式 `release_claim` / 购买失败后再 credit。补偿不是崩溃安全。

核对 02 的脚手架：若 paid-无-credit 已回填，删除支付里的“从 paid 补入”特殊分支。

## 人能跑什么

现有 `#[ignore]`：`redeem_code.rs`、`subscription_purchase.rs`，再加崩溃种子（`used` 无 credit、注册用户余额 0 无 `initial_register_credit` 流水）。

## 验证

- 兑换失败 credit → 码仍 `unused`。
- 购买过程中事务失败 → 无订阅行、余额不变、无补偿流水。
- 注册冲突 → 不产生第二笔跨用户同字面量 credit（靠 `(user_id, reference)`）。
- 退款 approve、管理员赠送/撤销订阅仍不动余额。

## 实现者可自行决定

`purchase_subscription` 的 `create` 闭包是否改成 `FnOnce(&mut Transaction<'_>)`。订阅稳定 reference 的字符串格式（需避开 0008 的 `shortfall_resolve:` 与 `payment_order:`）。

## 必须保持绿色

支付 02 测试；shortfall 核销唯一索引。
