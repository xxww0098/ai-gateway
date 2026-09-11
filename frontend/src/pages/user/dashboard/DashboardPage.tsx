import { lazy, Suspense, useState } from 'react'
import { useAuthStore } from '@/features/auth/auth_store'
import { isStaff } from '@/shared/role_core'
import type { IntegrationTab } from '@/features/user-dashboard/types'
import { DashboardAnnouncements } from '@/features/user-dashboard/components/DashboardAnnouncements'
import { AdminDashboardOverview } from '@/features/user-dashboard/components/AdminDashboardOverview'
import { AdminSetupChecklist } from '@/features/user-dashboard/components/AdminSetupChecklist'
import { UserDashboardHero } from '@/features/user-dashboard/components/UserDashboardHero'
import { RecentUsageTable } from '@/features/user-dashboard/components/RecentUsageTable'
import { QuickIntegrationPanel } from '@/features/user-dashboard/components/QuickIntegrationPanel'
import {
  useDashboardStats,
  useDashboardTrend,
  useDashboardModels,
  useRecentUsage,
  useAnnouncements,
} from '@/features/user-dashboard/hooks'

const AdminDashboardCharts = lazy(() =>
  import('@/features/user-dashboard/components/AdminDashboardCharts').then(m => ({
    default: m.AdminDashboardCharts,
  })),
)
const UserDashboardCharts = lazy(() =>
  import('@/features/user-dashboard/components/UserDashboardCharts').then(m => ({
    default: m.UserDashboardCharts,
  })),
)

export default function Dashboard() {
  const user = useAuthStore(s => s.user)
  const isAdmin = isStaff(user?.role)
  const [integrationTab, setIntegrationTab] = useState<IntegrationTab>('openai')
  const [trendDays, setTrendDays] = useState<7 | 30>(7)

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
  const isFirstRun = !isAdmin && !statsLoading && totalRequests === 0
  /** 新装的样子：除管理员外无人注册，窗口内也没有任何转发。 */
  const adminLooksNew = isAdmin && !statsLoading && (stats?.users?.total || 0) <= 1 && trendData.length === 0

  return (
    <div className="space-y-5">
      <DashboardAnnouncements announcements={announcements} />

      {isAdmin && (
        <>
          {/* 除了管理员没有别人、窗口内也没有任何转发 —— 这台网关显然还没开张。可永久关掉。 */}
          {adminLooksNew && (
            <AdminSetupChecklist
              userCount={stats?.users?.total || 0}
              hasTraffic={trendData.length > 0}
            />
          )}
          {statsLoading ? <StatsSkeleton /> : <AdminDashboardOverview stats={stats} />}
          <Suspense fallback={<ChartsSkeleton />}>
            <AdminDashboardCharts
              trendData={trendData}
              modelData={modelData}
              trendDays={trendDays}
              onTrendDaysChange={setTrendDays}
            />
          </Suspense>
        </>
      )}

      {!isAdmin && (
        <div className="space-y-5">
          {statsLoading ? (
            <StatsSkeleton />
          ) : (
            <UserDashboardHero email={user?.email} stats={stats} usageStats={usageStats} />
          )}

          {/* 一次都还没调用过：不铺三张空图表，把整屏让给「跑通第一次调用」。
              趋势、模型分布、最近调用会在有数据之后自己出现。 */}
          {isFirstRun && (
            <QuickIntegrationPanel
              apiKeyCount={apiKeyCount}
              totalRequests={totalRequests}
              balance={stats?.balance}
              integrationTab={integrationTab}
              onIntegrationTabChange={setIntegrationTab}
            />
          )}
          {!isAdmin && !isFirstRun && !statsLoading && (
            <>
              <Suspense fallback={<ChartsSkeleton />}>
                <UserDashboardCharts
                  trendData={trendData}
                  modelData={modelData}
                  trendDays={trendDays}
                  onTrendDaysChange={setTrendDays}
                />
              </Suspense>
              <div className="grid gap-4 lg:grid-cols-2">
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

function StatsSkeleton() {
  return (
    <div
      aria-busy="true"
      aria-label="正在加载统计数据"
      className="grid gap-3.5 sm:grid-cols-2 xl:grid-cols-4 animate-pulse"
      role="status"
    >
      {Array.from({ length: 4 }).map((_, index) => (
        <div
          className="rounded-[13px] border border-border bg-card p-4 shadow-[0_1px_2px_rgb(0_0_0/0.07)]"
          key={index}
        >
          <div className="mb-4 flex items-center justify-between">
            <div className="h-3.5 w-20 rounded bg-muted" />
            <div className="h-4 w-12 rounded bg-muted" />
          </div>
          <div className="h-8 w-28 rounded bg-muted" />
          <div className="mt-3 h-3 w-32 rounded bg-muted" />
        </div>
      ))}
    </div>
  )
}

function ChartsSkeleton() {
  return (
    <div
      aria-busy="true"
      aria-label="正在加载图表数据"
      className="grid gap-4 lg:grid-cols-3 animate-pulse"
      role="status"
    >
      <div className="rounded-[13px] border border-border bg-card p-4 shadow-[0_1px_2px_rgb(0_0_0/0.07)] lg:col-span-2">
        <div className="mb-4 flex items-center justify-between border-b border-border/50 pb-3">
          <div className="h-4 w-32 rounded bg-muted" />
          <div className="h-7 w-20 rounded-[8px] bg-muted" />
        </div>
        <div className="flex h-56 items-end gap-3 px-2">
          {[35, 58, 44, 72, 55, 88, 64].map(height => (
            <div className="flex-1 rounded-t-md bg-muted" key={height} style={{ height: `${height}%` }} />
          ))}
        </div>
      </div>
      <div className="rounded-[13px] border border-border bg-card p-4 shadow-[0_1px_2px_rgb(0_0_0/0.07)] flex flex-col justify-between">
        <div className="h-4 w-28 rounded bg-muted border-b border-border/50 pb-3" />
        <div className="flex justify-center py-6">
          <div className="size-32 rounded-full border-8 border-muted" />
        </div>
        <div className="space-y-2">
          <div className="h-3 w-3/4 rounded bg-muted" />
          <div className="h-3 w-1/2 rounded bg-muted" />
        </div>
      </div>
    </div>
  )
}
