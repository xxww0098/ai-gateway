-- users.concurrency = 0 表示「沿用限流器配置的 max_concurrent」
-- （gw-infra effective_limits_for：正值覆盖默认值，<= 0 回退到配置默认 10）。
-- 该列此前默认 1，注册路径也硬编码 1，等于每个租户只准一个在途请求 ——
-- Agent / IDE 插件天然并发调用，准入阶段直接 429。
--
-- 没有任何管理面写路径会碰这一列，库里每一个 `1` 都只能来自旧默认值，
-- 把它重置成「沿用配置」不可能覆盖运维的人工设定。
ALTER TABLE users ALTER COLUMN concurrency SET DEFAULT 0;
UPDATE users SET concurrency = 0 WHERE concurrency = 1;
