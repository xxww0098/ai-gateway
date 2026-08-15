import { RefreshCw, Activity } from 'lucide-react'
import type { AuthFileQuotaPanelProps } from '../types'

export function AuthFileQuotaPanel({ item, quota, loading, onRefresh }: AuthFileQuotaPanelProps) {
  const hasAuthIndex = (item.auth_index ?? item.authIndex) !== undefined
  if (!hasAuthIndex && !quota) return null

  return (
    <div className="rounded-xl border border-gray-200 dark:border-dark-600 overflow-hidden">
      <div className="flex items-center justify-between px-4 py-2.5 bg-gray-50/80 dark:bg-dark-800/60 border-b border-gray-200 dark:border-dark-600">
        <span className="text-xs font-medium text-gray-700 dark:text-gray-200 inline-flex items-center gap-1.5">
          <Activity className="h-3.5 w-3.5 text-sky-500" /> 账号额度
          {quota?.planLabel && <span className="rounded bg-sky-100 dark:bg-sky-900/30 px-1.5 py-0.5 text-[10px] text-sky-700 dark:text-sky-300">{quota.planLabel}</span>}
        </span>
        <button
          className="text-[11px] text-primary-500 hover:text-primary-600 font-medium disabled:opacity-50 inline-flex items-center gap-1"
          onClick={onRefresh}
          disabled={loading}
        >
          <RefreshCw className={`h-3 w-3 ${loading ? 'animate-spin' : ''}`} />
          {loading ? '查询中' : '刷新'}
        </button>
      </div>
      <div className="px-4 py-3 space-y-2.5">
        {loading && !quota && (
          <div className="flex items-center justify-center py-4">
            <RefreshCw className="h-4 w-4 animate-spin text-primary-500" />
            <span className="ml-2 text-sm text-gray-500">正在查询额度...</span>
          </div>
        )}
        {quota?.error && (
          <div className="rounded-lg bg-red-50 dark:bg-red-950/30 border border-red-200 dark:border-red-800/50 px-3 py-2 text-xs text-red-600 dark:text-red-400">
            {quota.error}
          </div>
        )}
        {quota && quota.windows.length === 0 && !quota.error && (
          <p className="text-xs text-gray-500 dark:text-dark-400 text-center py-2">未获取到额度数据</p>
        )}
        {quota?.windows.map(w => {
          const remaining = w.usedPercent !== null ? Math.max(0, Math.min(100, 100 - w.usedPercent)) : null
          const barColor = remaining === null ? 'bg-gray-300 dark:bg-dark-600'
            : remaining > 60 ? 'bg-emerald-500'
            : remaining > 20 ? 'bg-amber-500'
            : 'bg-red-500'
          return (
            <div key={w.id} className="space-y-1">
              <div className="flex items-center justify-between">
                <span className="text-xs text-gray-700 dark:text-gray-300 truncate max-w-[200px]" title={w.label}>{w.label}</span>
                <div className="flex items-center gap-2 text-[11px] tabular-nums">
                  <span className={`font-medium ${remaining !== null && remaining <= 20 ? 'text-red-500' : 'text-gray-600 dark:text-gray-400'}`}>
                    {remaining !== null ? `${Math.round(remaining)}%` : '--'}
                  </span>
                  <span className="text-gray-400 dark:text-dark-500">{w.resetLabel}</span>
                </div>
              </div>
              <div className="h-1.5 rounded-full bg-gray-100 dark:bg-dark-700 overflow-hidden">
                <div
                  className={`h-full rounded-full transition-all duration-500 ${barColor}`}
                  style={{ width: `${remaining ?? 0}%` }}
                />
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
