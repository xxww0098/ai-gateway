-- 兑换码与注册金各入账一次（billing-hardening slice 05）。
-- 不 unique 全部 debit：订阅购买仍用每次尝试的 UUID 作审计 reference。
CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_redeem_credit
    ON balance_logs (reference)
    WHERE type = 'credit' AND reference LIKE 'redeem:%';

CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_initial_register_credit
    ON balance_logs (user_id, reference)
    WHERE type = 'credit' AND reference = 'initial_register_credit';
