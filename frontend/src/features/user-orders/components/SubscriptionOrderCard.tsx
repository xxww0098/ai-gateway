import { useNavigate } from "react-router-dom"
import { Card, CardContent } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Clock, ArrowLeftRight, AlertCircle, CalendarDays } from "lucide-react"
import { SubscriptionStatusBadge } from "./SubscriptionStatusBadge"
import { calculateRefund } from "../constants"
import type { Subscription } from "../types"
import { userRefundApplyPath } from "@/shared/routes/user"

interface Props {
  sub: Subscription
  isRefundable: boolean
  hasPendingRefund: boolean
  hasCompletedRefund: boolean
  remainingDays: number
}

export function SubscriptionOrderCard({ sub, isRefundable, hasPendingRefund, hasCompletedRefund, remainingDays }: Props) {
  const navigate = useNavigate()
  const refundAmount = calculateRefund(sub)

  return (
    <Card
      className={`relative overflow-hidden group/card transition-all hover:shadow-lg flex flex-col ${
        sub.status === 'refunded'
          ? 'border-blue-100 dark:border-blue-900/30 bg-blue-50/20 dark:bg-blue-950/10'
          : 'border-gray-100 dark:border-dark-800 bg-white dark:bg-dark-900'
      }`}
    >
      <div
        className={`absolute top-0 inset-x-0 h-1 ${
          sub.status === 'active' && !hasPendingRefund && !hasCompletedRefund
            ? 'bg-gradient-to-r from-emerald-400 to-teal-500'
            : sub.status === 'refunded' || hasPendingRefund || hasCompletedRefund
            ? 'bg-gradient-to-r from-blue-400 to-indigo-500'
            : 'bg-gradient-to-r from-gray-300 to-gray-400'
        }`}
      />
      <CardContent className="p-6 flex-1 flex flex-col">
        <div className="flex justify-between items-start mb-4">
          <div className="space-y-1">
            <h4 className="text-lg font-bold text-gray-900 dark:text-white">
              {sub.group_name || `订阅 #${sub.id}`}
            </h4>
            {sub.status === 'active' && remainingDays > 0 && (
              <div className="flex items-center gap-1.5 text-sm font-medium text-emerald-600 dark:text-emerald-500">
                <Clock className="w-4 h-4" />
                剩余 {remainingDays} 天
              </div>
            )}
          </div>
          <SubscriptionStatusBadge status={sub.status} />
        </div>

        <div className="space-y-3 mb-4 flex-1">
          <div className="flex justify-between text-sm">
            <span className="text-gray-500">订单金额</span>
            <span className="font-semibold text-gray-900 dark:text-white">
              ${sub.price_paid?.toFixed(2) || '0.00'}
            </span>
          </div>
          <div className="flex justify-between text-sm">
            <span className="text-gray-500">生效日期</span>
            <span className="text-gray-700 dark:text-gray-300">
              {new Date(sub.starts_at).toLocaleDateString()}
            </span>
          </div>
          <div className="flex justify-between text-sm">
            <span className="text-gray-500">到期日期</span>
            <span className="text-gray-700 dark:text-gray-300">
              {new Date(sub.expires_at).toLocaleDateString()}
            </span>
          </div>

          {isRefundable && (
            <div className="bg-emerald-50 dark:bg-emerald-950/20 border border-emerald-100 dark:border-emerald-900/30 rounded-lg p-3 mt-2">
              <div className="flex items-center gap-2 text-emerald-700 dark:text-emerald-400 text-sm font-medium">
                <ArrowLeftRight className="w-4 h-4" />
                可退金额
              </div>
              <div className="text-2xl font-bold text-emerald-600 dark:text-emerald-400 mt-1">
                ${refundAmount.toFixed(2)}
              </div>
              <p className="text-xs text-emerald-600/70 dark:text-emerald-400/70 mt-1">
                基于剩余 {remainingDays} 天计算
              </p>
            </div>
          )}

          {hasPendingRefund && (
            <div className="bg-blue-50 dark:bg-blue-950/20 border border-blue-100 dark:border-blue-900/30 rounded-lg p-3 mt-2 flex items-center gap-2">
              <AlertCircle className="w-4 h-4 text-blue-500" />
              <span className="text-sm text-blue-700 dark:text-blue-400">
                已申请退款，请前往退款记录查看进度
              </span>
            </div>
          )}

          {hasCompletedRefund && sub.status !== 'refunded' && (
            <div className="bg-blue-50 dark:bg-blue-950/20 border border-blue-100 dark:border-blue-900/30 rounded-lg p-3 mt-2 flex items-center gap-2">
              <AlertCircle className="w-4 h-4 text-blue-500" />
              <span className="text-sm text-blue-700 dark:text-blue-400">
                退款已通过，请在退款记录中查看处理结果
              </span>
            </div>
          )}

          {sub.status === 'refunded' && (
            <div className="bg-blue-50 dark:bg-blue-950/20 border border-blue-100 dark:border-blue-900/30 rounded-lg p-3 mt-2 flex items-center gap-2">
              <AlertCircle className="w-4 h-4 text-blue-500" />
              <span className="text-sm text-blue-700 dark:text-blue-400">
                该订单已退款
              </span>
            </div>
          )}
        </div>

        <div className="pt-4 border-t border-gray-100 dark:border-dark-800 flex items-center justify-between">
          <div className="flex items-center gap-1 text-xs text-gray-400">
            <CalendarDays className="w-3.5 h-3.5" />
            <span>ID: {sub.id}</span>
          </div>
          {isRefundable ? (
            <Button
              size="sm"
              variant="outline"
              className="gap-1 border-emerald-200 text-emerald-700 hover:bg-emerald-50 hover:text-emerald-800 dark:border-emerald-800 dark:text-emerald-400 dark:hover:bg-emerald-950/30"
              onClick={() => navigate(userRefundApplyPath(sub.id))}
            >
              <ArrowLeftRight className="w-3.5 h-3.5" />
              申请退订
            </Button>
          ) : (
            <Button size="sm" variant="ghost" disabled className="text-gray-400 cursor-not-allowed">
              不可退订
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
