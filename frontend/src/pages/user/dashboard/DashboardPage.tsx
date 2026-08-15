import { useState } from 'react'
import { useAuthStore } from '@/features/auth/auth_store'
import type { IntegrationTab } from '@/features/user-dashboard/types'
import { DashboardAnnouncements } from '@/features/user-dashboard/components/DashboardAnnouncements'
import { AdminDashboardOverview } from '@/features/user-dashboard/components/AdminDashboardOverview'
import { AdminDashboardCharts } from '@/features/user-dashboard/components/AdminDashboardCharts'
import { UserDashboardHero } from '@/features/user-dashboard/components/UserDashboardHero'
import { UserDashboardCharts } from '@/features/user-dashboard/components/UserDashboardCharts'
import { RecentUsageTable } from '@/features/user-dashboard/components/RecentUsageTable'
import { QuickIntegrationPanel } from '@/features/user-dashboard/components/QuickIntegrationPanel'
import {
  useDashboardStats,
  useDashboardTrend,
  useDashboardModels,
  useRecentUsage,
  useAnnouncements,
} from '@/features/user-dashboard/hooks'

export default function Dashboard() {
  const user = useAuthStore(s => s.user)
  const isAdmin = user?.role === 'admin'
  const [integrationTab, setIntegrationTab] = useState<IntegrationTab>('openai')
  const [trendDays, setTrendDays] = useState<7 | 30>(7)

  // Data hooks
  const { stats, usageStats, loading: statsLoading } = useDashboardStats()
  const trendQuery = useDashboardTrend(trendDays)
  const modelsQuery = useDashboardModels()
  const recentUsageQuery = useRecentUsage()
  const announcementsQuery = useAnnouncements()

  const trendData = trendQuery.data || []
  const modelData = modelsQuery.data || []
  const recentUsage = recentUsageQuery.data || []
  const announcements = announcementsQuery.data || []

  if (statsLoading) {
    return <DashboardSkeleton />
  }

  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500">
      <DashboardAnnouncements announcements={announcements} />

      {/* Admin Dashboard */}
      {isAdmin && (
        <>
          <AdminDashboardOverview stats={stats} />
          <AdminDashboardCharts
            trendData={trendData}
            modelData={modelData}
            trendDays={trendDays}
            onTrendDaysChange={setTrendDays}
          />
        </>
      )}

      {/* User Dashboard */}
      {!isAdmin && (
        <div className="space-y-8">
          <UserDashboardHero email={user?.email} stats={stats} usageStats={usageStats} />
          <UserDashboardCharts
            trendData={trendData}
            modelData={modelData}
            trendDays={trendDays}
            onTrendDaysChange={setTrendDays}
          />
          <div className="grid gap-6 lg:grid-cols-2">
            <RecentUsageTable recentUsage={recentUsage} />
            <QuickIntegrationPanel
              apiKeyCount={stats?.api_keys?.total || 0}
              integrationTab={integrationTab}
              onIntegrationTabChange={setIntegrationTab}
            />
          </div>
        </div>
      )}
    </div>
  )
}

function DashboardSkeleton() {
  return (
    <div
      aria-busy="true"
      aria-label="Loading dashboard"
      className="space-y-8 animate-pulse"
      role="status"
    >
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {Array.from({ length: 4 }).map((_, index) => (
          <div
            className="rounded-2xl border border-border bg-card p-5 shadow-sm dark:border-dark-800 dark:bg-dark-900"
            key={index}
          >
            <div className="mb-5 flex items-center justify-between">
              <div className="h-4 w-24 rounded bg-muted dark:bg-dark-800" />
              <div className="h-10 w-10 rounded-xl bg-muted dark:bg-dark-800" />
            </div>
            <div className="h-7 w-28 rounded bg-muted dark:bg-dark-800" />
            <div className="mt-3 h-3 w-32 rounded bg-muted dark:bg-dark-800" />
          </div>
        ))}
      </div>
      <div className="grid gap-6 lg:grid-cols-3">
        <div className="rounded-2xl border border-border bg-card p-6 shadow-sm dark:border-dark-800 dark:bg-dark-900 lg:col-span-2">
          <div className="mb-6 flex items-center justify-between">
            <div className="h-5 w-40 rounded bg-muted dark:bg-dark-800" />
            <div className="h-9 w-28 rounded-full bg-muted dark:bg-dark-800" />
          </div>
          <div className="flex h-64 items-end gap-3">
            {[35, 58, 44, 72, 55, 88, 64].map(height => (
              <div className="flex-1 rounded-t-lg bg-muted dark:bg-dark-800" key={height} style={{ height: `${height}%` }} />
            ))}
          </div>
        </div>
        <div className="space-y-4 rounded-2xl border border-border bg-card p-6 shadow-sm dark:border-dark-800 dark:bg-dark-900">
          <div className="h-5 w-36 rounded bg-muted dark:bg-dark-800" />
          {Array.from({ length: 5 }).map((_, index) => (
            <div className="flex items-center gap-3" key={index}>
              <div className="h-9 w-9 rounded-full bg-muted dark:bg-dark-800" />
              <div className="flex-1 space-y-2">
                <div className="h-3 w-3/4 rounded bg-muted dark:bg-dark-800" />
                <div className="h-3 w-1/2 rounded bg-muted dark:bg-dark-800" />
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
