import { Users, Key, Activity, DollarSign } from 'lucide-react'
import type { AdminDashboardOverviewProps } from '../types'

export function AdminDashboardOverview({ stats }: AdminDashboardOverviewProps) {
  return (
    <>
      <div className="mb-6">
        <h2 className="text-2xl font-bold tracking-tight text-foreground">管理中心概览</h2>
        <p className="text-sm text-muted-foreground mt-1">系统全局统计数据及状态</p>
      </div>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <div className="rounded-xl border border-border bg-card p-5">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">总用户数</span>
            <Users className="w-4 h-4 text-muted-foreground" />
          </div>
          <div className="text-2xl font-bold tabular-nums text-foreground mt-2">{stats?.users?.total || 0}</div>
          <div className="text-xs text-muted-foreground mt-1 flex items-center gap-1">
            活跃 <span className="tabular-nums font-medium text-foreground">{stats?.users?.active || 0}</span>
          </div>
        </div>

        <div className="rounded-xl border border-border bg-card p-5">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">总 API Keys</span>
            <Key className="w-4 h-4 text-muted-foreground" />
          </div>
          <div className="text-2xl font-bold tabular-nums text-foreground mt-2">{stats?.api_keys?.total || 0}</div>
          <div className="text-xs text-muted-foreground mt-1 flex items-center gap-1">
            活跃 <span className="tabular-nums font-medium text-foreground">{stats?.api_keys?.active || 0}</span>
          </div>
        </div>

        <div className="rounded-xl border border-border bg-card p-5">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">今日调用量</span>
            <Activity className="w-4 h-4 text-muted-foreground" />
          </div>
          <div className="text-2xl font-bold tabular-nums text-foreground mt-2">{(stats?.usage?.today_requests || 0).toLocaleString()}</div>
          <div className="text-xs text-muted-foreground mt-1 flex items-center gap-1">
            最近 7 天 <span className="tabular-nums font-medium text-foreground">{(stats?.usage?.week_requests || 0).toLocaleString()}</span>
          </div>
        </div>

        <div className="rounded-xl border border-border bg-card p-5">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">今日产生费用</span>
            <DollarSign className="w-4 h-4 text-muted-foreground" />
          </div>
          <div className="text-2xl font-bold tabular-nums text-foreground mt-2">${(stats?.usage?.today_cost || 0).toFixed(4)}</div>
          <div className="text-xs text-muted-foreground mt-1 flex items-center gap-1">
            USD 实时精算
          </div>
        </div>
      </div>
    </>
  )
}
