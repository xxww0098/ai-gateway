import { memo } from 'react'
import { Search, RefreshCw, Download, X } from 'lucide-react'
import type { UsageFilterBarProps } from '../types'

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
  onExport,
  loading,
  exporting,
  total,
}: UsageFilterBarProps) {
  return (
    <div className="card">
      <div className="px-5 py-4">
        <div className="flex flex-wrap items-end gap-3">
          {/* API Key select */}
          <div className="min-w-[160px]">
            <label className="text-[11px] font-semibold text-gray-500 dark:text-dark-400 uppercase tracking-wider mb-1 block">API Key</label>
            <select
              value={filterKeyId}
              onChange={(e) => onFilterKeyIdChange(e.target.value)}
              className="input h-9 text-sm"
            >
              <option value="">全部 Key</option>
              {apiKeys.map(k => (
                <option key={k.id} value={k.id}>{k.name}</option>
              ))}
            </select>
          </div>

          {/* Model search */}
          <div className="min-w-[140px]">
            <label className="text-[11px] font-semibold text-gray-500 dark:text-dark-400 uppercase tracking-wider mb-1 block">模型</label>
            <div className="relative">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-gray-400 pointer-events-none" />
              <input
                type="text"
                value={filterModel}
                onChange={(e) => onFilterModelChange(e.target.value)}
                placeholder="搜索模型..."
                className="input h-9 text-sm pl-8"
                onKeyDown={(e) => { if (e.key === 'Enter') onFilter() }}
              />
              {filterModel && (
                <button onClick={() => { onFilterModelChange('') }} className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600">
                  <X className="w-3.5 h-3.5" />
                </button>
              )}
            </div>
          </div>

          {/* Date presets */}
          <div>
            <label className="text-[11px] font-semibold text-gray-500 dark:text-dark-400 uppercase tracking-wider mb-1 block">时间范围</label>
            <div className="flex gap-1">
              {([
                { key: 'today' as const, label: '今天' },
                { key: '7d' as const, label: '7天' },
                { key: '30d' as const, label: '30天' },
                { key: 'custom' as const, label: '自定义' },
              ]).map(r => (
                <button
                  key={r.key}
                  onClick={() => r.key !== 'custom' ? onDateRangeChange(r.key) : onDateRangeChange('custom')}
                  className={`px-3 py-1.5 rounded-lg text-xs font-medium transition-all ${
                    dateRange === r.key
                      ? 'bg-primary-500 text-white shadow-sm'
                      : 'bg-gray-100 dark:bg-dark-800 text-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-dark-700'
                  }`}
                >
                  {r.label}
                </button>
              ))}
            </div>
          </div>

          {/* Custom dates */}
          {dateRange === 'custom' && (
            <div className="flex items-center gap-2">
              <input type="date" value={startDate} onChange={e => onStartDateChange(e.target.value)} className="input h-9 text-xs w-[130px]" />
              <span className="text-gray-400 text-xs">至</span>
              <input type="date" value={endDate} onChange={e => onEndDateChange(e.target.value)} className="input h-9 text-xs w-[130px]" />
            </div>
          )}

          {/* Actions */}
          <div className="ml-auto flex items-center gap-2">
            <button onClick={onFilter} disabled={loading} className="btn btn-secondary h-9 px-3 text-xs">
              <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
              刷新
            </button>
            <button onClick={onExport} disabled={exporting || total === 0} className="btn btn-primary h-9 px-3 text-xs">
              {exporting ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
              {exporting ? '导出中...' : 'CSV'}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
})
