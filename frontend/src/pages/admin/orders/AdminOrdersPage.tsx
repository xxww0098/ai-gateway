import { useState, useEffect, useCallback, useRef } from 'react'
import { fetchApi } from '@/shared/api/client'
import { toast } from 'sonner'
import {
  RefreshCw, Search, ChevronLeft, ChevronRight,
  CheckCircle2, XCircle, Clock, AlertCircle,
  Download, ClipboardList
} from 'lucide-react'
import { EmptyStateRow } from '@/shared/components/EmptyState'

interface PaymentOrder {
  id: number
  user_id: number
  provider: string
  amount_usd: number
  amount_local: number
  currency: string
  status: string
  transaction_id: string | null
  metadata: string | null
  paid_at: string | null
  created_at: string
  updated_at: string
}

const PROVIDER_MAP: Record<string, { label: string; color: string }> = {
  stripe:  { label: 'Stripe', color: 'text-blue-500' },
  alipay:  { label: '支付宝', color: 'text-sky-500' },
  wechat:  { label: '微信支付', color: 'text-emerald-500' },
}

const STATUS_MAP: Record<string, { label: string; color: string; bg: string; icon: React.ElementType }> = {
  pending:  { label: '待支付', color: 'text-amber-600 dark:text-amber-400', bg: 'bg-amber-50 dark:bg-amber-900/20', icon: Clock },
  paid:     { label: '已支付', color: 'text-emerald-600 dark:text-emerald-400', bg: 'bg-emerald-50 dark:bg-emerald-900/20', icon: CheckCircle2 },
  failed:   { label: '失败',   color: 'text-red-600 dark:text-red-400',      bg: 'bg-red-50 dark:bg-red-900/20',      icon: XCircle },
  refunded: { label: '已退款', color: 'text-gray-600 dark:text-gray-400',    bg: 'bg-gray-100 dark:bg-dark-700',      icon: AlertCircle },
}

function getProviderInfo(p: string) {
  return PROVIDER_MAP[p] || { label: p, color: 'text-gray-500' }
}

function getStatusInfo(s: string) {
  return STATUS_MAP[s] || { label: s, color: 'text-gray-500', bg: 'bg-gray-100 dark:bg-dark-700', icon: AlertCircle }
}

function fmtAmount(n: number): string {
  return `$${n.toFixed(2)}`
}

function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

type AdminOrdersProps = {
  /** When true, omit page chrome (embedded in Commerce tabs). */
  embedded?: boolean
}

export default function AdminOrders({ embedded = false }: AdminOrdersProps) {
  const [orders, setOrders] = useState<PaymentOrder[]>([])
  const [loading, setLoading] = useState(true)
  const [page, setPage] = useState(1)
  const [totalPages, setTotalPages] = useState(1)
  const [totalOrders, setTotalOrders] = useState(0)

  const [searchQuery, setSearchQuery] = useState('')
  const [debouncedSearch, setDebouncedSearch] = useState('')
  const [filterStatus, setFilterStatus] = useState('')
  const [filterProvider, setFilterProvider] = useState('')
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const [selectedOrder, setSelectedOrder] = useState<PaymentOrder | null>(null)

  const loadOrders = useCallback(async (silent = false) => {
    if (!silent) setLoading(true)
    try {
      const params = new URLSearchParams()
      params.set('page', page.toString())
      params.set('page_size', '15')
      if (debouncedSearch) params.set('user_id', debouncedSearch)
      if (filterStatus) params.set('status', filterStatus)
      if (filterProvider) params.set('provider', filterProvider)

      const res = await fetchApi(`/admin/orders?${params.toString()}`)
      const d = res.data
      setOrders(Array.isArray(d.items) ? d.items : [])
      setTotalOrders(d.total || 0)
      setTotalPages(Math.max(1, Math.ceil((d.total || 0) / (d.page_size || 15))))
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : '无法加载订单列表')
    } finally {
      setLoading(false)
    }
  }, [page, debouncedSearch, filterStatus, filterProvider])

  useEffect(() => { loadOrders() }, [loadOrders])

  const handleSearchInput = (value: string) => {
    setSearchQuery(value)
    if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current)
    searchTimeoutRef.current = setTimeout(() => {
      setDebouncedSearch(value.trim())
      setPage(1)
    }, 300)
  }

  const handleExport = () => {
    const csvHeader = 'ID,UserID,Provider,AmountUSD,AmountLocal,Currency,Status,TransactionID,PaidAt,CreatedAt\n'
    const csvRows = orders.map(o =>
      [o.id, o.user_id, o.provider, o.amount_usd, o.amount_local, o.currency, o.status, o.transaction_id || '', o.paid_at || '', o.created_at].join(',')
    ).join('\n')
    const blob = new Blob([csvHeader + csvRows], { type: 'text/csv;charset=utf-8;' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `orders_${new Date().toISOString().slice(0, 10)}.csv`
    a.click()
    URL.revokeObjectURL(url)
    toast.success('导出成功')
  }

  return (
    <div
      className={`space-y-6 ${embedded ? '' : 'animate-in fade-in slide-in-from-bottom-4 duration-500'}`}
      style={embedded ? undefined : { willChange: 'transform, opacity' }}
    >
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        {!embedded ? (
          <div>
            <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">订单管理</h2>
            <p className="text-gray-500 dark:text-dark-300 mt-1 max-w-2xl">
              查看和管理所有用户的充值订单，支持按状态、渠道筛选与导出。
            </p>
          </div>
        ) : (
          <p className="text-sm text-gray-500 dark:text-dark-300">
            用户充值订单，支持按状态、渠道筛选与导出。
          </p>
        )}
        <div className="flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400">
          <span className="rounded-md border border-gray-200 dark:border-dark-600 bg-gray-50 dark:bg-dark-800 px-2 py-1 font-medium tabular-nums">
            {loading ? '加载中...' : `${totalOrders} 笔订单`}
          </span>
          <button className="btn btn-secondary h-9 px-3 text-sm" onClick={handleExport}>
            <Download className="h-4 w-4" />
            导出 CSV
          </button>
        </div>
      </div>

      <div className="rounded-xl border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-800/70 shadow-sm p-3">
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-[minmax(200px,1fr)_130px_130px_auto]">
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-400" />
            <input
              type="text"
              className="input h-9 pl-8 text-sm shadow-none"
              placeholder="搜索用户 ID..."
              value={searchQuery}
              onChange={(e) => handleSearchInput(e.target.value)}
            />
          </div>
          <select
            className="input h-9 text-sm shadow-none"
            value={filterStatus}
            onChange={(e) => { setFilterStatus(e.target.value); setPage(1) }}
          >
            <option value="">全部状态</option>
            <option value="pending">待支付</option>
            <option value="paid">已支付</option>
            <option value="failed">失败</option>
            <option value="refunded">已退款</option>
          </select>
          <select
            className="input h-9 text-sm shadow-none"
            value={filterProvider}
            onChange={(e) => { setFilterProvider(e.target.value); setPage(1) }}
          >
            <option value="">全部渠道</option>
            <option value="stripe">Stripe</option>
            <option value="alipay">支付宝</option>
            <option value="wechat">微信支付</option>
          </select>
          <button
            className="btn btn-secondary h-9 px-3 text-sm shadow-none"
            onClick={() => loadOrders(false)}
            disabled={loading}
          >
            <RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
            刷新
          </button>
        </div>
      </div>

      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th>ID</th>
                <th>用户 ID</th>
                <th>渠道</th>
                <th>状态</th>
                <th className="text-right">USD</th>
                <th className="text-right">本地金额</th>
                <th>交易号</th>
                <th className="text-right">创建时间</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={8} className="h-40 text-center">
                    <RefreshCw className="h-5 w-5 animate-spin mx-auto text-primary-500" />
                  </td>
                </tr>
              ) : orders.length === 0 ? (
                debouncedSearch || filterStatus || filterProvider ? (
                  <EmptyStateRow
                    colSpan={8}
                    tone="no-results"
                    icon={ClipboardList}
                    title="未找到匹配的订单"
                    description="请尝试调整搜索关键词，或清除状态与渠道筛选条件。"
                  />
                ) : (
                  <EmptyStateRow
                    colSpan={8}
                    tone="first-use"
                    icon={ClipboardList}
                    title="暂无充值订单"
                    description="用户发起在线充值后将在此生成订单记录。系统核对到账后将自动为用户账户充值。"
                  />
                )
              ) : (
                orders.map((o) => {
                  const sInfo = getStatusInfo(o.status)
                  const pInfo = getProviderInfo(o.provider)
                  const StatusIcon = sInfo.icon
                  return (
                    <tr
                      key={o.id}
                      className="cursor-pointer hover:bg-gray-50 dark:hover:bg-dark-800/50 transition-colors"
                      onClick={() => setSelectedOrder(o)}
                    >
                      <td className="font-mono text-[13px] text-gray-900 dark:text-white">#{o.id}</td>
                      <td className="font-mono text-[13px] text-gray-600 dark:text-gray-400">{o.user_id}</td>
                      <td>
                        <span className={`text-sm font-medium ${pInfo.color}`}>{pInfo.label}</span>
                      </td>
                      <td>
                        <span className={`inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-[11px] font-medium ${sInfo.bg} ${sInfo.color}`}>
                          <StatusIcon className="w-3 h-3" />
                          {sInfo.label}
                        </span>
                      </td>
                      <td className="text-right font-mono font-bold text-[13px] tabular-nums text-gray-900 dark:text-white">
                        {fmtAmount(o.amount_usd)}
                      </td>
                      <td className="text-right font-mono text-[13px] tabular-nums text-gray-600 dark:text-gray-400">
                        {o.amount_local.toFixed(2)} {o.currency}
                      </td>
                      <td>
                        <span className="text-sm text-gray-400 dark:text-dark-500 tabular-nums truncate block max-w-[160px]" title={o.transaction_id || ''}>
                          {o.transaction_id || '-'}
                        </span>
                      </td>
                      <td className="text-right text-[13px] text-gray-500 dark:text-gray-400 tabular-nums">
                        {fmtDateTime(o.created_at)}
                      </td>
                    </tr>
                  )
                })
              )}
            </tbody>
          </table>
        </div>

        {totalPages > 1 && (
          <div className="flex items-center justify-between border-t border-border px-4 py-3">
            <p className="text-xs text-gray-500 dark:text-gray-400 tabular-nums">
              第 {page} / {totalPages} 页 · 共 {totalOrders} 条
            </p>
            <div className="flex items-center gap-1">
              <button
                className="h-8 w-8 rounded-lg border border-gray-200 dark:border-dark-600 flex items-center justify-center text-gray-500 hover:bg-gray-100 dark:hover:bg-dark-700 disabled:opacity-40 transition-colors"
                disabled={page <= 1}
                onClick={() => setPage(p => Math.max(1, p - 1))}
              >
                <ChevronLeft className="h-4 w-4" />
              </button>
              {Array.from({ length: Math.min(5, totalPages) }, (_, i) => {
                let pageNum: number
                if (totalPages <= 5) { pageNum = i + 1 }
                else if (page <= 3) { pageNum = i + 1 }
                else if (page >= totalPages - 2) { pageNum = totalPages - 4 + i }
                else { pageNum = page - 2 + i }
                return (
                  <button
                    key={pageNum}
                    className={`h-8 w-8 rounded-lg text-xs font-medium transition-colors ${
                      pageNum === page
                        ? 'bg-primary-500 text-white shadow-sm'
                        : 'border border-gray-200 dark:border-dark-600 text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-dark-700'
                    }`}
                    onClick={() => setPage(pageNum)}
                  >
                    {pageNum}
                  </button>
                )
              })}
              <button
                className="h-8 w-8 rounded-lg border border-gray-200 dark:border-dark-600 flex items-center justify-center text-gray-500 hover:bg-gray-100 dark:hover:bg-dark-700 disabled:opacity-40 transition-colors"
                disabled={page >= totalPages}
                onClick={() => setPage(p => Math.min(totalPages, p + 1))}
              >
                <ChevronRight className="h-4 w-4" />
              </button>
            </div>
          </div>
        )}
      </div>

      {selectedOrder && (
        <AdminOrderDetailDrawer order={selectedOrder} onClose={() => setSelectedOrder(null)} />
      )}
    </div>
  )
}

function AdminOrderDetailDrawer({ order, onClose }: { order: PaymentOrder; onClose: () => void }) {
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
              <span className="text-sm text-gray-500 dark:text-dark-400">用户 ID</span>
              <span className="text-sm font-medium text-gray-900 dark:text-white font-mono">{order.user_id}</span>
            </div>
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
              <span className="text-sm text-gray-900 dark:text-white tabular-nums">{order.amount_local.toFixed(2)} {order.currency}</span>
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
