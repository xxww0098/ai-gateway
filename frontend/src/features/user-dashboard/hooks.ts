// React-query hooks for user-dashboard feature
import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/shared/api/query-keys'
import { useAuthStore } from '@/features/auth/auth_store'
import type { DashboardStats, UsageStats } from './types'
import {
  fetchAdminDashboard,
  fetchAdminUsageTrend,
  fetchAdminModelStats,
  fetchUserProfile,
  fetchUserUsageStats,
  fetchUserUsageTrend,
  fetchUserModelStats,
  fetchUserRecentUsage,
  fetchAnnouncements,
} from './api'

// ── useDashboardStats ───────────────────────────────────────────────────────

/**
 * Fetches dashboard stats for both admin and user roles.
 * Admin: fetches from /admin/dashboard
 * User: aggregates from /user/profile and /user/usage/stats
 *
 * Uses the default staleTime (5min) from the query client config.
 */
export function useDashboardStats() {
  const user = useAuthStore(s => s.user)
  const isAdmin = user?.role === 'admin'

  const adminQuery = useQuery({
    queryKey: queryKeys.dashboard.stats(),
    queryFn: async () => {
      const data = await fetchAdminDashboard()
      return {
        stats: {
          users: data.users,
          api_keys: data.api_keys,
          usage: data.usage,
        } as DashboardStats,
        usageStats: null as UsageStats | null,
      }
    },
    enabled: isAdmin,
  })

  const userQuery = useQuery({
    queryKey: queryKeys.dashboard.stats(),
    queryFn: async () => {
      const [profileRes, usageRes] = await Promise.all([
        fetchUserProfile(),
        fetchUserUsageStats().catch(() => null),
      ])
      const ustats = usageRes?.usage ?? null
      const userBalance = profileRes.available_balance ?? profileRes.user?.balance ?? 0
      const stats: DashboardStats = {
        users: { total: 1, active: 1 },
        api_keys: { total: profileRes.key_count, active: profileRes.key_count },
        usage: { today_requests: ustats?.total_requests || 0, today_cost: userBalance, week_requests: 0 },
        isUser: true,
        balance: userBalance,
        quota: profileRes.quota || 0,
        used_quota: profileRes.used_quota || 0,
      }
      return { stats, usageStats: ustats }
    },
    enabled: !isAdmin,
  })

  const activeQuery = isAdmin ? adminQuery : userQuery

  return {
    stats: activeQuery.data?.stats ?? null,
    usageStats: activeQuery.data?.usageStats ?? null,
    loading: activeQuery.isLoading,
    error: activeQuery.error,
  }
}

// ── useDashboardTrend ───────────────────────────────────────────────────────

/**
 * Fetches usage trend data for the dashboard charts.
 * Automatically selects admin or user endpoint based on role.
 */
export function useDashboardTrend(days: 7 | 30) {
  const user = useAuthStore(s => s.user)
  const isAdmin = user?.role === 'admin'

  return useQuery({
    queryKey: queryKeys.dashboard.trend(days),
    queryFn: () => isAdmin ? fetchAdminUsageTrend(days) : fetchUserUsageTrend(days),
  })
}

// ── useDashboardModels ──────────────────────────────────────────────────────

/**
 * Fetches model distribution data for the dashboard charts.
 */
export function useDashboardModels() {
  const user = useAuthStore(s => s.user)
  const isAdmin = user?.role === 'admin'

  return useQuery({
    queryKey: [...queryKeys.dashboard.all(), 'models'] as const,
    queryFn: () => isAdmin ? fetchAdminModelStats() : fetchUserModelStats(),
  })
}

// ── useRecentUsage ──────────────────────────────────────────────────────────

/**
 * Fetches recent usage logs for the user dashboard.
 * Only enabled for non-admin users.
 */
export function useRecentUsage() {
  const user = useAuthStore(s => s.user)
  const isAdmin = user?.role === 'admin'

  return useQuery({
    queryKey: queryKeys.dashboard.recentUsage(),
    queryFn: async () => {
      const res = await fetchUserRecentUsage()
      return res.items || []
    },
    enabled: !isAdmin,
  })
}

// ── useAnnouncements ────────────────────────────────────────────────────────

/**
 * Fetches announcements for the dashboard.
 */
export function useAnnouncements() {
  return useQuery({
    queryKey: [...queryKeys.dashboard.all(), 'announcements'] as const,
    queryFn: () => fetchAnnouncements(),
  })
}
