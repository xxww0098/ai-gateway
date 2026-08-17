-- 0004_model_capabilities.sql —— 模型目录能力字段，供 GET /v1/models 与
-- DeepSeek Harness AGW-Oauth 插件的 resolveModel() 使用。
--
-- 缺省为 NULL：没有填过能力的行不会被猜出 context / 模态 / 思考档位。

ALTER TABLE IF EXISTS "model_catalog_entries"
    ADD COLUMN IF NOT EXISTS "capabilities" jsonb;
