# 004 — Replace leftover CPA Gateway copy on every-session surfaces

- **Status**: TODO
- **Commit**: 34d21b3
- **Severity**: HIGH
- **Category**: Maintainability & architecture
- **Rule**: Beyond the scan
- **Estimated scope**: 7 files (user-facing). Admin leftovers optional in the same pass.

## Problem

Home and docs already say AI-GateWay. Login, chrome, and the dashboard still say CPA Gateway. That is the first thing a user sees after the access-guide work.

    // src/pages/public/LoginPage.tsx:35 — current
    <p className="text-gray-500 dark:text-gray-400">登入 CPA Gateway 计费管理平台</p>

    // src/pages/public/AuthLayout.tsx:21 — current
    <img src="/icon.svg" alt="CPA Gateway" className="w-14 h-14 rounded-2xl shadow-sm" />
    // src/pages/public/AuthLayout.tsx:31
    &copy; {new Date().getFullYear()} CPA Gateway. All rights reserved.

    // src/shared/components/ui/Logo.tsx:124 — current
    CPA Gateway

    // src/shared/components/layout/Header.tsx:175 — current
    <img src="/icon.svg" alt="CPA Gateway" className="w-7 h-7 shrink-0 rounded-lg" />

    // src/features/user-dashboard/components/QuickIntegrationPanel.tsx:18 — current
    ? '配置 OpenAI 兼容客户端（如 Cursor、Cline、aider）连接到 CPA Gateway 代理池：'

    // src/features/user-api-keys/components/ApiKeysTable.tsx:19 — current
    if (key.startsWith("sk-cpa-")) return "sk-cpa-****"

    // src/pages/user/billing/RedeemPage.tsx:111 — current
    placeholder="例如：CPA-a1b2c3d4..."

Product rule: brand is AI-GateWay, keys are `agw-` only. No `cpa-` accept or aliases.

## Target

    // LoginPage.tsx:35
    <p className="text-gray-500 dark:text-gray-400">登入 AI-GateWay 计费管理平台</p>

    // AuthLayout.tsx
    <img src="/icon.svg" alt="AI-GateWay" className="w-14 h-14 rounded-2xl shadow-sm" />
    &copy; {new Date().getFullYear()} AI-GateWay. All rights reserved.

    // Logo.tsx:124
    AI-GateWay

    // Header.tsx:175
    <img src="/icon.svg" alt="AI-GateWay" className="w-7 h-7 shrink-0 rounded-lg" />

    // QuickIntegrationPanel.tsx — replace every "CPA Gateway" with "AI-GateWay"

    // ApiKeysTable.tsx
    if (key.startsWith("agw-") || key.startsWith("sk-agw-")) return "agw-****"
    // do not keep an sk-cpa- branch

    // RedeemPage.tsx
    placeholder="例如：AGW-a1b2c3d4..."

Also fix the same leftover on admin pages in this pass if you touch the tree: `AdminUsersPage.tsx:89,114`, `AdminProxyAmpcodePage.tsx:351`, `AdminProxyConfigPage.tsx:327`, `ampcodeUpstreamTest.ts:33` (`AI-GateWay-Upstream-Probe/1.0`). Do not rename `cpa-models-registry.json` here unless that file is already moved on the branch.

## Repo conventions to follow

- Home already uses `AI-GateWay` (`src/pages/public/HomePage.tsx:34`).
- Docs tests already forbid `CPA Gateway` / `cpa-` (`src/pages/docs/docs.test.tsx:79`).

## Steps

1. Replace the every-session strings listed in Target.
2. Update `ApiKeysTable` mask to `agw-` only.
3. Optionally sweep admin copy in the same commit.
4. Do not edit `crates/gw-panel/src/upstream/oauth/**` unless a visible string there is still `CPA`.

## Boundaries

- Do NOT add a compatibility alias for `cpa-` keys.
- Do NOT change payment or auth behavior.
- STOP if PR #7 (`chore/purge-cpa`) is already on this commit; then only fill gaps this branch still has.

## Verification

- **Mechanical**: `rg -n "CPA Gateway|cpa_token|sk-cpa-" frontend/src` is empty except tests that assert absence and the models registry filename if still present.
- **Behavior check**: Open `/`, `/login`, `/dashboard`, `/keys`. Header, login card, and key mask say AI-GateWay / `agw-`.
- **Done when**: no user-visible CPA brand on those routes.
