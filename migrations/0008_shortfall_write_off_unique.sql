-- 一条欠款最多只能被注销一次。
-- 零额注销写的是一条 reference 为 `shortfall_resolve:<requestID>:<debitLogID>`
-- 的 credit 行，这个字符串已经唯一标识了那条欠款行，所以幂等直接钉在库里，
-- 不靠应用层「先查再写」（那有竞态窗口）。
-- 分区条件必须同时钉住 `type = 'credit'`：`balance_logs.reference` 在热路径上
-- 就是 request id，而它认客户端传来的 `X-Trace-Id`（见 gw-proxy 的 trace_id_from）。
-- 只按 reference 前缀分区的话，客户端发一个 `X-Trace-Id: shortfall_resolve:…`
-- 就能让自己的 hold / settle 审计行撞进这个唯一索引 —— settle 事务整个回滚、
-- 余额一分不扣，等于无限白嫖；而且超预留结算本来就会在同一事务里写两条
-- reference 相同的 settle 行，必然自撞。注销行只可能是 credit，收窄到 credit
-- 既堵死这条路，又不影响幂等。
CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_shortfall_resolve
    ON balance_logs (reference)
    WHERE type = 'credit' AND reference LIKE 'shortfall_resolve:%';
