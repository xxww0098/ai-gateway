import { Link } from 'react-router-dom'
import { ArrowRight, CheckCircle2, XCircle, Activity } from 'lucide-react'
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
  if (mins < 60) return `${mins} 分钟前`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours} 小时前`
  const days = Math.floor(hours / 24)
  return `${days} 天前`
}

export function RecentUsageTable({ recentUsage }: RecentUsageTableProps) {
  return (
    <div className="overflow-hidden rounded-[13px] border border-border bg-card shadow-[0_1px_2px_rgb(0_0_0/0.07)] flex flex-col justify-between">
      {/* Table Header */}
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <div className="flex items-center gap-2">
          <span className="size-1.5 rounded-[2px] bg-primary" aria-hidden />
          <div>
            <h3 className="text-xs font-semibold text-foreground">最近调用</h3>
            <p className="text-[11px] text-muted-foreground">最新发起的 API 请求精算与状态</p>
          </div>
        </div>
        <Link
          to={userRoutes.usage}
          className="inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
        >
          查看全部
          <ArrowRight className="size-3" />
        </Link>
      </div>

      {/* Table Content */}
      <div className="flex-1 overflow-y-auto max-h-[320px]">
        {recentUsage.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12 text-center px-4">
            <div className="flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground mb-3">
              <Activity className="size-5" />
            </div>
            <h4 className="text-xs font-semibold text-foreground">最近暂无调用</h4>
            <p className="mt-1 max-w-xs text-[11px] text-muted-foreground">
              跑一次请求，这里会按时间倒序列出每一笔记录，含模型、Token 吞吐和实际花费。
            </p>
            <Link
              to={userRoutes.usage}
              className="mt-3.5 inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
            >
              看完整用量账本 →
            </Link>
          </div>
        ) : (
          <div className="divide-y divide-border/50">
            {recentUsage.map(log => (
              <div
                key={log.id}
                className="flex items-center justify-between px-4 py-3 hover:bg-muted/30 transition-colors"
              >
                <div className="flex items-center gap-3 min-w-0">
                  <div className="shrink-0">
                    {log.failed ? (
                      <XCircle className="size-4 text-red-500" />
                    ) : (
                      <CheckCircle2 className="size-4 text-emerald-600 dark:text-emerald-400" />
                    )}
                  </div>
                  <div className="min-w-0">
                    <p className="text-xs font-medium text-foreground truncate font-mono">{log.model}</p>
                    <p className="text-[11px] text-muted-foreground mt-0.5 tabular-nums">
                      {log.api_key_name && <span className="mr-2 font-sans text-muted-foreground/80">{log.api_key_name}</span>}
                      ↓{fmtTokens(log.input_tokens)} · ↑{fmtTokens(log.output_tokens)}
                    </p>
                  </div>
                </div>
                <div className="text-right shrink-0 ml-3">
                  <p className="text-xs font-semibold text-foreground tabular-nums">
                    {fmtCost(log.actual_cost)}
                  </p>
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
