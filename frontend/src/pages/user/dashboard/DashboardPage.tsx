import { useState } from 'react'
import { useAuthStore } from '@/features/auth/auth_store'
import type { IntegrationTab } from '@/features/user-dashboard/types'
import { DashboardAnnouncements } from '@/features/user-dashboard/components/DashboardAnnouncements'
import { AdminDashboardOverview } from '@/features/user-dashboard/components/AdminDashboardOverview'
import { AdminSetupChecklist } from '@/features/user-dashboard/components/AdminSetupChecklist'
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

  const apiKeyCount = stats?.api_keys?.total || 0
  const totalRequests = usageStats?.total_requests || 0
  /** 一次都没调用成功过 —— 图表、最近调用、模型分布全都是空的，铺出来只是四个空盒子。 */
  const isFirstRun = !isAdmin && totalRequests === 0
  /** 新装的样子：除管理员外无人注册，窗口内也没有任何转发。 */
  const adminLooksNew = isAdmin && (stats?.users?.total || 0) <= 1 && trendData.length === 0

  if (statsLoading) {
    return <DashboardSkeleton />
  }

  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500">
      <DashboardAnnouncements announcements={announcements} />

      {/* Admin Dashboard */}
      {isAdmin && (
        <>
          {/* 除了管理员没有别人、窗口内也没有任何转发 —— 这台网关显然还没开张。可永久关掉。 */}
          {adminLooksNew && (
            <AdminSetupChecklist
              userCount={stats?.users?.total || 0}
              hasTraffic={trendData.length > 0}
            />
          )}
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

          {/* 一次都还没调用过：不铺三张空图表，把整屏让给「跑通第一次调用」。
              趋势、模型分布、最近调用会在有数据之后自己出现。 */}
          {isFirstRun ? (
            <QuickIntegrationPanel
              apiKeyCount={apiKeyCount}
              totalRequests={totalRequests}
              balance={stats?.balance}
              integrationTab={integrationTab}
              onIntegrationTabChange={setIntegrationTab}
            />
          ) : (
            <>
              <UserDashboardCharts
                trendData={trendData}
                modelData={modelData}
                trendDays={trendDays}
                onTrendDaysChange={setTrendDays}
              />
              <div className="grid gap-6 lg:grid-cols-2">
                <RecentUsageTable recentUsage={recentUsage} />
                <QuickIntegrationPanel
                  apiKeyCount={apiKeyCount}
                  totalRequests={totalRequests}
                  balance={stats?.balance}
                  integrationTab={integrationTab}
                  onIntegrationTabChange={setIntegrationTab}
                />
              </div>
            </>
          )}
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
