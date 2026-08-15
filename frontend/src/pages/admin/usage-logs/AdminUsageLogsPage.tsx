import { useState, useCallback, useMemo } from 'react'
import { useAuthStore } from '@/features/auth/auth_store'
import { useAdminUsageLogs, useInvalidateUsageLogs } from '@/features/usage/hooks'
import { fetchAdminUsageLogs } from '@/features/usage/api'
import type { AdminUsageLog, AdminUsageLogsFilter, DateRangePreset } from '@/features/usage/types'
import { QueryStateWrapper } from '@/shared/components/QueryStateWrapper'
import { toast } from 'sonner'
import {
  Search, RefreshCw, Download,
  ChevronLeft, ChevronRight, CheckCircle2, XCircle,
  ArrowDownCircle, ArrowUpCircle, X, Info, Database, Brain
} from 'lucide-react'

function fmtDuration(ms: number | null): string {
  if (ms == null) return '-'
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

function fmtCost(n: number): string {
  if (n === 0) return '$0.00'
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(4)}`
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${pad(d.getMonth()+1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

function todayStr(): string {
  const d = new Date()
  return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`
}

function daysAgo(n: number): string {
  const d = new Date(Date.now() - n * 86400000)
  return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`
}

function getDateRange(preset: DateRangePreset): { startDate: string; endDate: string } {
  const endDate = todayStr()
  let startDate: string
  if (preset === 'today') startDate = todayStr()
  else if (preset === '7d') startDate = daysAgo(6)
  else startDate = daysAgo(29)
  return { startDate, endDate }
}

// Token detail tooltip
interface TokenTooltipData { log: AdminUsageLog; x: number; y: number }

function TokenTooltip({ data }: { data: TokenTooltipData }) {
  const { log } = data
  const totalTokens = log.input_tokens + log.output_tokens
  return (
    <div className="fixed z-[9999] pointer-events-none" style={{ left: data.x + 12, top: data.y - 20 }}>
      <div className="whitespace-nowrap rounded-xl border border-gray-700/80 bg-gray-900/95 backdrop-blur-xl px-4 py-3 text-xs text-white shadow-2xl min-w-[260px]">
        <div className="text-[10px] font-bold uppercase tracking-wider text-gray-400 mb-2">Token 明细</div>
        <div className="space-y-1.5">
          <div className="flex justify-between gap-6">
            <span className="text-gray-400 flex items-center gap-1.5"><ArrowDownCircle className="w-3 h-3 text-emerald-400" /> 输入 Tokens</span>
            <span className="font-medium text-white tabular-nums">{log.input_tokens.toLocaleString()}</span>
          </div>
          <div className="flex justify-between gap-6">
            <span className="text-gray-400 flex items-center gap-1.5"><ArrowUpCircle className="w-3 h-3 text-violet-400" /> 输出 Tokens</span>
            <span className="font-medium text-white tabular-nums">{log.output_tokens.toLocaleString()}</span>
          </div>
          {log.reasoning_tokens > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400 flex items-center gap-1.5"><Brain className="w-3 h-3 text-amber-400" /> 推理 Tokens</span>
              <span className="font-medium text-amber-300 tabular-nums">{log.reasoning_tokens.toLocaleString()}</span>
            </div>
          )}
          {log.cached_tokens > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400 flex items-center gap-1.5"><Database className="w-3 h-3 text-sky-400" /> 缓存命中</span>
              <span className="font-medium text-sky-300 tabular-nums">{log.cached_tokens.toLocaleString()}</span>
            </div>
          )}
          <div className="border-t border-gray-700 pt-1.5 flex justify-between gap-6">
            <span className="text-gray-400">总 Tokens</span>
            <span className="font-semibold text-blue-400 tabular-nums">{totalTokens.toLocaleString()}</span>
          </div>
          {log.cached_tokens > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">缓存命中率</span>
              <span className="font-semibold text-sky-400 tabular-nums">
                {((log.cached_tokens / (log.input_tokens || 1)) * 100).toFixed(1)}%
              </span>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

// Cost tooltip
interface CostTooltipData { log: AdminUsageLog; x: number; y: number }

function CostTooltip({ data }: { data: CostTooltipData }) {
  const { log } = data
  return (
    <div className="fixed z-[9999] pointer-events-none" style={{ left: data.x + 12, top: data.y - 20 }}>
      <div className="whitespace-nowrap rounded-xl border border-gray-700/80 bg-gray-900/95 backdrop-blur-xl px-4 py-3 text-xs text-white shadow-2xl min-w-[230px]">
        <div className="text-[10px] font-bold uppercase tracking-wider text-gray-400 mb-2">费用明细</div>
        <div className="space-y-1.5">
          {log.input_cost > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">输入费用</span>
              <span className="font-medium text-emerald-300 tabular-nums">{fmtCost(log.input_cost)}</span>
            </div>
          )}
          {log.output_cost > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">输出费用</span>
              <span className="font-medium text-violet-300 tabular-nums">{fmtCost(log.output_cost)}</span>
            </div>
          )}
          <div className="border-t border-gray-700 pt-1.5">
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">标准费用</span>
              <span className="font-medium text-white tabular-nums">{fmtCost(log.total_cost)}</span>
            </div>
            <div className="flex justify-between gap-6 mt-1">
              <span className="text-gray-400">倍率</span>
              <span className="font-semibold text-blue-400 tabular-nums">{log.rate_multiplier}x</span>
            </div>
            <div className="flex justify-between gap-6 mt-1 border-t border-gray-700 pt-1.5">
              <span className="text-gray-400">实际扣费</span>
              <span className="font-bold text-green-400 tabular-nums">{fmtCost(log.actual_cost)}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

export default function AdminUsageLogs() {
  const user = useAuthStore(s => s.user)
  const isAdmin = user?.role === 'admin'

  // Filters
  const [filterModel, setFilterModel] = useState('')
  const [filterStatus, setFilterStatus] = useState('')
  const [dateRange, setDateRange] = useState<DateRangePreset>('7d')

  // Pagination
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(30)

  // Tooltips
  const [costTooltip, setCostTooltip] = useState<CostTooltipData | null>(null)
  const [tokenTooltip, setTokenTooltip] = useState<TokenTooltipData | null>(null)

  // Export state
  const [exporting, setExporting] = useState(false)

  // Build filter for the hook
  const { startDate, endDate } = useMemo(() => getDateRange(dateRange), [dateRange])

  const filter: AdminUsageLogsFilter = useMemo(() => ({
    page,
    pageSize,
    model: filterModel.trim() || undefined,
    status: filterStatus || undefined,
    startDate,
    endDate,
  }), [page, pageSize, filterModel, filterStatus, startDate, endDate])

  // Use the standardized hook
  const { logs, total, loading, refetch } = useAdminUsageLogs(filter)
  const invalidateUsageLogs = useInvalidateUsageLogs()

  const totalPages = Math.ceil(total / pageSize)

  const handleFilter = useCallback(() => {
    setPage(1)
  }, [])

  const handlePageChange = useCallback((p: number) => {
    setPage(p)
  }, [])

  // CSV Export
  const handleExport = useCallback(async () => {
    if (total === 0) return
    setExporting(true)
    try {
      const allLogs: AdminUsageLog[] = []
      const ps = 100
      const pages = Math.ceil(Math.min(total, 5000) / ps)
      for (let p = 1; p <= pages; p++) {
        const res = await fetchAdminUsageLogs({
          page: p,
          pageSize: ps,
          model: filterModel.trim() || undefined,
          status: filterStatus || undefined,
          startDate,
          endDate,
        })
        if (res?.items) allLogs.push(...res.items)
      }
      const header = '时间,用户,API Key,模型,Provider,类型,输入Tokens,输出Tokens,推理Tokens,缓存Tokens,标准费用,实际扣费,倍率,耗时(ms),状态\n'
      const rows = allLogs.map(l => [
        fmtDateTime(l.created_at), l.user_email || l.user_id, l.api_key_name || l.api_key_id,
        l.model, l.provider, l.stream ? 'Stream' : 'Sync',
        l.input_tokens, l.output_tokens, l.reasoning_tokens, l.cached_tokens,
        l.total_cost.toFixed(6), l.actual_cost.toFixed(6),
        l.rate_multiplier, l.duration_ms ?? '', l.failed ? '失败' : '成功',
      ].join(',')).join('\n')
      const blob = new Blob(['\ufeff' + header + rows], { type: 'text/csv;charset=utf-8;' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url; a.download = `admin_usage_${dateRange}.csv`; a.click()
      URL.revokeObjectURL(url)
      toast.success(`导出 ${allLogs.length} 条`)
    } catch { toast.error('导出失败') } finally { setExporting(false) }
  }, [total, filterModel, filterStatus, startDate, endDate, dateRange])

  if (!isAdmin) {
    return <div className="text-center py-20 text-gray-400">无权限访问此页面</div>
  }

  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500" style={{ willChange: 'transform, opacity' }}>
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">全局使用日志</h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1">查看所有用户的 API 调用记录。</p>
      </div>

      {/* Filter Bar */}
      <div className="card px-5 py-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-[140px]">
            <label className="text-[11px] font-semibold text-gray-500 uppercase tracking-wider mb-1 block">模型</label>
            <div className="relative">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-gray-400 pointer-events-none" />
              <input type="text" value={filterModel} onChange={e => setFilterModel(e.target.value)}
                placeholder="搜索模型..." className="input h-9 text-sm pl-8"
                onKeyDown={e => { if (e.key === 'Enter') handleFilter() }}
              />
              {filterModel && (
                <button onClick={() => setFilterModel('')} className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600">
                  <X className="w-3.5 h-3.5" />
                </button>
              )}
            </div>
          </div>
          <div>
            <label className="text-[11px] font-semibold text-gray-500 uppercase tracking-wider mb-1 block">状态</label>
            <select value={filterStatus} onChange={e => { setFilterStatus(e.target.value); setPage(1) }} className="input h-9 text-sm w-[100px]">
              <option value="">全部</option>
              <option value="success">成功</option>
              <option value="failed">失败</option>
            </select>
          </div>
          <div>
            <label className="text-[11px] font-semibold text-gray-500 uppercase tracking-wider mb-1 block">范围</label>
            <div className="flex gap-1">
              {([{ k: 'today' as const, l: '今天' }, { k: '7d' as const, l: '7天' }, { k: '30d' as const, l: '30天' }]).map(r => (
                <button key={r.k} onClick={() => { setDateRange(r.k); setPage(1) }}
                  className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-all ${
                    dateRange === r.k ? 'bg-primary-500 text-white shadow-sm' : 'bg-gray-100 dark:bg-dark-800 text-gray-600 dark:text-gray-400'
                  }`}
                >{r.l}</button>
              ))}
            </div>
          </div>
          <div className="ml-auto flex items-center gap-2">
            <button onClick={() => { invalidateUsageLogs(); refetch() }} disabled={loading} className="btn btn-secondary h-9 px-3 text-xs">
              <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} /> 刷新
            </button>
            <button onClick={handleExport} disabled={exporting || total === 0} className="btn btn-primary h-9 px-3 text-xs">
              {exporting ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
              CSV
            </button>
          </div>
        </div>
      </div>

      {/* Table */}
      <div className="glass-card overflow-hidden">
        <QueryStateWrapper
          isLoading={loading}
          isEmpty={!loading && logs.length === 0}
          onRetry={() => { invalidateUsageLogs(); refetch() }}
          emptyMessage="暂无使用记录"
        >
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th className="w-[140px]">用户</th>
                <th className="w-[90px]">API Key</th>
                <th>模型</th>
                <th className="w-[60px]">类型</th>
                <th className="w-[220px]">Tokens</th>
                <th className="w-[100px]">费用</th>
                <th className="w-[70px]">耗时</th>
                <th className="w-[120px]">时间</th>
                <th className="w-[40px]">状态</th>
              </tr>
            </thead>
            <tbody>
              {logs.map(log => (
                <tr key={log.id} className="group">
                  <td>
                    <span className="text-sm text-gray-900 dark:text-white truncate block max-w-[140px]" title={log.user_email}>
                      {log.user_email || `UID:${log.user_id}`}
                    </span>
                  </td>
                  <td>
                    <span className="text-sm text-gray-600 dark:text-gray-400 truncate block max-w-[90px]">{log.api_key_name || '-'}</span>
                  </td>
                  <td>
                    <div className="flex items-center gap-1.5 flex-wrap">
                      <span className="font-mono text-xs font-medium text-gray-900 dark:text-white">{log.model}</span>
                      {log.provider && (
                        <span className="text-[10px] font-medium text-gray-400 dark:text-dark-500 bg-gray-100 dark:bg-dark-800 px-1.5 py-0.5 rounded-md">
                          {log.provider}
                        </span>
                      )}
                    </div>
                  </td>
                  <td>
                    <span className={`inline-flex items-center rounded-md px-2 py-0.5 text-[11px] font-semibold ${
                      log.stream ? 'bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-400' : 'bg-gray-100 dark:bg-dark-800 text-gray-600 dark:text-gray-400'
                    }`}>{log.stream ? 'Stream' : 'Sync'}</span>
                  </td>
                  <td>
                    <div className="flex items-center gap-1">
                      {/* Input/Output tokens */}
                      <div className="flex items-center gap-1 text-sm">
                        <ArrowDownCircle className="w-3 h-3 text-emerald-500 flex-shrink-0" />
                        <span className="font-medium text-gray-900 dark:text-white tabular-nums">{fmtTokens(log.input_tokens)}</span>
                        <ArrowUpCircle className="w-3 h-3 text-violet-500 flex-shrink-0 ml-0.5" />
                        <span className="font-medium text-gray-900 dark:text-white tabular-nums">{fmtTokens(log.output_tokens)}</span>
                      </div>
                      {/* Cache & Reasoning badges */}
                      {log.cached_tokens > 0 && (
                        <span className="inline-flex items-center gap-0.5 rounded px-1 py-px text-[10px] font-semibold leading-tight bg-sky-100 text-sky-700 ring-1 ring-inset ring-sky-200 dark:bg-sky-500/20 dark:text-sky-300 dark:ring-sky-500/30 ml-0.5" title={`缓存命中 ${log.cached_tokens.toLocaleString()} tokens`}>
                          <Database className="w-2.5 h-2.5" />
                          {fmtTokens(log.cached_tokens)}
                        </span>
                      )}
                      {log.reasoning_tokens > 0 && (
                        <span className="inline-flex items-center gap-0.5 rounded px-1 py-px text-[10px] font-semibold leading-tight bg-amber-100 text-amber-700 ring-1 ring-inset ring-amber-200 dark:bg-amber-500/20 dark:text-amber-300 dark:ring-amber-500/30" title={`推理 ${log.reasoning_tokens.toLocaleString()} tokens`}>
                          <Brain className="w-2.5 h-2.5" />
                          {fmtTokens(log.reasoning_tokens)}
                        </span>
                      )}
                      {/* Token detail icon */}
                      <div
                        className="flex h-4 w-4 cursor-help items-center justify-center rounded-full bg-gray-100 dark:bg-dark-800 opacity-0 group-hover:opacity-100 transition flex-shrink-0"
                        onMouseEnter={e => setTokenTooltip({ log, x: e.clientX, y: e.clientY })}
                        onMouseLeave={() => setTokenTooltip(null)}
                      >
                        <Info className="w-2.5 h-2.5 text-gray-400" />
                      </div>
                    </div>
                  </td>
                  <td>
                    <div className="flex items-center gap-1.5">
                      <span className="font-medium text-green-600 dark:text-green-400 tabular-nums text-sm">{fmtCost(log.actual_cost)}</span>
                      <div
                        className="flex h-4 w-4 cursor-help items-center justify-center rounded-full bg-gray-100 dark:bg-dark-800 opacity-0 group-hover:opacity-100 transition"
                        onMouseEnter={e => setCostTooltip({ log, x: e.clientX, y: e.clientY })}
                        onMouseLeave={() => setCostTooltip(null)}
                      >
                        <Info className="w-2.5 h-2.5 text-gray-400" />
                      </div>
                    </div>
                  </td>
                  <td><span className="text-sm text-gray-500 tabular-nums">{fmtDuration(log.duration_ms)}</span></td>
                  <td><span className="text-sm text-gray-500 tabular-nums">{fmtDateTime(log.created_at)}</span></td>
                  <td className="text-center">
                    {log.failed ? <XCircle className="w-4 h-4 text-red-500 inline-block" /> : <CheckCircle2 className="w-4 h-4 text-green-500 inline-block" />}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        </QueryStateWrapper>

        {/* Pagination */}
        {total > 0 && (
          <div className="px-5 py-3 border-t border-border flex items-center justify-between bg-gray-50/50 dark:bg-dark-800/30">
            <div className="text-xs text-gray-500 tabular-nums">共 {total.toLocaleString()} 条 · 第 {page}/{totalPages} 页</div>
            <div className="flex items-center gap-1.5">
              <select value={pageSize} onChange={e => { setPageSize(Number(e.target.value)); setPage(1) }}
                className="h-8 rounded-lg border border-border bg-white dark:bg-dark-900 px-2 text-xs outline-none"
              >
                {[30, 50, 100].map(n => <option key={n} value={n}>{n} 条/页</option>)}
              </select>
              <button disabled={page <= 1} onClick={() => handlePageChange(page - 1)}
                className="h-8 w-8 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center disabled:opacity-30 transition-colors">
                <ChevronLeft className="w-4 h-4" />
              </button>
              <span className="px-2 text-xs text-gray-500 tabular-nums">{page}/{totalPages}</span>
              <button disabled={page >= totalPages} onClick={() => handlePageChange(page + 1)}
                className="h-8 w-8 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center disabled:opacity-30 transition-colors">
                <ChevronRight className="w-4 h-4" />
              </button>
            </div>
          </div>
        )}
      </div>

      {costTooltip && <CostTooltip data={costTooltip} />}
      {tokenTooltip && <TokenTooltip data={tokenTooltip} />}
    </div>
  )
}
