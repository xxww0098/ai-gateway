import { Link } from 'react-router-dom'
import { Activity, ArrowRight, CheckCircle2, XCircle } from 'lucide-react'
import type { RecentUsageTableProps } from '../types'

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

function fmtCost(n: number): string {
  if (n === 0) return '$0.00'
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(2)}`
}

function timeAgo(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime()
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return '刚刚'
  if (mins < 60) return `${mins}分钟前`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}小时前`
  const days = Math.floor(hours / 24)
  return `${days}天前`
}

export function RecentUsageTable({ recentUsage }: RecentUsageTableProps) {
  return (
    <div className="glass-card overflow-hidden flex flex-col">
      <div className="px-6 py-5 border-b border-border/50 flex items-center justify-between bg-gray-50/50 dark:bg-dark-800/50">
        <div className="flex items-center gap-2">
          <Activity className="w-5 h-5 text-blue-500" />
          <h3 className="text-sm font-bold uppercase tracking-wider text-gray-900 dark:text-white">
            最近调用
          </h3>
        </div>
        <Link
          to="/usage"
          className="text-xs text-primary-500 hover:text-primary-600 font-medium flex items-center gap-1 transition-colors"
        >
          查看全部
          <ArrowRight className="w-3 h-3" />
        </Link>
      </div>
      <div className="flex-1 p-0 overflow-y-auto max-h-[300px]">
        {recentUsage.length === 0 ? (
          <div className="p-8 text-center text-gray-500 dark:text-dark-500 text-sm">暂无调用记录</div>
        ) : (
          <div className="divide-y divide-border/50">
            {recentUsage.map(log => (
              <div key={log.id} className="px-6 py-3.5 flex items-center justify-between hover:bg-gray-50 dark:hover:bg-dark-900/30 transition-colors">
                <div className="flex items-center gap-3 min-w-0">
                  <div className="flex-shrink-0">
                    {log.failed ? (
                      <XCircle className="w-4 h-4 text-red-500" />
                    ) : (
                      <CheckCircle2 className="w-4 h-4 text-green-500" />
                    )}
                  </div>
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-gray-900 dark:text-white truncate font-mono">{log.model}</p>
                    <p className="text-[11px] text-gray-400 dark:text-dark-500 mt-0.5">
                      {log.api_key_name && <span className="mr-2">{log.api_key_name}</span>}
                      ↓{fmtTokens(log.input_tokens)} · ↑{fmtTokens(log.output_tokens)}
                    </p>
                  </div>
                </div>
                <div className="text-right flex-shrink-0 ml-3">
                  <p className="text-sm font-medium text-green-600 dark:text-green-400 tabular-nums">{fmtCost(log.actual_cost)}</p>
                  <p className="text-[11px] text-gray-400 dark:text-dark-500 mt-0.5">{timeAgo(log.created_at)}</p>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
