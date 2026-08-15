import { RefreshCw, ChevronLeft, ChevronRight, ShoppingCart } from "lucide-react"
import { OrderStatusBadge } from "./OrderStatusBadge"
import { getProviderInfo, fmtAmount, fmtLocalAmount, fmtDateTime } from "../constants"
import type { PaymentOrder } from "../types"

interface Props {
  orders: PaymentOrder[]
  loading: boolean
  page: number
  totalPages: number
  total: number
  filterStatus: string
  onFilter: (status: string) => void
  onPageChange: (p: number) => void
  onRefresh: (p: number) => void
  onSelectOrder: (order: PaymentOrder) => void
}

export function OrdersTable({
  orders,
  loading,
  page,
  totalPages,
  total,
  filterStatus,
  onFilter,
  onPageChange,
  onRefresh,
  onSelectOrder,
}: Props) {
  const filters = [
    { key: '', label: '全部' },
    { key: 'pending', label: '待支付' },
    { key: 'paid', label: '已支付' },
    { key: 'failed', label: '失败' },
    { key: 'refunded', label: '已退款' },
  ]

  return (
    <div className="space-y-4 sm:space-y-6">
      <div className="flex flex-wrap items-center gap-2">
        {filters.map(f => (
          <button
            key={f.key}
            type="button"
            onClick={() => onFilter(f.key)}
            className={`min-h-9 px-3 py-1.5 rounded-lg text-xs font-medium transition-all ${
              filterStatus === f.key
                ? 'bg-primary-500 text-white shadow-sm'
                : 'bg-gray-100 dark:bg-dark-800 text-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-dark-700'
            }`}
          >
            {f.label}
          </button>
        ))}
        <button
          type="button"
          onClick={() => onRefresh(page)}
          disabled={loading}
          className="ml-auto btn btn-secondary min-h-9 h-9 px-3 text-xs"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
          刷新
        </button>
      </div>

      {/* Mobile cards */}
      <div className="md:hidden space-y-3">
        {loading ? (
          <div className="glass-card flex h-32 items-center justify-center gap-2 text-gray-400 text-sm">
            <RefreshCw className="w-4 h-4 animate-spin text-primary-500" />
            加载中...
          </div>
        ) : orders.length === 0 ? (
          <div className="glass-card flex h-32 flex-col items-center justify-center gap-2 text-gray-400 text-sm">
            <ShoppingCart className="w-10 h-10 opacity-30" />
            暂无充值订单记录
          </div>
        ) : (
          orders.map((order) => {
            const pInfo = getProviderInfo(order.provider)
            return (
              <button
                key={order.id}
                type="button"
                onClick={() => onSelectOrder(order)}
                className="w-full text-left rounded-xl border border-border bg-white dark:bg-dark-900 p-3 shadow-sm active:bg-gray-50 dark:active:bg-dark-800 space-y-2"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className={`text-sm font-medium ${pInfo.color}`}>{pInfo.label}</span>
                  <OrderStatusBadge order={order} />
                </div>
                <div className="flex items-baseline justify-between">
                  <span className="text-lg font-bold tabular-nums text-gray-900 dark:text-white">
                    {fmtAmount(order.amount_usd)}
                  </span>
                  <span className="text-xs text-muted-foreground tabular-nums">
                    {fmtLocalAmount(order.amount_local, order.currency)}
                  </span>
                </div>
                <div className="text-[11px] text-muted-foreground tabular-nums">
                  {fmtDateTime(order.created_at)}
                  {order.transaction_id ? ` · ${order.transaction_id.slice(0, 12)}…` : ''}
                </div>
              </button>
            )
          })
        )}
      </div>

      {/* Desktop table */}
      <div className="glass-card overflow-hidden hidden md:block">
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th className="w-[140px]">时间</th>
                <th className="w-[100px]">渠道</th>
                <th className="w-[100px]">状态</th>
                <th className="w-[120px]">金额 (USD)</th>
                <th className="w-[140px]">本地金额</th>
                <th>交易号</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={6} className="h-40 text-center">
                    <div className="flex items-center justify-center gap-2 text-gray-400">
                      <RefreshCw className="w-4 h-4 animate-spin text-primary-500" />
                      加载中...
                    </div>
                  </td>
                </tr>
              ) : orders.length === 0 ? (
                <tr>
                  <td colSpan={6} className="h-40 text-center text-gray-400 dark:text-dark-500">
                    <div className="flex flex-col items-center gap-2">
                      <ShoppingCart className="w-10 h-10 opacity-30" />
                      <span>暂无充值订单记录</span>
                    </div>
                  </td>
                </tr>
              ) : (
                orders.map(order => {
                  const pInfo = getProviderInfo(order.provider)
                  return (
                    <tr
                      key={order.id}
                      className="cursor-pointer hover:bg-gray-50 dark:hover:bg-dark-800/50 transition-colors"
                      onClick={() => onSelectOrder(order)}
                    >
                      <td>
                        <span className="text-sm text-gray-500 dark:text-gray-400 tabular-nums">{fmtDateTime(order.created_at)}</span>
                      </td>
                      <td>
                        <span className={`text-sm font-medium ${pInfo.color}`}>{pInfo.label}</span>
                      </td>
                      <td><OrderStatusBadge order={order} /></td>
                      <td>
                        <span className="text-sm font-semibold tabular-nums text-gray-900 dark:text-white">{fmtAmount(order.amount_usd)}</span>
                      </td>
                      <td>
                        <span className="text-sm text-gray-500 dark:text-gray-400 tabular-nums">{fmtLocalAmount(order.amount_local, order.currency)}</span>
                      </td>
                      <td>
                        <span className="text-sm text-gray-400 dark:text-dark-500 tabular-nums truncate block max-w-[200px]" title={order.transaction_id || ''}>
                          {order.transaction_id || '-'}
                        </span>
                      </td>
                    </tr>
                  )
                })
              )}
            </tbody>
          </table>
        </div>

        {total > 0 && (
          <div className="px-5 py-3 border-t border-border flex items-center justify-between bg-gray-50/50 dark:bg-dark-800/30">
            <div className="text-xs text-gray-500 dark:text-dark-400 tabular-nums">
              共 {total} 条记录 · 第 {page}/{totalPages} 页
            </div>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                disabled={page <= 1}
                onClick={() => { onPageChange(page - 1); onRefresh(page - 1) }}
                className="h-9 w-9 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center text-gray-500 hover:bg-gray-50 dark:hover:bg-dark-800 disabled:opacity-30 transition-colors"
              >
                <ChevronLeft className="w-4 h-4" />
              </button>
              <span className="px-3 text-xs text-gray-600 dark:text-gray-400 tabular-nums">{page} / {totalPages}</span>
              <button
                type="button"
                disabled={page >= totalPages}
                onClick={() => { onPageChange(page + 1); onRefresh(page + 1) }}
                className="h-9 w-9 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center text-gray-500 hover:bg-gray-50 dark:hover:bg-dark-800 disabled:opacity-30 transition-colors"
              >
                <ChevronRight className="w-4 h-4" />
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Mobile pagination */}
      {total > 0 && (
        <div className="md:hidden flex items-center justify-between text-xs text-muted-foreground">
          <span className="tabular-nums">共 {total} · {page}/{totalPages}</span>
          <div className="flex gap-1.5">
            <button
              type="button"
              disabled={page <= 1}
              onClick={() => { onPageChange(page - 1); onRefresh(page - 1) }}
              className="h-9 w-9 rounded-lg border border-border flex items-center justify-center disabled:opacity-30"
            >
              <ChevronLeft className="w-4 h-4" />
            </button>
            <button
              type="button"
              disabled={page >= totalPages}
              onClick={() => { onPageChange(page + 1); onRefresh(page + 1) }}
              className="h-9 w-9 rounded-lg border border-border flex items-center justify-center disabled:opacity-30"
            >
              <ChevronRight className="w-4 h-4" />
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
