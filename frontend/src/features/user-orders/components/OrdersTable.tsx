import { useState, useMemo } from "react"
import { Link } from "react-router-dom"
import {
  RefreshCw,
  ChevronLeft,
  ChevronRight,
  Receipt,
  CreditCard,
  Search,
  X,
  Copy,
  Check,
  FilterX,
  ArrowRight,
  ReceiptText,
  ShieldCheck,
  Zap,
} from "lucide-react"
import { toast } from "sonner"
import { OrderStatusBadge } from "./OrderStatusBadge"
import { getProviderInfo, fmtAmount, fmtLocalAmount, fmtDateTime } from "../constants"
import { userRoutes } from "@/shared/routes/user"
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
  const [searchQuery, setSearchQuery] = useState("")
  const [copiedId, setCopiedId] = useState<string | null>(null)

  const filters = [
    { key: "", label: "全部" },
    { key: "pending", label: "待支付" },
    { key: "paid", label: "已支付" },
    { key: "failed", label: "失败" },
    { key: "refunded", label: "已退款" },
  ]

  const handleCopyTransaction = (e: React.MouseEvent, txId: string) => {
    e.stopPropagation()
    void navigator.clipboard.writeText(txId)
    setCopiedId(txId)
    toast.success("已复制交易单号")
    setTimeout(() => setCopiedId(null), 2000)
  }

  // Filter orders by local search query (transaction id or order id)
  const filteredOrders = useMemo(() => {
    if (!searchQuery.trim()) return orders
    const q = searchQuery.trim().toLowerCase()
    return orders.filter(
      (o) =>
        String(o.id).includes(q) ||
        (o.transaction_id && o.transaction_id.toLowerCase().includes(q)) ||
        o.provider.toLowerCase().includes(q)
    )
  }, [orders, searchQuery])

  const currentFilterLabel = filters.find((f) => f.key === filterStatus)?.label || "全部"
  const isInitialEmpty = !loading && total === 0 && filterStatus === "" && !searchQuery

  return (
    <div className="space-y-4 sm:space-y-6">
      {/* Filter and Search Toolbar */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        {/* Status filter segmented pills */}
        <div className="flex flex-wrap items-center gap-1.5 p-1 rounded-2xl bg-muted/60 border border-border/80 w-fit">
          {filters.map((f) => {
            const active = filterStatus === f.key
            return (
              <button
                key={f.key}
                type="button"
                onClick={() => {
                  onFilter(f.key)
                  setSearchQuery("")
                }}
                className={`min-h-8 px-3.5 py-1 rounded-xl text-xs font-semibold transition-all active:scale-[0.98] ${
                  active
                    ? "bg-background text-foreground shadow-2xs font-bold"
                    : "text-muted-foreground hover:text-foreground hover:bg-background/50"
                }`}
              >
                {f.label}
              </button>
            )
          })}
        </div>

        {/* Right side controls: Search input & Refresh button */}
        <div className="flex items-center gap-2">
          {/* Quick search input */}
          <div className="relative flex-1 sm:w-60">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground pointer-events-none" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="搜索单号 / 交易号..."
              className="h-9 w-full rounded-xl border border-border bg-card pl-8.5 pr-8 text-xs text-foreground placeholder:text-muted-foreground focus:outline-hidden focus:ring-2 focus:ring-primary/20 focus:border-primary transition-all"
            />
            {searchQuery && (
              <button
                type="button"
                onClick={() => setSearchQuery("")}
                className="absolute right-2.5 top-1/2 -translate-y-1/2 p-0.5 rounded text-muted-foreground hover:text-foreground"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            )}
          </div>

          {/* Refresh Button */}
          <button
            type="button"
            onClick={() => onRefresh(page)}
            disabled={loading}
            aria-label="刷新列表"
            className="btn btn-secondary min-h-9 h-9 px-3 text-xs font-semibold shadow-2xs gap-1.5 shrink-0"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin text-primary" : ""}`} />
            <span className="hidden sm:inline">刷新</span>
          </button>
        </div>
      </div>

      {/* Primary Content Area */}
      {isInitialEmpty ? (
        /* First-Use Empty State: Authoritative, Educational & Actionable */
        <div className="rounded-2xl border border-border/80 bg-card p-6 sm:p-10 shadow-xs relative overflow-hidden">
          {/* Subtle decorative glow */}
          <div className="pointer-events-none absolute -right-16 -top-16 h-56 w-56 rounded-full bg-primary/5 blur-3xl" />
          <div className="pointer-events-none absolute -left-16 -bottom-16 h-56 w-56 rounded-full bg-primary/5 blur-3xl" />

          <div className="relative z-10 max-w-2xl mx-auto text-center space-y-6">
            {/* Icon Header */}
            <div className="inline-flex items-center justify-center rounded-2xl h-16 w-16 bg-primary/10 text-primary shadow-xs ring-8 ring-primary/5 mx-auto">
              <Receipt className="h-8 w-8" />
            </div>

            {/* Title & Description */}
            <div className="space-y-2">
              <h3 className="text-xl sm:text-2xl font-bold tracking-tight text-foreground">
                暂无充值订单记录
              </h3>
              <p className="text-sm text-muted-foreground leading-relaxed max-w-lg mx-auto">
                在线充值或兑换完成后，系统将自动在此留存完整的交易流水凭据。支付确认成功后，额度将即时同步注入您的账户余额。
              </p>
            </div>

            {/* Feature Guidance Strip */}
            <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-left pt-2">
              <div className="rounded-xl border border-border/70 bg-muted/30 p-3.5 space-y-1.5">
                <div className="flex items-center gap-1.5 text-xs font-semibold text-foreground">
                  <CreditCard className="w-3.5 h-3.5 text-primary" />
                  <span>多渠道收银</span>
                </div>
                <p className="text-[11px] text-muted-foreground leading-relaxed">
                  支持支付宝、微信支付与 Stripe 在线安全快捷付款。
                </p>
              </div>

              <div className="rounded-xl border border-border/70 bg-muted/30 p-3.5 space-y-1.5">
                <div className="flex items-center gap-1.5 text-xs font-semibold text-foreground">
                  <Zap className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                  <span>秒级自动入账</span>
                </div>
                <p className="text-[11px] text-muted-foreground leading-relaxed">
                  收到支付回调后网关自动核销，余额实时生效可用。
                </p>
              </div>

              <div className="rounded-xl border border-border/70 bg-muted/30 p-3.5 space-y-1.5">
                <div className="flex items-center gap-1.5 text-xs font-semibold text-foreground">
                  <ShieldCheck className="w-3.5 h-3.5 text-indigo-600 dark:text-indigo-400" />
                  <span>凭证透明可溯</span>
                </div>
                <p className="text-[11px] text-muted-foreground leading-relaxed">
                  完整记录交易单号与支付时间，方便随时对账核验。
                </p>
              </div>
            </div>

            {/* Action Buttons */}
            <div className="pt-3 flex flex-wrap items-center justify-center gap-3">
              <Link
                to={userRoutes.financeTopup}
                className="btn btn-primary h-10 px-6 text-sm font-semibold shadow-xs gap-2"
              >
                <CreditCard className="w-4 h-4" />
                立即在线充值
              </Link>
              <Link
                to={userRoutes.finance}
                className="btn btn-secondary h-10 px-5 text-sm font-semibold shadow-2xs gap-2"
              >
                <ReceiptText className="w-4 h-4" />
                查看财务中心
              </Link>
            </div>
          </div>
        </div>
      ) : !loading && filteredOrders.length === 0 ? (
        /* Filter / Search No Results State */
        <div className="rounded-2xl border border-border/80 bg-card p-10 text-center space-y-4 shadow-xs">
          <div className="inline-flex items-center justify-center rounded-full h-12 w-12 bg-muted text-muted-foreground mx-auto">
            <FilterX className="h-6 w-6" />
          </div>
          <div className="space-y-1 max-w-sm mx-auto">
            <p className="text-base font-semibold text-foreground">未找到匹配的订单</p>
            <p className="text-xs text-muted-foreground">
              {searchQuery
                ? `没有找到与「${searchQuery}」相关的订单，请尝试其他关键词。`
                : `当前筛选条件「${currentFilterLabel}」下暂无相关订单记录。`}
            </p>
          </div>
          <button
            type="button"
            onClick={() => {
              onFilter("")
              setSearchQuery("")
            }}
            className="btn btn-secondary h-8 px-4 text-xs font-semibold shadow-2xs"
          >
            重置筛选与搜索
          </button>
        </div>
      ) : (
        /* Data Display: Mobile Cards + Desktop Table */
        <div className="space-y-4">
          {/* Mobile Cards (Visible below md) */}
          <div className="md:hidden space-y-3">
            {loading ? (
              /* Mobile Skeletons */
              Array.from({ length: 3 }).map((_, i) => (
                <div
                  key={i}
                  className="rounded-2xl border border-border bg-card p-4 space-y-3 animate-pulse"
                >
                  <div className="flex justify-between items-center">
                    <div className="h-4 w-20 bg-muted rounded" />
                    <div className="h-5 w-16 bg-muted rounded-lg" />
                  </div>
                  <div className="space-y-1.5">
                    <div className="h-6 w-28 bg-muted rounded" />
                    <div className="h-3.5 w-36 bg-muted rounded" />
                  </div>
                  <div className="pt-2 border-t border-border/60 flex justify-between">
                    <div className="h-3.5 w-24 bg-muted rounded" />
                    <div className="h-3.5 w-28 bg-muted rounded" />
                  </div>
                </div>
              ))
            ) : (
              filteredOrders.map((order) => {
                const pInfo = getProviderInfo(order.provider)
                return (
                  <button
                    key={order.id}
                    type="button"
                    onClick={() => onSelectOrder(order)}
                    className="w-full text-left rounded-2xl border border-border bg-card p-4 shadow-xs hover:border-primary/40 active:bg-muted/50 transition-all space-y-3 block"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-1.5">
                        <span
                          className={`text-xs font-bold px-2 py-0.5 rounded-md border ${pInfo.bg} ${pInfo.border} ${pInfo.color}`}
                        >
                          {pInfo.label}
                        </span>
                        <span className="text-xs font-mono text-muted-foreground tabular-nums">
                          #{order.id}
                        </span>
                      </div>
                      <OrderStatusBadge order={order} size="sm" />
                    </div>

                    <div className="flex items-baseline justify-between">
                      <span className="text-xl font-extrabold tabular-nums tracking-tight text-foreground">
                        {fmtAmount(order.amount_usd)}
                      </span>
                      <span className="text-xs font-medium text-muted-foreground tabular-nums">
                        {fmtLocalAmount(order.amount_local, order.currency)}
                      </span>
                    </div>

                    <div className="flex items-center justify-between text-xs text-muted-foreground pt-2 border-t border-border/60">
                      <span className="tabular-nums font-mono text-[11px]">
                        {fmtDateTime(order.created_at)}
                      </span>
                      {order.transaction_id ? (
                        <span
                          onClick={(e) => handleCopyTransaction(e, order.transaction_id!)}
                          className="font-mono text-[11px] text-foreground hover:text-primary transition-colors inline-flex items-center gap-1 max-w-[140px] truncate"
                        >
                          <span className="truncate">{order.transaction_id}</span>
                          {copiedId === order.transaction_id ? (
                            <Check className="w-3 h-3 text-emerald-500 shrink-0" />
                          ) : (
                            <Copy className="w-3 h-3 text-muted-foreground shrink-0" />
                          )}
                        </span>
                      ) : (
                        <span className="font-mono text-[11px]">-</span>
                      )}
                    </div>
                  </button>
                )
              })
            )}
          </div>

          {/* Desktop Table (Visible on md and above) */}
          <div className="hidden md:block rounded-2xl border border-border/80 bg-card overflow-hidden shadow-2xs">
            <div className="overflow-x-auto">
              <table className="w-full text-left text-sm">
                <thead>
                  <tr className="border-b border-border bg-muted/30 text-xs font-semibold text-muted-foreground">
                    <th className="py-3.5 px-5 w-[160px]">订单 / 渠道</th>
                    <th className="py-3.5 px-4 w-[130px]">状态</th>
                    <th className="py-3.5 px-4 w-[160px]">充值金额 (USD)</th>
                    <th className="py-3.5 px-4 w-[140px]">本地金额</th>
                    <th className="py-3.5 px-4">交易单号</th>
                    <th className="py-3.5 px-4 w-[160px]">创建时间</th>
                    <th className="py-3.5 px-5 w-[80px] text-right">操作</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border/60">
                  {loading ? (
                    /* Desktop Skeleton Rows */
                    Array.from({ length: 5 }).map((_, i) => (
                      <tr key={i} className="animate-pulse">
                        <td className="py-4 px-5">
                          <div className="h-4 w-24 bg-muted rounded" />
                        </td>
                        <td className="py-4 px-4">
                          <div className="h-5 w-16 bg-muted rounded-md" />
                        </td>
                        <td className="py-4 px-4">
                          <div className="h-5 w-20 bg-muted rounded" />
                        </td>
                        <td className="py-4 px-4">
                          <div className="h-4 w-20 bg-muted rounded" />
                        </td>
                        <td className="py-4 px-4">
                          <div className="h-4 w-32 bg-muted rounded" />
                        </td>
                        <td className="py-4 px-4">
                          <div className="h-4 w-28 bg-muted rounded" />
                        </td>
                        <td className="py-4 px-5 text-right">
                          <div className="h-6 w-12 bg-muted rounded ml-auto" />
                        </td>
                      </tr>
                    ))
                  ) : (
                    filteredOrders.map((order) => {
                      const pInfo = getProviderInfo(order.provider)
                      return (
                        <tr
                          key={order.id}
                          onClick={() => onSelectOrder(order)}
                          className="cursor-pointer hover:bg-muted/40 transition-colors group"
                        >
                          {/* Order ID & Provider */}
                          <td className="py-3.5 px-5">
                            <div className="flex items-center gap-2">
                              <span
                                className={`text-xs font-bold px-2 py-0.5 rounded-md border ${pInfo.bg} ${pInfo.border} ${pInfo.color}`}
                              >
                                {pInfo.label}
                              </span>
                              <span className="font-mono text-xs text-muted-foreground tabular-nums">
                                #{order.id}
                              </span>
                            </div>
                          </td>

                          {/* Status */}
                          <td className="py-3.5 px-4">
                            <OrderStatusBadge order={order} size="sm" />
                          </td>

                          {/* Amount USD */}
                          <td className="py-3.5 px-4">
                            <span className="font-bold text-foreground tabular-nums tracking-tight">
                              {fmtAmount(order.amount_usd)}
                            </span>
                          </td>

                          {/* Amount Local */}
                          <td className="py-3.5 px-4">
                            <span className="text-xs text-muted-foreground font-medium tabular-nums">
                              {fmtLocalAmount(order.amount_local, order.currency)}
                            </span>
                          </td>

                          {/* Transaction ID */}
                          <td className="py-3.5 px-4">
                            {order.transaction_id ? (
                              <div className="inline-flex items-center gap-1.5 max-w-[220px]">
                                <span
                                  className="font-mono text-xs text-muted-foreground group-hover:text-foreground transition-colors truncate"
                                  title={order.transaction_id}
                                >
                                  {order.transaction_id}
                                </span>
                                <button
                                  type="button"
                                  onClick={(e) => handleCopyTransaction(e, order.transaction_id!)}
                                  className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors shrink-0"
                                  title="复制交易单号"
                                >
                                  {copiedId === order.transaction_id ? (
                                    <Check className="w-3 h-3 text-emerald-500" />
                                  ) : (
                                    <Copy className="w-3 h-3" />
                                  )}
                                </button>
                              </div>
                            ) : (
                              <span className="font-mono text-xs text-muted-foreground">-</span>
                            )}
                          </td>

                          {/* Created Time */}
                          <td className="py-3.5 px-4">
                            <span className="font-mono text-xs text-muted-foreground tabular-nums">
                              {fmtDateTime(order.created_at)}
                            </span>
                          </td>

                          {/* Action */}
                          <td className="py-3.5 px-5 text-right">
                            <span className="inline-flex items-center gap-1 text-xs font-semibold text-muted-foreground group-hover:text-primary transition-colors">
                              详情
                              <ArrowRight className="w-3 h-3 transition-transform group-hover:translate-x-0.5" />
                            </span>
                          </td>
                        </tr>
                      )
                    })
                  )}
                </tbody>
              </table>
            </div>

            {/* Desktop Pagination */}
            {total > 0 && (
              <div className="px-5 py-3 border-t border-border flex items-center justify-between bg-muted/20">
                <div className="text-xs text-muted-foreground tabular-nums font-medium">
                  共 <span className="font-semibold text-foreground">{total}</span> 笔充值订单 · 第{" "}
                  {page} / {totalPages || 1} 页
                </div>
                <div className="flex items-center gap-1.5">
                  <button
                    type="button"
                    disabled={page <= 1 || loading}
                    onClick={() => {
                      onPageChange(page - 1)
                      onRefresh(page - 1)
                    }}
                    className="h-8 px-2.5 rounded-lg border border-border bg-card flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground hover:bg-muted disabled:opacity-40 transition-colors shadow-2xs"
                  >
                    <ChevronLeft className="w-3.5 h-3.5" />
                    <span>上一页</span>
                  </button>
                  <span className="px-3 text-xs text-muted-foreground tabular-nums font-semibold">
                    {page} / {totalPages || 1}
                  </span>
                  <button
                    type="button"
                    disabled={page >= totalPages || loading}
                    onClick={() => {
                      onPageChange(page + 1)
                      onRefresh(page + 1)
                    }}
                    className="h-8 px-2.5 rounded-lg border border-border bg-card flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground hover:bg-muted disabled:opacity-40 transition-colors shadow-2xs"
                  >
                    <span>下一页</span>
                    <ChevronRight className="w-3.5 h-3.5" />
                  </button>
                </div>
              </div>
            )}
          </div>

          {/* Mobile Pagination */}
          {total > 0 && (
            <div className="md:hidden flex items-center justify-between text-xs text-muted-foreground pt-1">
              <span className="tabular-nums font-medium">
                共 {total} 笔 · 第 {page}/{totalPages || 1} 页
              </span>
              <div className="flex gap-1.5">
                <button
                  type="button"
                  disabled={page <= 1 || loading}
                  onClick={() => {
                    onPageChange(page - 1)
                    onRefresh(page - 1)
                  }}
                  className="h-8 px-3 rounded-lg border border-border bg-card flex items-center justify-center disabled:opacity-40 shadow-2xs"
                >
                  <ChevronLeft className="w-4 h-4" />
                </button>
                <button
                  type="button"
                  disabled={page >= totalPages || loading}
                  onClick={() => {
                    onPageChange(page + 1)
                    onRefresh(page + 1)
                  }}
                  className="h-8 px-3 rounded-lg border border-border bg-card flex items-center justify-center disabled:opacity-40 shadow-2xs"
                >
                  <ChevronRight className="w-4 h-4" />
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

