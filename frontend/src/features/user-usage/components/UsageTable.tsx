import { memo, useState } from 'react'
import {
  FileText, RefreshCw, ChevronLeft, ChevronRight,
  ArrowDownCircle, ArrowUpCircle, CheckCircle2, XCircle, Info,
} from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/shared/components/ui/dialog'
import type { UsageLog, UsageTableProps } from '../types'

function fmtDuration(ms: number | null): string {
  if (ms == null) return '-'
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

function fmtCost(n: number): string {
  if (n === 0) return '$0.00'
  if (n < 0.0001) return `$${n.toFixed(6)}`
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(4)}`
}

function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

function LogDetailBody({ log }: { log: UsageLog }) {
  return (
    <div className="space-y-4 text-sm">
      <div className="grid grid-cols-2 gap-2 text-xs">
        <div className="text-muted-foreground">模型</div>
        <div className="font-mono text-right break-all">{log.model}</div>
        <div className="text-muted-foreground">Key</div>
        <div className="text-right truncate">{log.api_key_name || '-'}</div>
        <div className="text-muted-foreground">类型</div>
        <div className="text-right">{log.stream ? 'Stream' : 'Sync'}</div>
        <div className="text-muted-foreground">耗时</div>
        <div className="text-right tabular-nums">{fmtDuration(log.duration_ms)}</div>
        <div className="text-muted-foreground">时间</div>
        <div className="text-right tabular-nums">{fmtDateTime(log.created_at)}</div>
        <div className="text-muted-foreground">状态</div>
        <div className="text-right">{log.failed ? '失败' : '成功'}</div>
      </div>

      <div className="rounded-lg border border-border p-3 space-y-2">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Tokens</div>
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground inline-flex items-center gap-1">
            <ArrowDownCircle className="w-3.5 h-3.5 text-emerald-500" /> 输入
          </span>
          <span className="tabular-nums font-medium">{log.input_tokens.toLocaleString()}</span>
        </div>
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground inline-flex items-center gap-1">
            <ArrowUpCircle className="w-3.5 h-3.5 text-violet-500" /> 输出
          </span>
          <span className="tabular-nums font-medium">{log.output_tokens.toLocaleString()}</span>
        </div>
        {log.cached_tokens > 0 && (
          <div className="flex justify-between gap-4 text-sky-600 dark:text-sky-400">
            <span>缓存</span>
            <span className="tabular-nums">{fmtTokens(log.cached_tokens)}</span>
          </div>
        )}
        {log.reasoning_tokens > 0 && (
          <div className="flex justify-between gap-4 text-amber-600 dark:text-amber-400">
            <span>推理</span>
            <span className="tabular-nums">{fmtTokens(log.reasoning_tokens)}</span>
          </div>
        )}
      </div>

      <div className="rounded-lg border border-border p-3 space-y-2">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">费用</div>
        {log.input_cost > 0 && (
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">输入</span>
            <span className="tabular-nums text-emerald-600">{fmtCost(log.input_cost)}</span>
          </div>
        )}
        {log.output_cost > 0 && (
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">输出</span>
            <span className="tabular-nums text-violet-600">{fmtCost(log.output_cost)}</span>
          </div>
        )}
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground">倍率</span>
          <span className="tabular-nums font-medium">{log.rate_multiplier}x</span>
        </div>
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground">标准费用</span>
          <span className="tabular-nums">{fmtCost(log.total_cost)}</span>
        </div>
        <div className="flex justify-between gap-4 border-t border-border pt-2">
          <span className="font-medium">实际扣费</span>
          <span className="tabular-nums font-bold text-green-600 dark:text-green-400">
            {fmtCost(log.actual_cost)}
          </span>
        </div>
      </div>
    </div>
  )
}

function PaginationBar({
  total,
  page,
  pageSize,
  totalPages,
  onPageChange,
  onPageSizeChange,
}: {
  total: number
  page: number
  pageSize: number
  totalPages: number
  onPageChange: (p: number) => void
  onPageSizeChange: (s: number) => void
}) {
  if (total <= 0) return null
  return (
    <div className="px-3 sm:px-5 py-3 border-t border-border flex flex-wrap items-center justify-between gap-2 bg-gray-50/50 dark:bg-dark-800/30">
      <div className="text-xs text-gray-500 dark:text-dark-400 tabular-nums">
        共 {total.toLocaleString()} 条 · {page}/{totalPages}
      </div>
      <div className="flex items-center gap-1.5">
        <select
          value={pageSize}
          onChange={(e) => {
            onPageSizeChange(Number(e.target.value))
            onPageChange(1)
          }}
          className="hidden sm:block h-9 rounded-lg border border-border bg-white dark:bg-dark-900 px-2 text-xs outline-none"
        >
          {[20, 50, 100].map((n) => (
            <option key={n} value={n}>
              {n} 条/页
            </option>
          ))}
        </select>
        <button
          type="button"
          disabled={page <= 1}
          onClick={() => onPageChange(page - 1)}
          className="h-9 w-9 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center disabled:opacity-30"
          aria-label="上一页"
        >
          <ChevronLeft className="w-4 h-4" />
        </button>
        <button
          type="button"
          disabled={page >= totalPages}
          onClick={() => onPageChange(page + 1)}
          className="h-9 w-9 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center disabled:opacity-30"
          aria-label="下一页"
        >
          <ChevronRight className="w-4 h-4" />
        </button>
      </div>
    </div>
  )
}

export const UsageTable = memo(function UsageTable({
  logs,
  loading,
  total,
  page,
  pageSize,
  totalPages,
  onPageChange,
  onPageSizeChange,
  onCostTooltip,
  onTokenTooltip,
}: UsageTableProps) {
  const [detail, setDetail] = useState<UsageLog | null>(null)

  const openDetail = (log: UsageLog) => {
    setDetail(log)
    // Clear floating tooltips if parent still mounts them
    onCostTooltip(null)
    onTokenTooltip(null)
  }

  return (
    <>
      {/* Mobile cards */}
      <div className="md:hidden space-y-3">
        {loading ? (
          <div className="glass-card flex h-32 items-center justify-center gap-2 text-gray-400 text-sm">
            <RefreshCw className="w-4 h-4 animate-spin text-primary-500" />
            加载中...
          </div>
        ) : logs.length === 0 ? (
          <div className="glass-card flex h-32 flex-col items-center justify-center gap-2 text-gray-400 text-sm">
            <FileText className="w-10 h-10 opacity-30" />
            所选范围内暂无使用记录
          </div>
        ) : (
          logs.map((log) => (
            <button
              key={log.id}
              type="button"
              onClick={() => openDetail(log)}
              className="w-full text-left rounded-xl border border-border bg-white dark:bg-dark-900 p-3 shadow-sm active:bg-gray-50 dark:active:bg-dark-800 space-y-2"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="font-mono text-xs font-medium truncate">{log.model}</div>
                  <div className="text-[11px] text-muted-foreground mt-0.5">
                    {log.api_key_name || '-'} · {log.stream ? 'Stream' : 'Sync'}
                  </div>
                </div>
                {log.failed ? (
                  <XCircle className="w-4 h-4 text-red-500 shrink-0" />
                ) : (
                  <CheckCircle2 className="w-4 h-4 text-green-500 shrink-0" />
                )}
              </div>
              <div className="flex items-center justify-between text-sm">
                <span className="tabular-nums text-muted-foreground">
                  ↓{fmtTokens(log.input_tokens)} ↑{fmtTokens(log.output_tokens)}
                </span>
                <span className="font-semibold tabular-nums text-green-600 dark:text-green-400">
                  {fmtCost(log.actual_cost)}
                </span>
              </div>
              <div className="text-[11px] text-muted-foreground tabular-nums">
                {fmtDateTime(log.created_at)} · {fmtDuration(log.duration_ms)}
              </div>
            </button>
          ))
        )}
        <div className="glass-card overflow-hidden">
          <PaginationBar
            total={total}
            page={page}
            pageSize={pageSize}
            totalPages={totalPages}
            onPageChange={onPageChange}
            onPageSizeChange={onPageSizeChange}
          />
        </div>
      </div>

      {/* Desktop table */}
      <div className="glass-card overflow-hidden hidden md:block">
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th className="w-[120px]">API Key</th>
                <th>模型</th>
                <th className="w-[60px]">类型</th>
                <th className="w-[180px]">Tokens</th>
                <th className="w-[120px]">费用</th>
                <th className="w-[80px]">耗时</th>
                <th className="w-[160px]">时间</th>
                <th className="w-[50px]">状态</th>
                <th className="w-[44px]"></th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={9} className="h-40 text-center">
                    <div className="flex items-center justify-center gap-2 text-gray-400">
                      <RefreshCw className="w-4 h-4 animate-spin text-primary-500" />
                      加载中...
                    </div>
                  </td>
                </tr>
              ) : logs.length === 0 ? (
                <tr>
                  <td colSpan={9} className="h-40 text-center text-gray-400 dark:text-dark-500">
                    <div className="flex flex-col items-center gap-2">
                      <FileText className="w-10 h-10 opacity-30" />
                      <span>所选范围内暂无使用记录</span>
                    </div>
                  </td>
                </tr>
              ) : (
                logs.map((log) => (
                  <tr
                    key={log.id}
                    className="cursor-pointer hover:bg-gray-50 dark:hover:bg-dark-800/40"
                    onClick={() => openDetail(log)}
                  >
                    <td>
                      <span
                        className="text-sm font-medium text-gray-900 dark:text-white truncate block max-w-[120px]"
                        title={log.api_key_name}
                      >
                        {log.api_key_name || '-'}
                      </span>
                    </td>
                    <td>
                      <span className="font-mono text-xs font-medium text-gray-900 dark:text-white" title={log.model}>
                        {log.model}
                      </span>
                      {log.provider && (
                        <span className="ml-1.5 inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium bg-gray-100 dark:bg-dark-800 text-gray-500 dark:text-dark-400">
                          {log.provider}
                        </span>
                      )}
                    </td>
                    <td>
                      <span
                        className={`inline-flex items-center rounded-md px-2 py-0.5 text-[11px] font-semibold ${
                          log.stream
                            ? 'bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-400'
                            : 'bg-gray-100 dark:bg-dark-800 text-gray-600 dark:text-gray-400'
                        }`}
                      >
                        {log.stream ? 'Stream' : 'Sync'}
                      </span>
                    </td>
                    <td>
                      <div className="flex items-center gap-1.5">
                        <div className="space-y-0.5">
                          <div className="flex items-center gap-2 text-sm">
                            <div className="inline-flex items-center gap-0.5">
                              <ArrowDownCircle className="w-3 h-3 text-emerald-500" />
                              <span className="font-medium tabular-nums">{log.input_tokens.toLocaleString()}</span>
                            </div>
                            <div className="inline-flex items-center gap-0.5">
                              <ArrowUpCircle className="w-3 h-3 text-violet-500" />
                              <span className="font-medium tabular-nums">{log.output_tokens.toLocaleString()}</span>
                            </div>
                          </div>
                          {(log.cached_tokens > 0 || log.reasoning_tokens > 0) && (
                            <div className="flex items-center gap-2 text-[11px]">
                              {log.cached_tokens > 0 && (
                                <span className="text-sky-500 tabular-nums">缓存 {fmtTokens(log.cached_tokens)}</span>
                              )}
                              {log.reasoning_tokens > 0 && (
                                <span className="text-amber-500 tabular-nums">
                                  推理 {fmtTokens(log.reasoning_tokens)}
                                </span>
                              )}
                            </div>
                          )}
                        </div>
                      </div>
                    </td>
                    <td>
                      <span className="font-medium text-green-600 dark:text-green-400 tabular-nums text-sm">
                        {fmtCost(log.actual_cost)}
                      </span>
                    </td>
                    <td>
                      <span className="text-sm text-gray-500 dark:text-gray-400 tabular-nums">
                        {fmtDuration(log.duration_ms)}
                      </span>
                    </td>
                    <td>
                      <span className="text-sm text-gray-500 dark:text-gray-400 tabular-nums">
                        {fmtDateTime(log.created_at)}
                      </span>
                    </td>
                    <td className="text-center">
                      {log.failed ? (
                        <XCircle className="w-4 h-4 text-red-500 inline-block" />
                      ) : (
                        <CheckCircle2 className="w-4 h-4 text-green-500 inline-block" />
                      )}
                    </td>
                    <td>
                      <button
                        type="button"
                        className="flex h-8 w-8 items-center justify-center rounded-full bg-gray-100 dark:bg-dark-800 hover:bg-primary-50 dark:hover:bg-primary-900/30"
                        onClick={(e) => {
                          e.stopPropagation()
                          openDetail(log)
                        }}
                        aria-label="查看明细"
                      >
                        <Info className="w-3.5 h-3.5 text-gray-500" />
                      </button>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
        <PaginationBar
          total={total}
          page={page}
          pageSize={pageSize}
          totalPages={totalPages}
          onPageChange={onPageChange}
          onPageSizeChange={onPageSizeChange}
        />
      </div>

      <Dialog open={!!detail} onOpenChange={(o) => !o && setDetail(null)}>
        <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-md">
          <DialogHeader>
            <DialogTitle>用量明细</DialogTitle>
          </DialogHeader>
          {detail && <LogDetailBody log={detail} />}
        </DialogContent>
      </Dialog>
    </>
  )
})
