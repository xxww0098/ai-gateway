import { memo } from 'react'
import { Search, RefreshCw, Download, X } from 'lucide-react'
import type { UsageFilterBarProps } from '../types'

const RANGES = [
  { key: 'today' as const, label: '今天' },
  { key: '7d' as const, label: '7 天' },
  { key: '30d' as const, label: '30 天' },
  { key: 'custom' as const, label: '自定义' },
]

export const UsageFilterBar = memo(function UsageFilterBar({
  apiKeys,
  filterKeyId,
  onFilterKeyIdChange,
  filterModel,
  onFilterModelChange,
  dateRange,
  onDateRangeChange,
  startDate,
  onStartDateChange,
  endDate,
  onEndDateChange,
  onFilter,
  onRefresh,
  onExport,
  loading,
  exporting,
  total,
}: UsageFilterBarProps) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <div className="flex flex-wrap items-center gap-2 min-w-0 flex-1">
        <div className="flex rounded-[8px] border border-border bg-card p-0.5 shadow-[0_1px_2px_rgb(0_0_0/0.07)]">
          {RANGES.map((r) => (
            <button
              key={r.key}
              type="button"
              onClick={() => onDateRangeChange(r.key)}
              className={`min-h-8 rounded-[7px] px-3 text-xs font-semibold transition-colors ${
                dateRange === r.key
                  ? 'bg-primary text-primary-foreground'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              {r.label}
            </button>
          ))}
        </div>

        {dateRange === 'custom' ? (
          <div className="flex items-center gap-1.5">
            <input
              type="date"
              value={startDate}
              onChange={(e) => onStartDateChange(e.target.value)}
              className="usage-control w-[138px]"
            />
            <span className="text-xs text-muted-foreground">至</span>
            <input
              type="date"
              value={endDate}
              onChange={(e) => onEndDateChange(e.target.value)}
              className="usage-control w-[138px]"
            />
          </div>
        ) : null}

        <select
          value={filterKeyId}
          onChange={(e) => onFilterKeyIdChange(e.target.value)}
          className="usage-control min-w-[140px]"
          aria-label="API Key"
        >
          <option value="">全部 Key</option>
          {apiKeys.map((k) => (
            <option key={k.id} value={k.id}>{k.name}</option>
          ))}
        </select>

        <div className="relative min-w-[160px] flex-1 max-w-[240px]">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            type="search"
            value={filterModel}
            onChange={(e) => onFilterModelChange(e.target.value)}
            placeholder="搜索模型"
            className="usage-control w-full pl-8"
            onKeyDown={(e) => { if (e.key === 'Enter') onFilter() }}
          />
          {filterModel ? (
            <button
              type="button"
              onClick={() => onFilterModelChange('')}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              aria-label="清除模型筛选"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          ) : null}
        </div>
      </div>

      <div className="ml-auto flex items-center gap-2">
        <button
          type="button"
          onClick={onExport}
          disabled={exporting || total === 0}
          className="usage-control inline-flex items-center gap-1.5 px-3 font-semibold disabled:opacity-50"
        >
          {exporting ? <RefreshCw className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
          {exporting ? '导出中' : '导出 CSV'}
        </button>
        <button
          type="button"
          onClick={onRefresh}
          disabled={loading}
          className="inline-flex min-h-[38px] items-center gap-1.5 rounded-[8px] bg-primary px-3.5 text-xs font-semibold text-primary-foreground shadow-[0_1px_2px_rgb(0_0_0/0.07)] hover:bg-[#0d9488] disabled:opacity-50"
        >
          <RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
          刷新
        </button>
      </div>
    </div>
  )
})
