import { Users, Key, Activity, DollarSign } from 'lucide-react'
import type { AdminDashboardOverviewProps } from '../types'

export function AdminDashboardOverview({ stats }: AdminDashboardOverviewProps) {
  return (
    <>
      <div className="mb-8">
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">管理中心概览</h2>
        <p className="text-gray-500 dark:text-dark-300">系统全局统计数据及状态</p>
      </div>
      <div className="grid gap-6 md:grid-cols-2 xl:grid-cols-4">
        <div className="stat-card group">
          <div className="stat-icon bg-blue-50 dark:bg-blue-900/20 text-blue-500 group-hover:scale-110 transition-transform">
            <Users className="w-6 h-6" />
          </div>
          <div>
            <p className="text-sm font-medium text-gray-500 dark:text-dark-400">总用户数</p>
            <div className="text-2xl font-bold text-gray-900 dark:text-white mt-1">{stats?.users?.total || 0}</div>
            <div className="text-xs text-green-500 mt-1 flex items-center gap-1">
              活跃 {stats?.users?.active || 0}
            </div>
          </div>
        </div>

        <div className="stat-card group">
          <div className="stat-icon bg-purple-50 dark:bg-purple-900/20 text-purple-500 group-hover:scale-110 transition-transform">
            <Key className="w-6 h-6" />
          </div>
          <div>
            <p className="text-sm font-medium text-gray-500 dark:text-dark-400">总 API Keys</p>
            <div className="text-2xl font-bold text-gray-900 dark:text-white mt-1">{stats?.api_keys?.total || 0}</div>
            <div className="text-xs text-green-500 mt-1 flex items-center gap-1">
              活跃 {stats?.api_keys?.active || 0}
            </div>
          </div>
        </div>

        <div className="stat-card group">
          <div className="stat-icon bg-amber-50 dark:bg-amber-900/20 text-amber-500 group-hover:scale-110 transition-transform">
            <Activity className="w-6 h-6" />
          </div>
          <div>
            <p className="text-sm font-medium text-gray-500 dark:text-dark-400">今日调用量</p>
            <div className="text-2xl font-bold text-gray-900 dark:text-white mt-1">{stats?.usage?.today_requests || 0}</div>
            <div className="text-xs text-gray-500 dark:text-dark-400 mt-1 flex items-center gap-1">
              最近7天 {stats?.usage?.week_requests || 0}
            </div>
          </div>
        </div>

        <div className="stat-card group">
          <div className="stat-icon bg-emerald-50 dark:bg-emerald-900/20 text-emerald-500 group-hover:scale-110 transition-transform">
            <DollarSign className="w-6 h-6" />
          </div>
          <div>
            <p className="text-sm font-medium text-gray-500 dark:text-dark-400">今日产生费用</p>
            <div className="text-2xl font-bold text-gray-900 dark:text-white mt-1">${(stats?.usage?.today_cost || 0).toFixed(4)}</div>
            <div className="text-xs text-gray-500 dark:text-dark-400 mt-1 flex items-center gap-1">
              USD结算
            </div>
          </div>
        </div>
      </div>
    </>
  )
}
