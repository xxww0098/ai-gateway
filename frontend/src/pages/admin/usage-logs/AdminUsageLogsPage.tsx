import { useState, useCallback, useMemo } from 'react'
import { useAuthStore } from '@/features/auth/auth_store'
import { useAdminUsageLogs, useInvalidateUsageLogs } from '@/features/usage/hooks'
import { fetchAdminUsageLogs } from '@/features/usage/api'
import type { AdminUsageLog, AdminUsageLogsFilter, DateRangePreset } from '@/features/usage/types'
import { QueryStateWrapper } from '@/shared/components/QueryStateWrapper'
import { toast } from 'sonner'
import {
  Search, RefreshCw, Download,
  ChevronLeft, ChevronRight, X, Check, Copy,
} from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/shared/components/ui/dialog'

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

function StatusPill({ failed }: { failed: boolean }) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-semibold ${
        failed
          ? 'border-red-200 bg-red-50 text-red-700 dark:border-red-900/60 dark:bg-red-950/30 dark:text-red-300'
          : 'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900/60 dark:bg-emerald-950/30 dark:text-emerald-300'
      }`}
    >
      <span className={`h-1.5 w-1.5 rounded-full ${failed ? 'bg-red-500' : 'bg-emerald-500'}`} />
      {failed ? '失败' : '成功'}
    </span>
  )
}

function AdminLogDetail({ log }: { log: AdminUsageLog }) {
  const [copied, setCopied] = useState(false)

  const copyRequestId = async () => {
    await navigator.clipboard.writeText(log.request_id)
    setCopied(true)
    window.setTimeout(() => {
      setCopied(false)
    }, 1500)
  }

  return (
    <div className="space-y-4 text-sm">
      <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-xs">
        <span className="text-muted-foreground">用户</span>
        <span className="truncate text-right" title={log.user_email}>{log.user_email || `UID:${log.user_id}`}</span>
        <span className="text-muted-foreground">API Key</span>
        <span className="truncate text-right">{log.api_key_name || `#${log.api_key_id}`}</span>
        <span className="text-muted-foreground">模型</span>
        <span className="break-all text-right font-mono">{log.model}</span>
        <span className="text-muted-foreground">Provider</span>
        <span className="text-right font-mono">{log.provider || '-'}</span>
        <span className="text-muted-foreground">耗时</span>
        <span className="text-right tabular-nums">{fmtDuration(log.duration_ms)} · {log.stream ? 'Stream' : 'Sync'}</span>
        <span className="text-muted-foreground">时间</span>
        <span className="text-right tabular-nums">{fmtDateTime(log.created_at)}</span>
        <span className="text-muted-foreground">状态</span>
        <span className="text-right"><StatusPill failed={log.failed} /></span>
        <span className="self-center text-muted-foreground">请求 ID</span>
        <span className="flex min-w-0 items-center justify-end gap-2">
          <code className="truncate font-mono text-[11px]" title={log.request_id}>{log.request_id}</code>
          <button
            type="button"
            onClick={() => void copyRequestId()}
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-border text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={copied ? '请求 ID 已复制' : '复制请求 ID'}
          >
            {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
          </button>
        </span>
      </div>

      <div className="rounded-lg border border-border p-3">
        <div className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Tokens</div>
        {[
          ['输入', log.input_tokens],
          ['输出', log.output_tokens],
          ...(log.cached_tokens > 0 ? [['缓存', log.cached_tokens] as const] : []),
          ...(log.reasoning_tokens > 0 ? [['推理', log.reasoning_tokens] as const] : []),
        ].map(([label, value]) => (
          <div key={label} className="flex justify-between gap-4 py-0.5">
            <span className="text-muted-foreground">{label}</span>
            <span className="tabular-nums">{value.toLocaleString()}</span>
          </div>
        ))}
      </div>

      <div className="rounded-lg border border-border p-3">
        <div className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">费用</div>
        {log.input_cost > 0 && (
          <div className="flex justify-between gap-4 py-0.5">
            <span className="text-muted-foreground">输入</span>
            <span className="tabular-nums">{fmtCost(log.input_cost)}</span>
          </div>
        )}
        {log.output_cost > 0 && (
          <div className="flex justify-between gap-4 py-0.5">
            <span className="text-muted-foreground">输出</span>
            <span className="tabular-nums">{fmtCost(log.output_cost)}</span>
          </div>
        )}
        <div className="flex justify-between gap-4 py-0.5">
          <span className="text-muted-foreground">倍率</span>
          <span className="tabular-nums">{log.rate_multiplier}x</span>
        </div>
        <div className="flex justify-between gap-4 py-0.5">
          <span className="text-muted-foreground">标准费用</span>
          <span className="tabular-nums">{fmtCost(log.total_cost)}</span>
        </div>
        <div className="mt-1 flex justify-between gap-4 border-t border-border pt-2">
          <span className="font-medium">实际扣费</span>
          <span className="font-bold tabular-nums">{fmtCost(log.actual_cost)}</span>
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

  const [detail, setDetail] = useState<AdminUsageLog | null>(null)

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
        allLogs.push(...res.items)
      }
      const header = '时间,用户,API Key,模型,Provider,类型,输入Tokens,输出Tokens,推理Tokens,缓存Tokens,标准费用,实际扣费,倍率,耗时(ms),状态\n'
      const rows = allLogs.map(l => [
        fmtDateTime(l.created_at), l.user_email || l.user_id, l.api_key_name || l.api_key_id,
        l.model, l.provider, l.stream ? 'Stream' : 'Sync',
        l.input_tokens, l.output_tokens, l.reasoning_tokens, l.cached_tokens,
        l.total_cost.toFixed(6), l.actual_cost.toFixed(6),
        l.rate_multiplier, l.duration_ms, l.failed ? '失败' : '成功',
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
              <input type="text" value={filterModel} onChange={e => { setFilterModel(e.target.value) }}
                placeholder="搜索模型..." className="input h-9 text-sm pl-8"
                onKeyDown={e => { if (e.key === 'Enter') handleFilter() }}
              />
              {filterModel && (
                <button onClick={() => { setFilterModel('') }} className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600">
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
            <button onClick={() => { invalidateUsageLogs(); void refetch() }} disabled={loading} className="btn btn-secondary h-9 px-3 text-xs">
              <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} /> 刷新
            </button>
            <button onClick={() => { void handleExport() }} disabled={exporting || total === 0} className="btn btn-primary h-9 px-3 text-xs">
              {exporting ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
              {exporting ? '导出中...' : '导出 CSV'}
            </button>
          </div>
        </div>
      </div>

      {/* Table */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <QueryStateWrapper
          isLoading={loading}
          isEmpty={!loading && logs.length === 0}
          onRetry={() => { invalidateUsageLogs(); void refetch() }}
          empty={{
            tone: 'no-results',
            title: '当前筛选条件下暂无调用记录',
            description: '请尝试调整时间范围或清除筛选条件。全站 API 调用的 Token 消耗与计费数据将在此实时记录。',
          }}
        >
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th className="w-[120px]">时间</th>
                <th className="w-[140px]">用户</th>
                <th className="w-[90px]">API Key</th>
                <th>模型</th>
                <th className="w-[180px]">Tokens</th>
                <th className="w-[100px] text-right">实际扣费</th>
                <th className="w-[100px]">耗时</th>
                <th className="w-[84px]">状态</th>
              </tr>
            </thead>
            <tbody>
              {logs.map(log => (
                <tr
                  key={log.id}
                  className="cursor-pointer hover:bg-muted/50"
                  onClick={() => {
                    setDetail(log)
                  }}
                >
                  <td><span className="whitespace-nowrap text-sm text-muted-foreground tabular-nums">{fmtDateTime(log.created_at)}</span></td>
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
                    <div className="space-y-0.5">
                      <div className="flex items-center gap-2 text-sm tabular-nums">
                        <span className="font-medium">↓ {fmtTokens(log.input_tokens)}</span>
                        <span className="font-medium">↑ {fmtTokens(log.output_tokens)}</span>
                      </div>
                      {(log.cached_tokens > 0 || log.reasoning_tokens > 0) && (
                        <div className="flex gap-2 text-[11px] text-muted-foreground">
                          {log.cached_tokens > 0 && <span>缓存 {fmtTokens(log.cached_tokens)}</span>}
                          {log.reasoning_tokens > 0 && <span>推理 {fmtTokens(log.reasoning_tokens)}</span>}
                        </div>
                      )}
                    </div>
                  </td>
                  <td className="text-right">
                    <span className="font-semibold tabular-nums text-sm">{fmtCost(log.actual_cost)}</span>
                  </td>
                  <td>
                    <div className="text-sm tabular-nums">{fmtDuration(log.duration_ms)}</div>
                    <div className="text-[11px] text-muted-foreground">{log.stream ? 'Stream' : 'Sync'}</div>
                  </td>
                  <td><StatusPill failed={log.failed} /></td>
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
              <button disabled={page <= 1} onClick={() => { handlePageChange(page - 1) }}
                className="h-8 w-8 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center disabled:opacity-30 transition-colors">
                <ChevronLeft className="w-4 h-4" />
              </button>
              <span className="px-2 text-xs text-gray-500 tabular-nums">{page}/{totalPages}</span>
              <button disabled={page >= totalPages} onClick={() => { handlePageChange(page + 1) }}
                className="h-8 w-8 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center disabled:opacity-30 transition-colors">
                <ChevronRight className="w-4 h-4" />
              </button>
            </div>
          </div>
        )}
      </div>

      <Dialog
        open={!!detail}
        onOpenChange={(open) => {
          if (!open) setDetail(null)
        }}
      >
        <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-md">
          <DialogHeader>
            <DialogTitle>请求详情</DialogTitle>
          </DialogHeader>
          {detail && <AdminLogDetail log={detail} />}
        </DialogContent>
      </Dialog>
    </div>
  )
}
