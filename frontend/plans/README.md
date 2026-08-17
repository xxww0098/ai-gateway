# React plans

Scan: `npx react-doctor@latest` on `frontend/` @ `34d21b3` (`feat/access-guide`). Score **57 Critical**, 331 warnings, 82 files.

Most of those warnings are Radix/shadcn label noise, `transition-all`, giant admin dialogs, and cold-path memo. The plans below are the ones that survived a file:line pass.

## Order

1. **002** — stop white-screen on bad `localStorage` (frontend only, do first).
2. **001** — HttpOnly session cookie (needs `gw-panel`; do next if you want the security finding gone).
3. **003** — payment success fires once (finance).
4. **004** — leftover CPA copy on login/chrome/dashboard. Conflicts with open PR #7 (`chore/purge-cpa`); if you merge #7 first, re-read and only fill gaps.
5. **005** — login/register labels (every session).
6. **006** — create-key dialog labels + stuck loading.
7. **008** — dashboard admin/user cache split (do with 002; logout clear).
8. **009** — Home theme override.
9. **007** — layout ErrorBoundary.
10. **010** — keys copy timeout + model-list race.

001 and 002 share `auth_store.ts`. Execute 002 first, then 001 on top (001 deletes JWT persistence).
008 is independent of 001. Do 008 before any dashboard UX pass.

004 and #7 overlap. Do not run both blindly.

## Status

| Plan | Severity | Status |
|------|----------|--------|
| [001-http-only-session-cookie.md](001-http-only-session-cookie.md) | HIGH | TODO |
| [002-versioned-auth-user-cache.md](002-versioned-auth-user-cache.md) | HIGH | TODO |
| [003-payment-success-once.md](003-payment-success-once.md) | HIGH | TODO |
| [004-replace-cpa-brand-copy.md](004-replace-cpa-brand-copy.md) | HIGH | TODO |
| [005-login-register-labels.md](005-login-register-labels.md) | MEDIUM | TODO |
| [006-create-key-dialog-a11y-loading.md](006-create-key-dialog-a11y-loading.md) | MEDIUM | TODO |
| [007-layout-error-boundary.md](007-layout-error-boundary.md) | MEDIUM | TODO |
| [008-dashboard-query-keys-by-role.md](008-dashboard-query-keys-by-role.md) | HIGH | TODO |
| [009-home-theme-override.md](009-home-theme-override.md) | MEDIUM | TODO |
| [010-keys-copy-and-models-race.md](010-keys-copy-and-models-race.md) | MEDIUM | TODO |
