import { Link } from 'react-router-dom'
import { Activity, Cpu, DollarSign, Key, Ticket, Wallet } from 'lucide-react'
import { ProgressRing } from '@/shared/components/ui/ProgressRing'
import { Button } from '@/shared/components/ui/button'
import { userRoutes } from '@/shared/routes/user'
import type { UserDashboardHeroProps } from '../types'

export function UserDashboardHero({ email, stats, usageStats }: UserDashboardHeroProps) {
  const balance = stats?.balance || 0
  const totalReq = usageStats?.total_requests || 0
  const success = usageStats?.success_count || 0
  const tokens = usageStats?.total_tokens || 0

  return (
    <div className="space-y-5 sm:space-y-8">
      <div>
        <h2 className="text-xl sm:text-2xl font-bold tracking-tight text-gray-900 dark:text-white">
          欢迎，{email}
        </h2>
        <p className="text-sm sm:text-base text-gray-500 dark:text-dark-400 mt-1">
          额度、请求与常用操作
        </p>
      </div>

      {/* Mobile: balance CTA strip */}
      <div className="sm:hidden rounded-2xl border border-emerald-100 dark:border-emerald-900/40 bg-gradient-to-br from-emerald-50 to-teal-50 dark:from-emerald-950/30 dark:to-teal-950/20 p-4">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="text-xs font-medium text-emerald-700/80 dark:text-emerald-400/80">可用余额</p>
            <p className="text-2xl font-bold tabular-nums text-emerald-800 dark:text-emerald-200 mt-0.5">
              ${balance.toFixed(2)}
            </p>
          </div>
          <Button asChild className="min-h-11 shrink-0">
            <Link to={userRoutes.financeTopup}>
              <Wallet className="h-4 w-4 mr-1.5" />
              去充值
            </Link>
          </Button>
        </div>
        <div className="mt-3 flex gap-4 text-xs text-gray-600 dark:text-dark-300">
          <span>
            请求 <strong className="tabular-nums text-gray-900 dark:text-white">{totalReq}</strong>
            {totalReq > 0 && (
              <span className="text-gray-400"> · {success} 成功</span>
            )}
          </span>
          <span>
            Tokens <strong className="tabular-nums text-gray-900 dark:text-white">{tokens.toLocaleString()}</strong>
          </span>
        </div>
      </div>

      {/* Mobile quick actions */}
      <div className="sm:hidden grid grid-cols-3 gap-2">
        <Link
          to={userRoutes.keys}
          className="flex min-h-14 flex-col items-center justify-center gap-1 rounded-xl border border-border bg-white dark:bg-dark-900 text-xs font-medium active:bg-gray-50 dark:active:bg-dark-800"
        >
          <Key className="h-5 w-5 text-primary-600" />
          密钥
        </Link>
        <Link
          to={userRoutes.financeTopup}
          className="flex min-h-14 flex-col items-center justify-center gap-1 rounded-xl border border-border bg-white dark:bg-dark-900 text-xs font-medium active:bg-gray-50 dark:active:bg-dark-800"
        >
          <Wallet className="h-5 w-5 text-emerald-600" />
          充值
        </Link>
        <Link
          to={userRoutes.tickets}
          className="flex min-h-14 flex-col items-center justify-center gap-1 rounded-xl border border-border bg-white dark:bg-dark-900 text-xs font-medium active:bg-gray-50 dark:active:bg-dark-800"
        >
          <Ticket className="h-5 w-5 text-amber-600" />
          工单
        </Link>
      </div>

      {/* Desktop progress rings */}
      <div className="hidden sm:grid gap-6 md:grid-cols-3">
        <div className="glass-card overflow-hidden">
          <div className="px-6 pt-6 pb-4 border-b border-border/50">
            <h3 className="text-sm font-semibold uppercase tracking-wider text-gray-500 dark:text-dark-300 flex justify-between items-center">
              可用余额
              <DollarSign className="w-5 h-5 opacity-50" />
            </h3>
          </div>
          <div className="p-6 flex justify-center pb-8">
            <ProgressRing
              percentage={100}
              value={`$${balance.toFixed(2)}`}
              label="当前可用"
              gradientFrom="#14b8a6"
              gradientTo="#0d9488"
            />
          </div>
        </div>

        <div className="glass-card overflow-hidden">
          <div className="px-6 pt-6 pb-4 border-b border-border/50">
            <h3 className="text-sm font-semibold uppercase tracking-wider text-gray-500 dark:text-dark-300 flex justify-between items-center">
              总请求次数
              <Activity className="w-5 h-5 opacity-50" />
            </h3>
          </div>
          <div className="p-6 flex justify-center pb-8">
            <ProgressRing
              percentage={success ? (success / (totalReq || 1)) * 100 : 100}
              value={`${totalReq}`}
              label="请求总数"
              subValue={`${success} 成功`}
              gradientFrom="#3b82f6"
              gradientTo="#6366f1"
            />
          </div>
        </div>

        <div className="glass-card overflow-hidden">
          <div className="px-6 pt-6 pb-4 border-b border-border/50">
            <h3 className="text-sm font-semibold uppercase tracking-wider text-gray-500 dark:text-dark-300 flex justify-between items-center">
              消耗 Tokens
              <Cpu className="w-5 h-5 opacity-50" />
            </h3>
          </div>
          <div className="p-6 flex justify-center pb-8">
            <ProgressRing
              percentage={75}
              value={`${tokens.toLocaleString()}`}
              label="处理 Tokens"
              gradientFrom="#f59e0b"
              gradientTo="#f97316"
            />
          </div>
        </div>
      </div>
    </div>
  )
}
