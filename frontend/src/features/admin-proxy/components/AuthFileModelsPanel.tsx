import { RefreshCw, Cpu } from 'lucide-react'
import type { AuthFileModelsPanelProps } from '../types'

export function AuthFileModelsPanel({ models, loading, onRefresh }: AuthFileModelsPanelProps) {
  return (
    <div className="rounded-xl border border-gray-200 dark:border-dark-600 overflow-hidden">
      <div className="flex items-center justify-between px-4 py-2.5 bg-gray-50/80 dark:bg-dark-800/60 border-b border-gray-200 dark:border-dark-600">
        <span className="text-xs font-medium text-gray-700 dark:text-gray-200 inline-flex items-center gap-1.5">
          <Cpu className="h-3.5 w-3.5 text-violet-500" /> 已注册模型
          {models && models.length > 0 && (
            <span className="rounded bg-violet-100 dark:bg-violet-900/30 px-1.5 py-0.5 text-[10px] text-violet-700 dark:text-violet-300 tabular-nums">
              {models.length}
            </span>
          )}
        </span>
        <button
          className="text-[11px] text-primary-500 hover:text-primary-600 font-medium disabled:opacity-50 inline-flex items-center gap-1"
          onClick={onRefresh}
          disabled={loading}
        >
          <RefreshCw className={`h-3 w-3 ${loading ? 'animate-spin' : ''}`} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>
      <div className="px-4 py-3">
        {loading && !models && (
          <div className="flex items-center justify-center py-4">
            <RefreshCw className="h-4 w-4 animate-spin text-primary-500" />
            <span className="ml-2 text-sm text-gray-500">正在查询已注册模型...</span>
          </div>
        )}
        {models && models.length === 0 && !loading && (
          <p className="text-xs text-gray-500 dark:text-dark-400 text-center py-2">此凭证暂无已注册模型</p>
        )}
        {models && models.length > 0 && (
          <div className="grid grid-cols-1 gap-1.5 max-h-[200px] overflow-y-auto">
            {models.map(m => (
              <div key={m.id} className="flex items-center justify-between rounded-lg bg-gray-50 dark:bg-dark-800 px-3 py-1.5">
                <div className="min-w-0 flex-1">
                  <span className="text-xs font-medium text-gray-900 dark:text-white truncate block" title={m.id}>{m.id}</span>
                  {m.display_name && m.display_name !== m.id && (
                    <span className="text-[10px] text-gray-500 dark:text-dark-400 truncate block">{m.display_name}</span>
                  )}
                </div>
                {m.type && (
                  <span className="flex-shrink-0 ml-2 inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium bg-gray-200 dark:bg-dark-600 text-gray-600 dark:text-gray-300 capitalize">
                    {m.type}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
