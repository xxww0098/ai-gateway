import { Link } from 'react-router-dom'
import { Activity, ArrowRight, CheckCircle2, XCircle } from 'lucide-react'
import { EmptyState } from '@/shared/components/EmptyState'
import { userRoutes } from '@/shared/routes/user'
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
    <div className="rounded-xl border border-border bg-card overflow-hidden flex flex-col">
      <div className="px-5 py-4 border-b border-border flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Activity className="w-4 h-4 text-primary" />
          <h3 className="text-sm font-semibold text-foreground">
            最近调用
          </h3>
        </div>
        <Link
          to="/usage"
          className="text-xs text-primary hover:underline font-medium flex items-center gap-1 transition-colors"
        >
          查看全部
          <ArrowRight className="w-3 h-3" />
        </Link>
      </div>
      <div className="flex-1 p-0 overflow-y-auto max-h-[300px]">
        {recentUsage.length === 0 ? (
          <EmptyState
            size="compact"
            icon={Activity}
            title="最近没有调用"
            description="跑一次请求，这里会按时间倒序列出每一笔，含模型、token 和实际花费。"
            action={{ label: '看完整用量', to: userRoutes.usage }}
          />
        ) : (
          <div className="divide-y divide-border/50">
            {recentUsage.map(log => (
              <div key={log.id} className="px-5 py-3 flex items-center justify-between hover:bg-muted/40 transition-colors">
                <div className="flex items-center gap-3 min-w-0">
                  <div className="flex-shrink-0">
                    {log.failed ? (
                      <XCircle className="w-4 h-4 text-red-500" />
                    ) : (
                      <CheckCircle2 className="w-4 h-4 text-emerald-500" />
                    )}
                  </div>
                  <div className="min-w-0">
                    <p className="text-xs font-medium text-foreground truncate font-mono">{log.model}</p>
                    <p className="text-[11px] text-muted-foreground mt-0.5 tabular-nums">
                      {log.api_key_name && <span className="mr-2 font-sans">{log.api_key_name}</span>}
                      ↓{fmtTokens(log.input_tokens)} · ↑{fmtTokens(log.output_tokens)}
                    </p>
                  </div>
                </div>
                <div className="text-right flex-shrink-0 ml-3">
                  <p className="text-xs font-semibold text-emerald-600 dark:text-emerald-400 tabular-nums">{fmtCost(log.actual_cost)}</p>
                  <p className="text-[10px] text-muted-foreground mt-0.5">{timeAgo(log.created_at)}</p>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
