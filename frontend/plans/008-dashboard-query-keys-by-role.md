# 008 — Separate admin and user dashboard query keys

- **Status**: DONE
- **Commit**: 34d21b3
- **Severity**: HIGH
- **Category**: Bugs & correctness
- **Rule**: Beyond the scan
- **Estimated scope**: 3 files (`query-keys.ts`, `user-dashboard/hooks.ts`, logout in `Sidebar.tsx`)

## Problem

Admin and user dashboard queries share the same React Query keys. `enabled` only controls who fetches; the cache entry is global. Logout does not `queryClient.clear()`. A later user session (same tab, 5-minute `staleTime`) can render the previous admin stats, trend, and model mix.

    // src/features/user-dashboard/hooks.ts:31 — current
    const adminQuery = useQuery({
      queryKey: queryKeys.dashboard.stats(),
      queryFn: async () => { /* fetchAdminDashboard */ },
      enabled: isAdmin,
    })
    const userQuery = useQuery({
      queryKey: queryKeys.dashboard.stats(),
      queryFn: async () => { /* fetchUserProfile + usage */ },
      enabled: !isAdmin,
    })

    // src/features/user-dashboard/hooks.ts:90 — current
    return useQuery({
      queryKey: queryKeys.dashboard.trend(days),
      queryFn: () => isAdmin ? fetchAdminUsageTrend(days) : fetchUserUsageTrend(days),
    })

    // src/features/user-dashboard/hooks.ts:105 — current
    queryKey: [...queryKeys.dashboard.all(), 'models'] as const,
    queryFn: () => isAdmin ? fetchAdminModelStats() : fetchUserModelStats(),

    // src/shared/components/layout/Sidebar.tsx:316 — current
    logout()
    // no queryClient.clear()

## Target

    // src/shared/api/query-keys.ts — target
    dashboard: {
      all: () => ['dashboard'] as const,
      stats: (role: 'admin' | 'user') => ['dashboard', 'stats', role] as const,
      trend: (days: number, role: 'admin' | 'user') => ['dashboard', 'trend', days, role] as const,
      models: (role: 'admin' | 'user') => ['dashboard', 'models', role] as const,
      recentUsage: () => ['dashboard', 'recentUsage'] as const,
    },

    // hooks.ts — target
    const role = isAdmin ? 'admin' : 'user'
    const adminQuery = useQuery({
      queryKey: queryKeys.dashboard.stats('admin'),
      queryFn: /* unchanged */,
      enabled: isAdmin,
    })
    const userQuery = useQuery({
      queryKey: queryKeys.dashboard.stats('user'),
      queryFn: /* unchanged */,
      enabled: !isAdmin,
    })
    useQuery({
      queryKey: queryKeys.dashboard.trend(days, role),
      queryFn: () => isAdmin ? fetchAdminUsageTrend(days) : fetchUserUsageTrend(days),
    })
    useQuery({
      queryKey: queryKeys.dashboard.models(role),
      queryFn: () => isAdmin ? fetchAdminModelStats() : fetchUserModelStats(),
    })

    // Sidebar logout — target
    await serverLogout().catch(() => {})
    logout()
    queryClient.clear()

Keep existing server logout if it is already there; only add `queryClient.clear()` after the store logout.

## Repo conventions to follow

- Query keys already live in `src/shared/api/query-keys.ts`.
- Imitate `queryKeys.orders.list(params)` (role is just another discriminator).

## Steps

1. Add `role` to `stats` / `trend` / `models` key factories. Update every caller (only `user-dashboard/hooks.ts` if grep agrees).
2. Call `queryClient.clear()` on explicit logout in `Sidebar.tsx`.
3. Do not change fetch URLs or chart components.

## Boundaries

- Do NOT change dashboard JSON shapes.
- Do NOT add dependencies.
- STOP if keys already include role.

## Verification

- **Mechanical**: `rg "dashboard.stats\\(\\)" frontend/src` is empty. Typecheck.
- **Behavior check**: Log in as admin, open `/dashboard`, note user count. Log out, log in as a normal user without a full reload if you can. Confirm the hero/charts show that user’s numbers, not the admin totals. Log out again and confirm the next login does not flash the previous role’s cards.
- **Done when**: admin and user caches cannot satisfy each other; logout drops the cache.
