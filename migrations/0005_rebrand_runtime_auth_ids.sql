-- 0005_rebrand_runtime_auth_ids.sql —— 数据迁移：config-seeded 凭据改名。
--
-- 这是数据迁移（非 schema），只影响 config.yaml 播种的运行时凭据行，
-- 不新增/删除任何表、列或索引，与 pg_dump schema diff 契约无冲突。
--
-- 旧运行时凭据 id 形如 `cpa-gateway-<provider>`，对应 attributes jsonb 里的
-- `source = 'cpa-gateway-config'`。这里把两者一起改成 `ai-gateway-<provider>` 与
-- `ai-gateway-config`，对齐 gw-authcore::runtime 的新常量。
--
-- 前缀 `cpa-gateway-` 长 12 个字符；`right(id, -length('cpa-gateway-'))` 取前缀之后的
-- provider 段，再拼上 `ai-gateway-`（11 字符）得到新 id，长度差 1 由前缀本身消化。
--
-- 幂等性：WHERE 只匹配 id 以 `cpa-gateway-` 开头的行；第一遍把它们全部改名后，
-- 第二遍没有任何行满足该前缀，UPDATE 影响 0 行。
UPDATE auth_records
SET id         = 'ai-gateway-' || right(id, -length('cpa-gateway-')),
    attributes = jsonb_set(COALESCE(attributes, '{}'::jsonb), '{source}', '"ai-gateway-config"')
WHERE id LIKE 'cpa-gateway-%';

-- channel_policies.auth_id 与 auth_records.id 对齐；漏改会导致策略 miss，回退到
-- 「enabled + weight=1」默认，打乱负载均衡。
UPDATE channel_policies
SET auth_id = 'ai-gateway-' || right(auth_id, -length('cpa-gateway-'))
WHERE auth_id LIKE 'cpa-gateway-%';
