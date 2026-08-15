import { useState, useEffect, useCallback } from 'react'
import { fetchApi } from '@/shared/api/client'
import { toast } from 'sonner'
import {
  Wallet, ArrowUpRight, RefreshCw,
  ChevronLeft, ChevronRight, Gift, Settings2, Coins, FileText
} from 'lucide-react'

interface BalanceEntry {
  id: number
  kind?: string
  type?: string
  amount: number
  balance_before?: number
  balance_after?: number
  operator_email?: string | null
  note?: string | null
  reference?: string | null
  created_at: string
}

const KIND_MAP: Record<string, { label: string; color: string; icon: React.ElementType }> = {
  deposit:    { label: '管理员充值', color: 'text-emerald-500', icon: ArrowUpRight },
  redeem:     { label: '兑换码充值', color: 'text-emerald-500', icon: Gift },
  initial:    { label: '初始余额',   color: 'text-blue-500',    icon: Coins },
  usage:      { label: 'API 调用',   color: 'text-orange-500',  icon: Coins },
  adjustment: { label: '管理员调账', color: 'text-purple-500',  icon: Settings2 },
}

function getKindInfo(kind: string) {
  return KIND_MAP[kind] || { label: kind, color: 'text-gray-500', icon: FileText }
}

function fmtAmount(n: number): string {
  const prefix = n >= 0 ? '+' : ''
  return `${prefix}$${n.toFixed(4)}`
}

function fmtBalance(n: number | undefined | null): string {
  if (n === undefined || n === null) return '-'
  return `$${n.toFixed(4)}`
}

function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

type BalanceHistoryProps = {
  /** When true, omit page-level chrome (used inside Finance tabs). */
  embedded?: boolean
}

export default function BalanceHistory({ embedded = false }: BalanceHistoryProps) {
  const [entries, setEntries] = useState<BalanceEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [page, setPage] = useState(1)
  const [pageSize] = useState(20)
  const [total, setTotal] = useState(0)
  const [filterKind, setFilterKind] = useState('')

  const loadData = useCallback(async (p = page) => {
    setLoading(true)
    try {
      const params = new URLSearchParams()
      params.set('page', String(p))
      params.set('page_size', String(pageSize))
      if (filterKind) params.set('kind', filterKind)

      const res = await fetchApi(`/user/balance-history?${params}`)
      if (res?.data) {
        setEntries(res.data.items || [])
        setTotal(res.data.total || 0)
        setPage(res.data.page || p)
      }
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '加载余额记录失败')
    } finally {
      setLoading(false)
    }
  }, [page, pageSize, filterKind])

  useEffect(() => { loadData(1) }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const handleFilter = (kind: string) => {
    setFilterKind(kind)
    setPage(1)
    setTimeout(() => loadData(1), 0)
  }

  const totalPages = Math.ceil(total / pageSize)
  const handlePage = (p: number) => { setPage(p); loadData(p) }

  return (
    <div
      className={`space-y-6 ${embedded ? '' : 'animate-in fade-in slide-in-from-bottom-4 duration-500'}`}
      style={embedded ? undefined : { willChange: 'transform, opacity' }}
    >
      {!embedded && (
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">余额流水</h2>
          <p className="text-gray-500 dark:text-dark-300 mt-1">查看每笔余额变动的详细记录，包括充值、扣费、退款等。</p>
        </div>
      )}

      {/* Filter bar */}
      <div className="flex flex-wrap items-center gap-2">
        {[
          { key: '', label: '全部' },
          { key: 'deposit', label: '充值' },
          { key: 'redeem', label: '兑换' },
          { key: 'usage', label: 'API 扣费' },
          { key: 'adjustment', label: '调账' },
        ].map(f => (
          <button
            key={f.key}
            onClick={() => handleFilter(f.key)}
            className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-all ${
              filterKind === f.key
                ? 'bg-primary-500 text-white shadow-sm'
                : 'bg-gray-100 dark:bg-dark-800 text-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-dark-700'
            }`}
          >
            {f.label}
          </button>
        ))}
        <button
          onClick={() => loadData(page)}
          disabled={loading}
          className="ml-auto btn btn-secondary h-8 px-3 text-xs"
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
        ) : entries.length === 0 ? (
          <div className="glass-card flex h-32 flex-col items-center justify-center gap-2 text-gray-400 text-sm">
            <Wallet className="w-10 h-10 opacity-30" />
            暂无余额变动记录
          </div>
        ) : (
          entries.map((entry) => {
            const kind = entry.kind ?? entry.type ?? 'unknown'
            const info = getKindInfo(kind)
            const Icon = info.icon
            const isPositive = entry.amount >= 0
            return (
              <div
                key={entry.id}
                className="rounded-xl border border-border bg-white dark:bg-dark-900 p-3 shadow-sm space-y-2"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2 min-w-0">
                    <div
                      className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 ${
                        isPositive
                          ? 'bg-emerald-50 dark:bg-emerald-900/20'
                          : 'bg-orange-50 dark:bg-orange-900/20'
                      }`}
                    >
                      <Icon className={`w-4 h-4 ${info.color}`} />
                    </div>
                    <span className="text-sm font-medium truncate">{info.label}</span>
                  </div>
                  <span
                    className={`text-sm font-bold tabular-nums shrink-0 ${
                      isPositive
                        ? 'text-emerald-600 dark:text-emerald-400'
                        : 'text-orange-600 dark:text-orange-400'
                    }`}
                  >
                    {fmtAmount(entry.amount)}
                  </span>
                </div>
                <div className="flex justify-between text-[11px] text-muted-foreground tabular-nums">
                  <span>{fmtDateTime(entry.created_at)}</span>
                  <span>
                    {fmtBalance(entry.balance_before)} → {fmtBalance(entry.balance_after)}
                  </span>
                </div>
                {(entry.note || entry.reference) && (
                  <p className="text-xs text-muted-foreground line-clamp-2">
                    {entry.note || entry.reference}
                  </p>
                )}
              </div>
            )
          })
        )}
        {total > 0 && (
          <div className="flex items-center justify-between text-xs text-muted-foreground pt-1">
            <span className="tabular-nums">
              共 {total} · {page}/{totalPages}
            </span>
            <div className="flex gap-1.5">
              <button
                type="button"
                disabled={page <= 1}
                onClick={() => handlePage(page - 1)}
                className="h-9 w-9 rounded-lg border border-border flex items-center justify-center disabled:opacity-30"
              >
                <ChevronLeft className="w-4 h-4" />
              </button>
              <button
                type="button"
                disabled={page >= totalPages}
                onClick={() => handlePage(page + 1)}
                className="h-9 w-9 rounded-lg border border-border flex items-center justify-center disabled:opacity-30"
              >
                <ChevronRight className="w-4 h-4" />
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Desktop table */}
      <div className="glass-card overflow-hidden hidden md:block">
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th className="w-[160px]">时间</th>
                <th className="w-[130px]">类型</th>
                <th className="w-[120px]">变动金额</th>
                <th className="w-[120px]">变动前余额</th>
                <th className="w-[120px]">变动后余额</th>
                <th>备注</th>
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
              ) : entries.length === 0 ? (
                <tr>
                  <td colSpan={6} className="h-40 text-center text-gray-400 dark:text-dark-500">
                    <div className="flex flex-col items-center gap-2">
                      <Wallet className="w-10 h-10 opacity-30" />
                      <span>暂无余额变动记录</span>
                    </div>
                  </td>
                </tr>
              ) : (
                entries.map((entry) => {
                  const kind = entry.kind ?? entry.type ?? 'unknown'
                  const info = getKindInfo(kind)
                  const Icon = info.icon
                  const isPositive = entry.amount >= 0
                  return (
                    <tr key={entry.id}>
                      <td>
                        <span className="text-sm text-gray-500 dark:text-gray-400 tabular-nums">
                          {fmtDateTime(entry.created_at)}
                        </span>
                      </td>
                      <td>
                        <div className="flex items-center gap-2">
                          <div
                            className={`w-7 h-7 rounded-lg flex items-center justify-center ${
                              isPositive
                                ? 'bg-emerald-50 dark:bg-emerald-900/20'
                                : 'bg-orange-50 dark:bg-orange-900/20'
                            }`}
                          >
                            <Icon className={`w-3.5 h-3.5 ${info.color}`} />
                          </div>
                          <span className="text-sm font-medium text-gray-900 dark:text-white">
                            {info.label}
                          </span>
                        </div>
                      </td>
                      <td>
                        <span
                          className={`text-sm font-semibold tabular-nums ${
                            isPositive
                              ? 'text-emerald-600 dark:text-emerald-400'
                              : 'text-orange-600 dark:text-orange-400'
                          }`}
                        >
                          {fmtAmount(entry.amount)}
                        </span>
                      </td>
                      <td>
                        <span className="text-sm text-gray-500 dark:text-gray-400 tabular-nums">
                          {fmtBalance(entry.balance_before)}
                        </span>
                      </td>
                      <td>
                        <span className="text-sm font-medium text-gray-900 dark:text-white tabular-nums">
                          {fmtBalance(entry.balance_after)}
                        </span>
                      </td>
                      <td>
                        <span
                          className="text-sm text-gray-500 dark:text-gray-400 truncate block max-w-[300px]"
                          title={entry.note || entry.reference || ''}
                        >
                          {entry.note || entry.reference || '-'}
                          {entry.operator_email && (
                            <span className="text-xs text-gray-400 dark:text-dark-500 ml-1">
                              ({entry.operator_email})
                            </span>
                          )}
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
                onClick={() => handlePage(page - 1)}
                className="h-9 w-9 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center text-gray-500 hover:bg-gray-50 dark:hover:bg-dark-800 disabled:opacity-30 transition-colors"
              >
                <ChevronLeft className="w-4 h-4" />
              </button>
              <span className="px-3 text-xs text-gray-600 dark:text-gray-400 tabular-nums">
                {page} / {totalPages}
              </span>
              <button
                type="button"
                disabled={page >= totalPages}
                onClick={() => handlePage(page + 1)}
                className="h-9 w-9 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center text-gray-500 hover:bg-gray-50 dark:hover:bg-dark-800 disabled:opacity-30 transition-colors"
              >
                <ChevronRight className="w-4 h-4" />
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
