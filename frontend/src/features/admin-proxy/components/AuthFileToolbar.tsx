import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu'
import {
  RefreshCw, Plus, Trash2, Search,
  Gauge, ChevronDown, Pause, Play, Download
} from 'lucide-react'
import { providerBadgeColor } from '../authFileViewUtils'
import type { AuthFileToolbarProps, SmartView } from '../types'

interface SmartViewDef {
  id: SmartView
  label: string
  count: number
  color: string
}

export function AuthFileToolbar({
  smartView,
  onSmartViewChange,
  filterProvider,
  onFilterProviderChange,
  searchQuery,
  onSearchChange,
  providers,
  counts,
  selectedCount,
  selectedPausableCount,
  selectedResumableCount,
  batchStatusLoading,
  exportLoading,
  onBatchPause,
  onBatchResume,
  onBatchDelete,
  onBatchExport,
  onRefresh,
  refreshing,
  onUpload,
}: AuthFileToolbarProps) {
  const smartViews: SmartViewDef[] = [
    { id: 'all', label: '全部', count: counts.all, color: '' },
    { id: 'healthy', label: '路由中', count: counts.healthy, color: 'text-emerald-600' },
    { id: 'warning', label: '异常', count: counts.warning, color: 'text-amber-600' },
    { id: 'disabled', label: '已停用', count: counts.disabled, color: 'text-gray-500' },
  ]

  return (
    <section className="sticky top-2 z-20 rounded-2xl border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 px-4 py-3 shadow-sm">
      <div className="flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
        {/* Left: Smart Views + Filters */}
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2 sm:gap-3">
          {/* Smart View Pills */}
          <div className="flex flex-wrap items-center gap-1.5">
            {smartViews.map(v => (
              <button
                key={v.id}
                onClick={() => onSmartViewChange(v.id)}
                className={`inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-[11px] font-medium transition-colors ${
                  smartView === v.id
                    ? 'border-primary-300 bg-primary-50 text-primary-700 dark:border-primary-800 dark:bg-primary-950/30 dark:text-primary-300'
                    : 'border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 text-gray-500 dark:text-gray-400 hover:border-gray-300 dark:hover:border-dark-500 hover:text-gray-700 dark:hover:text-gray-200'
                }`}
              >
                {v.label}
                <span className="rounded bg-black/5 dark:bg-white/10 px-1 py-0.5 text-[10px] tabular-nums">{v.count}</span>
              </button>
            ))}
          </div>

          {/* Divider */}
          <div className="hidden h-4 w-px rounded-full bg-gray-200 dark:bg-dark-600 sm:block" />

          {/* Provider Filter */}
          <DropdownMenuPrimitive.Root>
            <DropdownMenuPrimitive.Trigger asChild>
              <button className="inline-flex items-center gap-1.5 rounded-md border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 px-2 py-1 text-[11px] font-medium text-gray-500 dark:text-gray-400 hover:border-gray-300 dark:hover:border-dark-500 transition-colors">
                <Gauge className="h-3 w-3" />
                {filterProvider === 'all' ? '全部渠道' : filterProvider}
                <ChevronDown className="h-2.5 w-2.5" />
              </button>
            </DropdownMenuPrimitive.Trigger>
            <DropdownMenuPrimitive.Portal>
              <DropdownMenuPrimitive.Content align="start" sideOffset={4} className="z-50 w-44 rounded-xl border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-800 p-1 shadow-lg animate-in fade-in-0 zoom-in-95">
                <DropdownMenuPrimitive.Item className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm cursor-pointer outline-none hover:bg-gray-100 dark:hover:bg-dark-700 text-gray-700 dark:text-gray-200" onSelect={() => onFilterProviderChange('all')}>
                  全部渠道
                </DropdownMenuPrimitive.Item>
                {providers.map(p => (
                  <DropdownMenuPrimitive.Item key={p} className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm cursor-pointer outline-none hover:bg-gray-100 dark:hover:bg-dark-700 text-gray-700 dark:text-gray-200" onSelect={() => onFilterProviderChange(p)}>
                    <span className={`h-2 w-2 rounded-full ${providerBadgeColor(p).split(' ')[0]}`} />
                    {p}
                  </DropdownMenuPrimitive.Item>
                ))}
              </DropdownMenuPrimitive.Content>
            </DropdownMenuPrimitive.Portal>
          </DropdownMenuPrimitive.Root>
        </div>

        {/* Right: Actions */}
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
          {selectedCount > 0 && (
            <div className="flex shrink-0 items-center gap-1.5 rounded-xl border border-primary-200 bg-primary-50/80 px-2 py-1 dark:border-primary-900/60 dark:bg-primary-950/30">
              <span className="px-1 text-[11px] font-medium text-primary-700 dark:text-primary-300">已选 {selectedCount} 项</span>
              <button
                onClick={onBatchPause}
                disabled={batchStatusLoading || selectedPausableCount === 0}
                className="inline-flex h-7 items-center gap-1 rounded-lg border border-amber-200 bg-white px-2 text-[11px] font-medium text-amber-700 shadow-sm transition-colors hover:bg-amber-50 disabled:cursor-not-allowed disabled:opacity-45 dark:border-amber-900/60 dark:bg-dark-900 dark:text-amber-300 dark:hover:bg-amber-950/30"
              >
                <Pause className="h-3 w-3" />
                暂停调度 {selectedPausableCount > 0 ? selectedPausableCount : ''}
              </button>
              <button
                onClick={onBatchResume}
                disabled={batchStatusLoading || selectedResumableCount === 0}
                className="inline-flex h-7 items-center gap-1 rounded-lg border border-emerald-200 bg-white px-2 text-[11px] font-medium text-emerald-700 shadow-sm transition-colors hover:bg-emerald-50 disabled:cursor-not-allowed disabled:opacity-45 dark:border-emerald-900/60 dark:bg-dark-900 dark:text-emerald-300 dark:hover:bg-emerald-950/30"
              >
                <Play className="h-3 w-3" />
                恢复调度 {selectedResumableCount > 0 ? selectedResumableCount : ''}
              </button>
              <button
                onClick={onBatchExport}
                disabled={exportLoading}
                className="inline-flex h-7 items-center gap-1 rounded-lg border border-indigo-200 bg-white px-2 text-[11px] font-medium text-indigo-700 shadow-sm transition-colors hover:bg-indigo-50 disabled:cursor-not-allowed disabled:opacity-45 dark:border-indigo-900/60 dark:bg-dark-900 dark:text-indigo-300 dark:hover:bg-indigo-950/30"
              >
                <Download className={`h-3 w-3 ${exportLoading ? 'animate-pulse' : ''}`} />
                {exportLoading ? '导出中…' : '导出 zip'}
              </button>
              <button
                onClick={onBatchDelete}
                className="inline-flex h-7 items-center gap-1 rounded-lg bg-red-600 px-2 text-[11px] font-medium text-white shadow-sm transition-colors hover:bg-red-700"
              >
                <Trash2 className="h-3 w-3" />
                删除
              </button>
            </div>
          )}
          <div className="relative hidden shrink-0 sm:block">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-gray-400" />
            <input
              className="h-8 w-[260px] shrink-0 rounded-lg border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 pl-7 pr-3 text-xs text-gray-700 dark:text-gray-200 placeholder:text-gray-400 outline-none focus:border-primary-400 focus:ring-1 focus:ring-primary-400/30 transition-colors"
              placeholder="搜索账号、凭证名或渠道..."
              value={searchQuery}
              onChange={e => onSearchChange(e.target.value)}
            />
          </div>
          <button
            onClick={onRefresh}
            disabled={refreshing}
            className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 px-3 text-xs font-medium text-gray-600 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-dark-800 transition-colors disabled:opacity-50"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${refreshing ? 'animate-spin' : ''}`} />
            刷新列表
          </button>
          <button
            onClick={onUpload}
            className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg bg-primary-600 hover:bg-primary-700 px-3 text-xs font-medium text-white shadow-sm transition-colors"
          >
            <Plus className="h-3.5 w-3.5" />
            导入凭证
          </button>
        </div>
      </div>
    </section>
  )
}
