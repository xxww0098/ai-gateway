import { KeyRound, CheckCircle2, ShieldAlert, FolderKanban } from 'lucide-react'
import type { ApiKey } from '../types'

interface Props {
  keys: ApiKey[]
}

export function ApiKeysStatsBar({ keys }: Props) {
  const totalCount = keys.length
  const activeCount = keys.filter(
    (k) => (k.display_status || k.status) === 'active'
  ).length

  const limitedOrExpiringCount = keys.filter((k) => {
    const hasQuota = k.quota > 0 || k.rate_limit_30d > 0 || k.rate_limit_1d > 0 || k.rate_limit_7d > 0
    if (hasQuota) return true
    if (k.expires_at) {
      const diffMs = new Date(k.expires_at).getTime() - Date.now()
      const diffDays = Math.ceil(diffMs / (1000 * 60 * 60 * 24))
      return diffDays <= 7
    }
    return false
  }).length

  const distinctGroupCount = new Set(
    keys.map((k) => k.group_id).filter((id): id is number => id != null)
  ).size

  return (
    <div className="grid grid-cols-2 lg:grid-cols-4 gap-3">
      {/* Total Keys */}
      <div className="rounded-xl border border-border bg-card p-3.5 sm:p-4 shadow-2xs">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium text-muted-foreground">调用凭证总数</span>
          <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <KeyRound className="h-4 w-4" />
          </span>
        </div>
        <div className="mt-2 text-2xl font-bold tracking-tight text-foreground tabular-nums">
          {totalCount}
        </div>
        <p className="mt-0.5 text-[11px] text-muted-foreground">已签发可用的 API Key</p>
      </div>

      {/* Active Keys */}
      <div className="rounded-xl border border-border bg-card p-3.5 sm:p-4 shadow-2xs">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium text-muted-foreground">正常调用中</span>
          <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
            <CheckCircle2 className="h-4 w-4" />
          </span>
        </div>
        <div className="mt-2 text-2xl font-bold tracking-tight text-foreground tabular-nums">
          {activeCount}
        </div>
        <p className="mt-0.5 text-[11px] text-muted-foreground">状态有效且未被拦截</p>
      </div>

      {/* Limited or Expiring */}
      <div className="rounded-xl border border-border bg-card p-3.5 sm:p-4 shadow-2xs">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium text-muted-foreground">受限 / 即将到期</span>
          <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-amber-500/10 text-amber-600 dark:text-amber-400">
            <ShieldAlert className="h-4 w-4" />
          </span>
        </div>
        <div className="mt-2 text-2xl font-bold tracking-tight text-foreground tabular-nums">
          {limitedOrExpiringCount}
        </div>
        <p className="mt-0.5 text-[11px] text-muted-foreground">配有消费上限或 7 天内到期</p>
      </div>

      {/* Distinct Groups */}
      <div className="rounded-xl border border-border bg-card p-3.5 sm:p-4 shadow-2xs">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium text-muted-foreground">已关联分组</span>
          <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-violet-500/10 text-violet-600 dark:text-violet-400">
            <FolderKanban className="h-4 w-4" />
          </span>
        </div>
        <div className="mt-2 text-2xl font-bold tracking-tight text-foreground tabular-nums">
          {distinctGroupCount}
        </div>
        <p className="mt-0.5 text-[11px] text-muted-foreground">绑定指定权限或订阅池</p>
      </div>
    </div>
  )
}
