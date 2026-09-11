import { memo, type ReactNode } from 'react'
import type { UsageStatsCardsProps } from '../types'
import { fmtCost, fmtTokens, fmtTokensExact } from '../format'

type KpiTone = 'tokens' | 'cost' | 'requests' | 'model'

const DOT_CLASSES: Record<KpiTone, string> = {
  tokens: 'bg-teal-600 dark:bg-teal-400',
  cost: 'bg-[#716dff] dark:bg-[#8b87ff]',
  requests: 'bg-sky-600 dark:bg-sky-400',
  model: 'bg-amber-600 dark:bg-amber-400',
}

const UNIT_CLASSES: Record<KpiTone, string> = {
  tokens: 'bg-teal-500/10 text-teal-700 dark:text-teal-300 border border-teal-500/20',
  cost: 'bg-[#716dff]/10 text-[#716dff] dark:text-[#a09dff] border border-[#716dff]/20',
  requests: 'bg-sky-500/10 text-sky-700 dark:text-sky-300 border border-sky-500/20',
  model: 'bg-amber-500/10 text-amber-700 dark:text-amber-300 border border-amber-500/20',
}

function KpiCard({
  label,
  value,
  detail,
  unit,
  compact,
  tone = 'tokens',
}: {
  label: string
  value: string
  detail: ReactNode
  unit?: string
  compact?: boolean
  tone?: KpiTone
}) {
  return (
    <div className="usage-kpi" data-tone={tone}>
      <div className="flex min-h-7 items-center gap-1.5 text-xs font-semibold text-muted-foreground">
        <span className={`size-1.5 rounded-[2px] ${DOT_CLASSES[tone]}`} aria-hidden />
        <span>{label}</span>
        {unit ? (
          <span className={`ml-auto rounded-md px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide ${UNIT_CLASSES[tone]}`}>
            {unit}
          </span>
        ) : null}
      </div>
      <p className={compact ? 'usage-kpi-value is-compact' : 'usage-kpi-value'}>{value}</p>
      <div className="usage-kpi-detail">{detail}</div>
    </div>
  )
}

export const UsageStatsCards = memo(function UsageStatsCards({
  stats,
  topModel,
}: UsageStatsCardsProps) {
  const requests = stats?.total_requests ?? 0
  const success = stats?.success_count ?? 0
  const rate = requests === 0 ? 0 : (success / requests) * 100
  const rateText = requests === 0 ? '—' : `${rate.toFixed(1)}%`

  const rateClass =
    requests === 0
      ? 'text-muted-foreground'
      : rate >= 99
        ? 'text-emerald-600 dark:text-emerald-400 font-semibold'
        : rate >= 95
          ? 'text-amber-700 dark:text-amber-400 font-semibold'
          : 'text-red-600 dark:text-red-400 font-semibold'

  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <KpiCard
        tone="tokens"
        label="总消耗 Token"
        unit="m"
        value={fmtTokens(stats?.total_tokens || 0)}
        detail={
          <span>
            Input <strong className="font-medium text-foreground">{fmtTokensExact(stats?.total_input_tokens || 0)}</strong> · Output <strong className="font-medium text-foreground">{fmtTokensExact(stats?.total_output_tokens || 0)}</strong>
          </span>
        }
      />
      <KpiCard
        tone="cost"
        label="实际扣费"
        unit="USD"
        value={fmtCost(stats?.total_actual_cost || 0)}
        detail={
          <span>
            标准 <span className="tabular-nums font-medium text-foreground">{fmtCost(stats?.total_cost || 0)}</span>
          </span>
        }
      />
      <KpiCard
        tone="requests"
        label="总请求"
        value={(stats?.total_requests ?? 0).toLocaleString()}
        detail={
          <span>
            成功率 <span className={`tabular-nums ${rateClass}`}>{rateText}</span>
          </span>
        }
      />
      <KpiCard
        tone="model"
        label="最常用模型"
        compact
        value={topModel?.model ?? '—'}
        detail={
          topModel ? (
            <span>
              <strong className="font-medium text-foreground">{topModel.requests.toLocaleString()}</strong> 次 · <span className="text-foreground/90">{fmtTokens(topModel.tokens)}</span>
            </span>
          ) : (
            '当前范围还没有模型用量'
          )
        }
      />
    </div>
  )
})
