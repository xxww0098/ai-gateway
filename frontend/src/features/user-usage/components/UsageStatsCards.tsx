import { memo } from 'react'
import { FileText, Coins, DollarSign, Clock } from 'lucide-react'
import type { UsageStatsCardsProps } from '../types'

function fmtDuration(ms: number | null): string {
  if (ms == null) return '-'
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

function fmtCost(n: number): string {
  if (n === 0) return '$0.00'
  if (n < 0.0001) return `$${n.toFixed(6)}`
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(4)}`
}

export const UsageStatsCards = memo(function UsageStatsCards({ stats }: UsageStatsCardsProps) {
  return (
    <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
      {/* Total Requests */}
      <div className="card p-4">
        <div className="flex items-center gap-3">
          <div className="rounded-xl bg-blue-50 dark:bg-blue-900/20 p-2.5">
            <FileText className="w-5 h-5 text-blue-500" />
          </div>
          <div className="min-w-0">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400">总请求</p>
            <p className="text-xl font-bold text-gray-900 dark:text-white tabular-nums">
              {stats?.total_requests?.toLocaleString() || '0'}
            </p>
            <p className="text-[11px] text-gray-400 dark:text-dark-500">
              <span className="text-green-500">{stats?.success_count || 0}</span> 成功
              {(stats?.fail_count ?? 0) > 0 && (
                <> · <span className="text-red-500">{stats?.fail_count}</span> 失败</>
              )}
            </p>
          </div>
        </div>
      </div>

      {/* Total Tokens */}
      <div className="card p-4">
        <div className="flex items-center gap-3">
          <div className="rounded-xl bg-amber-50 dark:bg-amber-900/20 p-2.5">
            <Coins className="w-5 h-5 text-amber-500" />
          </div>
          <div className="min-w-0">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400">总 Tokens</p>
            <p className="text-xl font-bold text-gray-900 dark:text-white tabular-nums">
              {fmtTokens(stats?.total_tokens || 0)}
            </p>
            <p className="text-[11px] text-gray-400 dark:text-dark-500 tabular-nums">
              ↓{fmtTokens(stats?.total_input_tokens || 0)} · ↑{fmtTokens(stats?.total_output_tokens || 0)}
            </p>
          </div>
        </div>
      </div>

      {/* Total Cost */}
      <div className="card p-4">
        <div className="flex items-center gap-3">
          <div className="rounded-xl bg-green-50 dark:bg-green-900/20 p-2.5">
            <DollarSign className="w-5 h-5 text-green-500" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400">总费用</p>
            <p className="text-xl font-bold text-green-600 dark:text-green-400 tabular-nums">
              {fmtCost(stats?.total_actual_cost || 0)}
            </p>
            <p className="text-[11px] text-gray-400 dark:text-dark-500 tabular-nums">
              标准 <span className="line-through">{fmtCost(stats?.total_cost || 0)}</span>
            </p>
          </div>
        </div>
      </div>

      {/* Avg Duration */}
      <div className="card p-4">
        <div className="flex items-center gap-3">
          <div className="rounded-xl bg-purple-50 dark:bg-purple-900/20 p-2.5">
            <Clock className="w-5 h-5 text-purple-500" />
          </div>
          <div className="min-w-0">
            <p className="text-xs font-medium text-gray-500 dark:text-gray-400">平均耗时</p>
            <p className="text-xl font-bold text-gray-900 dark:text-white tabular-nums">
              {fmtDuration(stats?.avg_duration_ms || 0)}
            </p>
            <p className="text-[11px] text-gray-400 dark:text-dark-500">每次请求</p>
          </div>
        </div>
      </div>
    </div>
  )
})
