import { Link } from 'react-router-dom'
import { Wallet, Key, Ticket, ArrowRight, ArrowUpRight, BookOpen } from 'lucide-react'
import { Button } from '@/shared/components/ui/button'
import { userRoutes } from '@/shared/routes/user'
import { docsPath } from '@/shared/routes/docs'
import type { UserDashboardHeroProps } from '../types'

export function UserDashboardHero({ email, stats, usageStats }: UserDashboardHeroProps) {
  const balance = stats?.balance || 0
  const totalReq = usageStats?.total_requests || 0
  const success = usageStats?.success_count || 0
  const tokens = usageStats?.total_tokens || 0
  const rateText = totalReq > 0 ? `${((success / totalReq) * 100).toFixed(1)}%` : '—'

  return (
    <div className="space-y-4">
      {/* Top Welcome & Actions Strip */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-foreground">
            欢迎回来，{email}
          </h2>
          <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span className="relative flex size-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
              <span className="relative inline-flex size-2 rounded-full bg-emerald-500" />
            </span>
            <span>可用余额实时精算</span>
            <span>·</span>
            <span>仅认 agw- 密钥</span>
            <span>·</span>
            <span>双通道高可用中转</span>
          </div>
        </div>

        {/* Quick Action Buttons on Desktop */}
        <div className="flex flex-wrap items-center gap-2">
          <Button asChild size="sm" className="h-8 text-xs rounded-[8px] bg-primary text-primary-foreground hover:bg-primary/90">
            <Link to={userRoutes.financeTopup} className="inline-flex items-center gap-1.5">
              <Wallet className="size-3.5" />
              在线充值
            </Link>
          </Button>
          <Button asChild size="sm" variant="outline" className="h-8 text-xs rounded-[8px]">
            <Link to={userRoutes.keys} className="inline-flex items-center gap-1.5">
              <Key className="size-3.5" />
              管理密钥
            </Link>
          </Button>
          <Button asChild size="sm" variant="ghost" className="h-8 text-xs rounded-[8px] text-muted-foreground hover:text-foreground">
            <Link to={docsPath('quickstart')} className="inline-flex items-center gap-1.5">
              <BookOpen className="size-3.5" />
              接入文档
            </Link>
          </Button>
        </div>
      </div>

      {/* 3 Core Metric Tiles */}
      <div className="grid gap-3.5 sm:grid-cols-3">
        {/* Available Balance */}
        <div className="relative min-h-[142px] overflow-hidden rounded-[13px] border border-border bg-card px-4 py-3.5 shadow-[0_1px_2px_rgb(0_0_0/0.07)] flex flex-col justify-between transition-colors">
          <div className="flex min-h-6 items-center gap-1.5 text-xs font-semibold text-muted-foreground">
            <span className="size-1.5 rounded-[2px] bg-[#716dff] dark:bg-[#8b87ff]" aria-hidden />
            <span>可用余额</span>
            <span className="ml-auto rounded-md px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide bg-[#716dff]/10 text-[#716dff] dark:text-[#a09dff] border border-[#716dff]/20">
              USD
            </span>
          </div>
          <div className="my-1.5 text-[clamp(24px,2.2vw,32px)] font-bold leading-tight tracking-[-0.04em] text-foreground tabular-nums">
            ${balance.toFixed(2)}
          </div>
          <div className="flex items-center justify-between text-[11px] leading-snug text-muted-foreground border-t border-border/50 pt-2">
            <span>实时精算入账</span>
            <Link
              to={userRoutes.financeTopup}
              className="inline-flex items-center gap-0.5 font-medium text-primary hover:underline"
            >
              充值 <ArrowUpRight className="size-3" />
            </Link>
          </div>
        </div>

        {/* Total Requests */}
        <div className="relative min-h-[142px] overflow-hidden rounded-[13px] border border-border bg-card px-4 py-3.5 shadow-[0_1px_2px_rgb(0_0_0/0.07)] flex flex-col justify-between transition-colors">
          <div className="flex min-h-6 items-center gap-1.5 text-xs font-semibold text-muted-foreground">
            <span className="size-1.5 rounded-[2px] bg-sky-600 dark:bg-sky-400" aria-hidden />
            <span>总请求次数</span>
            <span className="ml-auto rounded-md px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide bg-sky-500/10 text-sky-700 dark:text-sky-300 border border-sky-500/20">
              REQ
            </span>
          </div>
          <div className="my-1.5 text-[clamp(24px,2.2vw,32px)] font-bold leading-tight tracking-[-0.04em] text-foreground tabular-nums">
            {totalReq.toLocaleString()}
          </div>
          <div className="flex items-center justify-between text-[11px] leading-snug text-muted-foreground border-t border-border/50 pt-2">
            <span>
              成功率 <strong className="font-semibold text-foreground tabular-nums">{rateText}</strong>
            </span>
            <span className="tabular-nums font-medium text-muted-foreground">
              {success.toLocaleString()} 成功
            </span>
          </div>
        </div>

        {/* Total Tokens */}
        <div className="relative min-h-[142px] overflow-hidden rounded-[13px] border border-border bg-card px-4 py-3.5 shadow-[0_1px_2px_rgb(0_0_0/0.07)] flex flex-col justify-between transition-colors">
          <div className="flex min-h-6 items-center gap-1.5 text-xs font-semibold text-muted-foreground">
            <span className="size-1.5 rounded-[2px] bg-teal-600 dark:bg-teal-400" aria-hidden />
            <span>消耗 Tokens</span>
            <span className="ml-auto rounded-md px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide bg-teal-500/10 text-teal-700 dark:text-teal-300 border border-teal-500/20">
              TOKENS
            </span>
          </div>
          <div className="my-1.5 text-[clamp(24px,2.2vw,32px)] font-bold leading-tight tracking-[-0.04em] text-foreground tabular-nums">
            {tokens.toLocaleString()}
          </div>
          <div className="flex items-center justify-between text-[11px] leading-snug text-muted-foreground border-t border-border/50 pt-2">
            <span>输入 / 输出 / 缓存 / 推理</span>
            <Link
              to={userRoutes.usage}
              className="inline-flex items-center gap-0.5 font-medium text-primary hover:underline"
            >
              看明细 <ArrowRight className="size-3" />
            </Link>
          </div>
        </div>
      </div>

      {/* Mobile Quick Actions (hidden on sm+) */}
      <div className="grid grid-cols-3 gap-2 sm:hidden">
        <Link
          to={userRoutes.keys}
          className="flex min-h-10 items-center justify-center gap-1.5 rounded-[8px] border border-border bg-card text-xs font-semibold text-foreground active:bg-muted"
        >
          <Key className="size-3.5 text-primary" />
          密钥
        </Link>
        <Link
          to={userRoutes.financeTopup}
          className="flex min-h-10 items-center justify-center gap-1.5 rounded-[8px] border border-border bg-card text-xs font-semibold text-foreground active:bg-muted"
        >
          <Wallet className="size-3.5 text-primary" />
          充值
        </Link>
        <Link
          to={userRoutes.tickets}
          className="flex min-h-10 items-center justify-center gap-1.5 rounded-[8px] border border-border bg-card text-xs font-semibold text-foreground active:bg-muted"
        >
          <Ticket className="size-3.5 text-primary" />
          工单
        </Link>
      </div>
    </div>
  )
}
