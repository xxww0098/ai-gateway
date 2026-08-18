import { memo, useState } from 'react'
import {
  FileText, RefreshCw, ChevronLeft, ChevronRight,
  Check, Copy,
} from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/shared/components/ui/dialog'
import { EmptyState, EmptyStateRow } from '@/shared/components/EmptyState'
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

function LogDetailBody({ log }: { log: UsageLog }) {
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
        <div className="text-right"><StatusPill failed={log.failed} /></div>
        <div className="self-center text-muted-foreground">请求 ID</div>
        <div className="flex min-w-0 items-center justify-end gap-2">
          <code className="truncate font-mono text-[11px]" title={log.request_id}>{log.request_id}</code>
          <button
            type="button"
            onClick={() => void copyRequestId()}
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-border text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={copied ? '请求 ID 已复制' : '复制请求 ID'}
          >
            {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
          </button>
        </div>
      </div>

      <div className="rounded-lg border border-border p-3 space-y-2">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Tokens</div>
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground">输入</span>
          <span className="tabular-nums font-medium">{log.input_tokens.toLocaleString()}</span>
        </div>
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground">输出</span>
          <span className="tabular-nums font-medium">{log.output_tokens.toLocaleString()}</span>
        </div>
        {log.cached_tokens > 0 && (
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">缓存</span>
            <span className="tabular-nums">{fmtTokens(log.cached_tokens)}</span>
          </div>
        )}
        {log.reasoning_tokens > 0 && (
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">推理</span>
            <span className="tabular-nums">{fmtTokens(log.reasoning_tokens)}</span>
          </div>
        )}
      </div>

      <div className="rounded-lg border border-border p-3 space-y-2">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">费用</div>
        {log.input_cost > 0 && (
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">输入</span>
            <span className="tabular-nums">{fmtCost(log.input_cost)}</span>
          </div>
        )}
        {log.output_cost > 0 && (
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">输出</span>
            <span className="tabular-nums">{fmtCost(log.output_cost)}</span>
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
          <span className="tabular-nums font-bold">{fmtCost(log.actual_cost)}</span>
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
          onClick={() => {
            onPageChange(page - 1)
          }}
          className="h-9 w-9 rounded-lg border border-border bg-white dark:bg-dark-900 flex items-center justify-center disabled:opacity-30"
          aria-label="上一页"
        >
          <ChevronLeft className="w-4 h-4" />
        </button>
        <button
          type="button"
          disabled={page >= totalPages}
          onClick={() => {
            onPageChange(page + 1)
          }}
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
}: UsageTableProps) {
  const [detail, setDetail] = useState<UsageLog | null>(null)

  const openDetail = (log: UsageLog) => {
    setDetail(log)
  }

  return (
    <>
      {/* Mobile cards */}
      <div className="md:hidden space-y-3">
        {loading ? (
          <div className="rounded-xl border border-border bg-card flex h-32 items-center justify-center gap-2 text-muted-foreground text-sm">
            <RefreshCw className="w-4 h-4 animate-spin text-primary" />
            加载中...
          </div>
        ) : logs.length === 0 ? (
          <EmptyState
            bordered
            size="compact"
            tone="no-results"
            icon={FileText}
            title="当前筛选条件下暂无调用记录"
            description="请尝试调整时间范围或清除筛选条件。每次成功请求都会在此记录 Token 消耗与扣费明细。"
          />
        ) : (
          logs.map((log) => (
            <button
              key={log.id}
              type="button"
              onClick={() => {
                openDetail(log)
              }}
              className="w-full space-y-2 rounded-xl border border-border bg-card p-3 text-left hover:bg-muted/40 active:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="font-mono text-xs font-medium truncate">{log.model}</div>
                  <div className="text-[11px] text-muted-foreground mt-0.5">
                    {fmtDuration(log.duration_ms)} · {log.stream ? 'Stream' : 'Sync'}
                  </div>
                </div>
                <StatusPill failed={log.failed} />
              </div>
              <div className="flex items-center justify-between text-sm">
                <span className="tabular-nums text-muted-foreground">
                  ↓{fmtTokens(log.input_tokens)} ↑{fmtTokens(log.output_tokens)}
                </span>
                <span className="font-semibold tabular-nums">
                  {fmtCost(log.actual_cost)}
                </span>
              </div>
              <div className="text-[11px] text-muted-foreground tabular-nums">{fmtDateTime(log.created_at)}</div>
            </button>
          ))
        )}
        <div className="rounded-xl border border-border bg-card overflow-hidden">
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
      <div className="rounded-xl border border-border bg-card overflow-hidden hidden md:block">
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th className="w-[160px]">时间</th>
                <th className="w-[120px]">API Key</th>
                <th>模型</th>
                <th className="w-[180px]">Tokens</th>
                <th className="w-[120px] text-right">实际扣费</th>
                <th className="w-[100px]">耗时</th>
                <th className="w-[84px]">状态</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={7} className="h-40 text-center">
                    <div className="flex items-center justify-center gap-2 text-gray-400">
                      <RefreshCw className="w-4 h-4 animate-spin text-primary-500" />
                      加载中...
                    </div>
                  </td>
                </tr>
              ) : logs.length === 0 ? (
                <EmptyStateRow
                  colSpan={7}
                  tone="no-results"
                  icon={FileText}
                  title="当前筛选条件下暂无调用记录"
                  description="请尝试调整时间范围或清除筛选条件。每次成功请求都会在此记录 Token 消耗与扣费明细。"
                />
              ) : (
                logs.map((log) => (
                  <tr
                    key={log.id}
                    className="cursor-pointer hover:bg-gray-50 dark:hover:bg-dark-800/40"
                    onClick={() => {
                      openDetail(log)
                    }}
                  >
                    <td>
                      <span className="text-sm text-muted-foreground tabular-nums whitespace-nowrap">
                        {fmtDateTime(log.created_at)}
                      </span>
                    </td>
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
                      <div className="space-y-0.5">
                        <div className="flex items-center gap-2 text-sm tabular-nums">
                          <span className="font-medium">↓ {log.input_tokens.toLocaleString()}</span>
                          <span className="font-medium">↑ {log.output_tokens.toLocaleString()}</span>
                        </div>
                          {(log.cached_tokens > 0 || log.reasoning_tokens > 0) && (
                            <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                              {log.cached_tokens > 0 && (
                                <span className="tabular-nums">缓存 {fmtTokens(log.cached_tokens)}</span>
                              )}
                              {log.reasoning_tokens > 0 && (
                                <span className="tabular-nums">推理 {fmtTokens(log.reasoning_tokens)}</span>
                              )}
                            </div>
                          )}
                      </div>
                    </td>
                    <td className="text-right">
                      <span className="font-semibold tabular-nums text-sm">
                        {fmtCost(log.actual_cost)}
                      </span>
                    </td>
                    <td>
                      <div className="text-sm tabular-nums">{fmtDuration(log.duration_ms)}</div>
                      <div className="text-[11px] text-muted-foreground">{log.stream ? 'Stream' : 'Sync'}</div>
                    </td>
                    <td>
                      <StatusPill failed={log.failed} />
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

      <Dialog
        open={!!detail}
        onOpenChange={(open) => {
          if (!open) setDetail(null)
        }}
      >
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
