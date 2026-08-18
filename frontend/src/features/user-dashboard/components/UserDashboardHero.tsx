import { Link } from 'react-router-dom'
import { Activity, Cpu, DollarSign, Key, Ticket, Wallet, ArrowUpRight } from 'lucide-react'
import { Button } from '@/shared/components/ui/button'
import { userRoutes } from '@/shared/routes/user'
import type { UserDashboardHeroProps } from '../types'

export function UserDashboardHero({ email, stats, usageStats }: UserDashboardHeroProps) {
  const balance = stats?.balance || 0
  const totalReq = usageStats?.total_requests || 0
  const success = usageStats?.success_count || 0
  const tokens = usageStats?.total_tokens || 0

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-xl sm:text-2xl font-bold tracking-tight text-foreground">
          欢迎，{email}
        </h2>
        <p className="text-sm text-muted-foreground mt-1">
          额度、请求与常用操作
        </p>
      </div>

      {/* Mobile: Balance & Key Metrics Strip */}
      <div className="sm:hidden rounded-xl border border-border bg-card p-4 space-y-3">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="text-xs uppercase tracking-wider text-muted-foreground font-semibold">可用余额</p>
            <p className="text-2xl font-bold tabular-nums text-foreground mt-0.5">
              ${balance.toFixed(2)}
            </p>
          </div>
          <Button asChild size="sm" className="min-h-10 shrink-0">
            <Link to={userRoutes.financeTopup}>
              <Wallet className="h-3.5 w-3.5 mr-1" />
              充值
            </Link>
          </Button>
        </div>
        <div className="flex gap-4 text-xs text-muted-foreground pt-1 border-t border-border/50">
          <span>
            请求 <strong className="tabular-nums text-foreground">{totalReq}</strong>
            {totalReq > 0 && <span className="text-muted-foreground"> · {success} 成功</span>}
          </span>
          <span>
            Tokens <strong className="tabular-nums text-foreground">{tokens.toLocaleString()}</strong>
          </span>
        </div>
      </div>

      {/* Mobile quick actions */}
      <div className="sm:hidden grid grid-cols-3 gap-2">
        <Link
          to={userRoutes.keys}
          className="flex min-h-12 flex-col items-center justify-center gap-1 rounded-xl border border-border bg-card text-xs font-medium active:bg-muted"
        >
          <Key className="h-4 w-4 text-primary" />
          密钥
        </Link>
        <Link
          to={userRoutes.financeTopup}
          className="flex min-h-12 flex-col items-center justify-center gap-1 rounded-xl border border-border bg-card text-xs font-medium active:bg-muted"
        >
          <Wallet className="h-4 w-4 text-primary" />
          充值
        </Link>
        <Link
          to={userRoutes.tickets}
          className="flex min-h-12 flex-col items-center justify-center gap-1 rounded-xl border border-border bg-card text-xs font-medium active:bg-muted"
        >
          <Ticket className="h-4 w-4 text-primary" />
          工单
        </Link>
      </div>

      {/* Desktop Signature Stat Tiles */}
      <div className="hidden sm:grid gap-4 md:grid-cols-3">
        <div className="rounded-xl border border-border bg-card p-5 flex flex-col justify-between">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              可用余额
            </span>
            <DollarSign className="w-4 h-4 text-muted-foreground" />
          </div>
          <div className="my-3">
            <span className="text-2xl sm:text-3xl font-bold tabular-nums text-foreground">
              ${balance.toFixed(2)}
            </span>
          </div>
          <div className="flex items-center justify-between text-xs text-muted-foreground pt-1 border-t border-border/40">
            <span>实时精算余额</span>
            <Link
              to={userRoutes.financeTopup}
              className="font-medium text-primary hover:underline inline-flex items-center gap-0.5"
            >
              去充值 <ArrowUpRight className="w-3 h-3" />
            </Link>
          </div>
        </div>

        <div className="rounded-xl border border-border bg-card p-5 flex flex-col justify-between">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              总请求次数
            </span>
            <Activity className="w-4 h-4 text-muted-foreground" />
          </div>
          <div className="my-3">
            <span className="text-2xl sm:text-3xl font-bold tabular-nums text-foreground">
              {totalReq.toLocaleString()}
            </span>
          </div>
          <div className="flex items-center justify-between text-xs text-muted-foreground pt-1 border-t border-border/40">
            <span>成功率 {totalReq > 0 ? ((success / totalReq) * 100).toFixed(1) : 100}%</span>
            <span className="tabular-nums">{success.toLocaleString()} 成功</span>
          </div>
        </div>

        <div className="rounded-xl border border-border bg-card p-5 flex flex-col justify-between">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              消耗 Tokens
            </span>
            <Cpu className="w-4 h-4 text-muted-foreground" />
          </div>
          <div className="my-3">
            <span className="text-2xl sm:text-3xl font-bold tabular-nums text-foreground">
              {tokens.toLocaleString()}
            </span>
          </div>
          <div className="flex items-center justify-between text-xs text-muted-foreground pt-1 border-t border-border/40">
            <span>输入 / 输出 / 缓存 / 推理</span>
            <Link
              to={userRoutes.usage}
              className="font-medium text-primary hover:underline inline-flex items-center gap-0.5"
            >
              看明细 <ArrowUpRight className="w-3 h-3" />
            </Link>
          </div>
        </div>
      </div>
    </div>
  )
}
