import { XCircle } from "lucide-react"
import { getStatusInfo, getProviderInfo, fmtAmount, fmtLocalAmount, fmtDateTime } from "../constants"
import type { PaymentOrder } from "../types"

interface Props {
  order: PaymentOrder
  onClose: () => void
}

export function OrderDetailDrawer({ order, onClose }: Props) {
  const sInfo = getStatusInfo(order.status)
  const pInfo = getProviderInfo(order.provider)
  const StatusIcon = sInfo.icon

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 bg-black/40 backdrop-blur-sm" onClick={onClose} />
      <div className="relative w-full max-w-md bg-white dark:bg-dark-900 h-full shadow-2xl border-l border-border flex flex-col animate-in slide-in-from-right duration-300">
        <div className="px-6 py-5 border-b border-border flex items-center justify-between">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white">订单详情</h3>
          <button onClick={onClose} className="h-8 w-8 rounded-lg flex items-center justify-center text-gray-500 hover:bg-gray-100 dark:hover:bg-dark-800 transition-colors">
            <XCircle className="w-5 h-5" />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-6 py-6 space-y-6">
          <div className="flex items-center gap-3">
            <div className={`w-10 h-10 rounded-xl flex items-center justify-center ${sInfo.bg}`}>
              <StatusIcon className={`w-5 h-5 ${sInfo.color}`} />
            </div>
            <div>
              <p className="text-sm font-medium text-gray-900 dark:text-white">{sInfo.label}</p>
              <p className="text-xs text-gray-500 dark:text-dark-400">订单 #{order.id}</p>
            </div>
          </div>

          <div className="rounded-xl border border-border bg-gray-50/50 dark:bg-dark-800/30 p-4 space-y-3">
            <div className="flex justify-between">
              <span className="text-sm text-gray-500 dark:text-dark-400">支付渠道</span>
              <span className={`text-sm font-medium ${pInfo.color}`}>{pInfo.label}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-sm text-gray-500 dark:text-dark-400">USD 金额</span>
              <span className="text-sm font-semibold text-gray-900 dark:text-white tabular-nums">{fmtAmount(order.amount_usd)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-sm text-gray-500 dark:text-dark-400">本地金额</span>
              <span className="text-sm text-gray-900 dark:text-white tabular-nums">{fmtLocalAmount(order.amount_local, order.currency)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-sm text-gray-500 dark:text-dark-400">交易号</span>
              <span className="text-sm text-gray-900 dark:text-white tabular-nums">{order.transaction_id || '-'}</span>
            </div>
            {order.paid_at && (
              <div className="flex justify-between">
                <span className="text-sm text-gray-500 dark:text-dark-400">支付时间</span>
                <span className="text-sm text-gray-900 dark:text-white tabular-nums">{fmtDateTime(order.paid_at)}</span>
              </div>
            )}
            <div className="flex justify-between">
              <span className="text-sm text-gray-500 dark:text-dark-400">创建时间</span>
              <span className="text-sm text-gray-900 dark:text-white tabular-nums">{fmtDateTime(order.created_at)}</span>
            </div>
          </div>

          {order.metadata && (
            <div>
              <p className="text-xs font-medium text-gray-500 dark:text-dark-400 uppercase tracking-wider mb-2">元数据</p>
              <pre className="rounded-lg border border-border bg-gray-50 dark:bg-dark-800 p-3 text-xs text-gray-700 dark:text-gray-300 overflow-x-auto">
                {order.metadata}
              </pre>
            </div>
          )}
        </div>

        <div className="px-6 py-4 border-t border-border">
          <button onClick={onClose} className="btn btn-secondary w-full text-sm">
            关闭
          </button>
        </div>
      </div>
    </div>
  )
}
