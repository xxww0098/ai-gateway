import { Link } from 'react-router-dom'
import { Server, Coins, FileText, ShieldAlert } from 'lucide-react'
import { adminRoutes, adminBillingTab } from '@/shared/routes/admin'
import type { AdminDashboardOverviewProps } from '../types'

type AdminKpiTone = 'users' | 'keys' | 'requests' | 'cost'

const DOT_CLASSES: Record<AdminKpiTone, string> = {
  users: 'bg-sky-600 dark:bg-sky-400',
  keys: 'bg-amber-600 dark:bg-amber-400',
  requests: 'bg-teal-600 dark:bg-teal-400',
  cost: 'bg-[#716dff] dark:bg-[#8b87ff]',
}

const UNIT_CLASSES: Record<AdminKpiTone, string> = {
  users: 'bg-sky-500/10 text-sky-700 dark:text-sky-300 border border-sky-500/20',
  keys: 'bg-amber-500/10 text-amber-700 dark:text-amber-300 border border-amber-500/20',
  requests: 'bg-teal-500/10 text-teal-700 dark:text-teal-300 border border-teal-500/20',
  cost: 'bg-[#716dff]/10 text-[#716dff] dark:text-[#a09dff] border border-[#716dff]/20',
}

interface KpiItemProps {
  tone: AdminKpiTone
  label: string
  unit: string
  value: string | number
  detail: React.ReactNode
}

function AdminKpiCard({ tone, label, unit, value, detail }: KpiItemProps) {
  return (
    <div
      className="relative min-h-[142px] overflow-hidden rounded-[13px] border border-border bg-card px-4 py-3.5 shadow-[0_1px_2px_rgb(0_0_0/0.07)] flex flex-col justify-between transition-colors"
      data-tone={tone}
    >
      <div className="flex min-h-6 items-center gap-1.5 text-xs font-semibold text-muted-foreground">
        <span className={`size-1.5 rounded-[2px] ${DOT_CLASSES[tone]}`} aria-hidden />
        <span>{label}</span>
        <span className={`ml-auto rounded-md px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide ${UNIT_CLASSES[tone]}`}>
          {unit}
        </span>
      </div>
      <div className="my-1.5 text-[clamp(22px,2vw,30px)] font-bold leading-tight tracking-[-0.04em] text-foreground tabular-nums">
        {value}
      </div>
      <div className="text-[11px] leading-snug text-muted-foreground">
        {detail}
      </div>
    </div>
  )
}

export function AdminDashboardOverview({ stats }: AdminDashboardOverviewProps) {
  const totalUsers = stats?.users?.total || 0
  const activeUsers = stats?.users?.active || 0
  const totalKeys = stats?.api_keys?.total || 0
  const activeKeys = stats?.api_keys?.active || 0
  const todayRequests = stats?.usage?.today_requests || 0
  const weekRequests = stats?.usage?.week_requests || 0
  const todayCost = stats?.usage?.today_cost || 0

  const userActiveRatio = totalUsers > 0 ? Math.round((activeUsers / totalUsers) * 100) : 0
  const keyActiveRatio = totalKeys > 0 ? Math.round((activeKeys / totalKeys) * 100) : 0

  return (
    <div className="space-y-4">
      {/* Header bar: Title, live pulse, and quick actions */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-foreground">管理中心概览</h2>
          <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span className="relative flex size-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
              <span className="relative inline-flex size-2 rounded-full bg-emerald-500" />
            </span>
            <span>系统运行正常</span>
            <span>·</span>
            <span>实时精算入账</span>
            <span>·</span>
            <span>契约只认 agw- 密钥</span>
          </div>
        </div>

        {/* Quick Admin Navigation Chips */}
        <div className="flex flex-wrap items-center gap-1.5">
          <Link
            to={adminRoutes.channels}
            className="inline-flex items-center gap-1.5 rounded-[8px] border border-border bg-card px-2.5 py-1.5 text-xs font-medium text-muted-foreground shadow-[0_1px_2px_rgb(0_0_0/0.07)] transition-colors hover:bg-muted hover:text-foreground"
          >
            <Server className="size-3.5 text-primary" />
            <span>渠道配置</span>
          </Link>
          <Link
            to={adminBillingTab('pricing')}
            className="inline-flex items-center gap-1.5 rounded-[8px] border border-border bg-card px-2.5 py-1.5 text-xs font-medium text-muted-foreground shadow-[0_1px_2px_rgb(0_0_0/0.07)] transition-colors hover:bg-muted hover:text-foreground"
          >
            <Coins className="size-3.5 text-[#716dff]" />
            <span>定价规则</span>
          </Link>
          <Link
            to={adminRoutes.usageLogs}
            className="inline-flex items-center gap-1.5 rounded-[8px] border border-border bg-card px-2.5 py-1.5 text-xs font-medium text-muted-foreground shadow-[0_1px_2px_rgb(0_0_0/0.07)] transition-colors hover:bg-muted hover:text-foreground"
          >
            <FileText className="size-3.5 text-sky-600 dark:text-sky-400" />
            <span>用量明细</span>
          </Link>
          <Link
            to={adminRoutes.auditLogs}
            className="inline-flex items-center gap-1.5 rounded-[8px] border border-border bg-card px-2.5 py-1.5 text-xs font-medium text-muted-foreground shadow-[0_1px_2px_rgb(0_0_0/0.07)] transition-colors hover:bg-muted hover:text-foreground"
          >
            <ShieldAlert className="size-3.5 text-amber-600 dark:text-amber-400" />
            <span>审计日志</span>
          </Link>
        </div>
      </div>

      {/* 4 Core Admin KPI Panels */}
      <div className="grid grid-cols-2 gap-3.5 xl:grid-cols-4">
        <AdminKpiCard
          tone="users"
          label="全站用户数"
          unit="USERS"
          value={totalUsers}
          detail={
            <span>
              活跃 <strong className="font-semibold text-foreground tabular-nums">{activeUsers}</strong> 人
              {totalUsers > 0 && <span className="ml-1.5 text-muted-foreground/80">({userActiveRatio}% 活跃率)</span>}
            </span>
          }
        />

        <AdminKpiCard
          tone="keys"
          label="总 API Keys"
          unit="KEYS"
          value={totalKeys}
          detail={
            <span>
              活跃 <strong className="font-semibold text-foreground tabular-nums">{activeKeys}</strong> 个
              {totalKeys > 0 && <span className="ml-1.5 text-muted-foreground/80">({keyActiveRatio}% 使用中)</span>}
            </span>
          }
        />

        <AdminKpiCard
          tone="requests"
          label="今日调用量"
          unit="REQ"
          value={todayRequests.toLocaleString()}
          detail={
            <span>
              最近 7 天 <strong className="font-semibold text-foreground tabular-nums">{weekRequests.toLocaleString()}</strong> 次
            </span>
          }
        />

        <AdminKpiCard
          tone="cost"
          label="今日产生费用"
          unit="USD"
          value={`$${todayCost.toFixed(4)}`}
          detail={
            <span>
              USD 实时精算 · <span className="text-muted-foreground/90">Hold/Settle</span>
            </span>
          }
        />
      </div>
    </div>
  )
}
