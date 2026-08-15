import { useState, useMemo, useCallback } from 'react'
import { useAuthStore } from '@/features/auth/auth_store'
import { Search, RefreshCw, Cpu, Pencil } from 'lucide-react'
import {
  getModelProviderKey,
  getProviderDisplayName,
  getProviderOptions,
  matchesModelSearch,
} from '@/features/pricing/model_catalog'
import { ModelCatalogCard } from '@/features/pricing/components/ModelCatalogCard'
import { getProviderStyle } from '@/features/pricing/modelCatalogUtils'
import { useModels } from '@/features/pricing/hooks'

// ── Main Component ──────────────────────────────────────────────────────────

export default function Models() {
  const [search, setSearch] = useState('')
  const [filterProvider, setFilterProvider] = useState('all')
  const [copiedId, setCopiedId] = useState<string | null>(null)

  const user = useAuthStore((s) => s.user)
  const isAdmin = user?.role === 'admin'

  const { models, rateMultiplier, isLoading: loading, refetch } = useModels()

  const providers = useMemo(() => {
    return getProviderOptions(models)
  }, [models])

  const filtered = useMemo(() => {
    let items = [...models]
    if (filterProvider !== 'all') {
      items = items.filter((m) => getModelProviderKey(m) === filterProvider)
    }
    if (search.trim()) {
      items = items.filter(m => matchesModelSearch(m, search))
    }
    return items
  }, [models, filterProvider, search])

  const handleCopy = useCallback((id: string) => {
    navigator.clipboard.writeText(id)
    setCopiedId(id)
    setTimeout(() => setCopiedId(null), 1500)
  }, [])

  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500" style={{ willChange: 'transform, opacity' }}>
      {/* Header */}
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">模型</h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1">
          当前 API 代理网关支持的所有 AI 模型及其定价信息。
          {rateMultiplier !== 1 && (
            <span className="ml-2 inline-flex items-center rounded-md bg-primary-50 dark:bg-primary-950/30 px-2 py-0.5 text-xs font-medium text-primary-700 dark:text-primary-300">
              当前倍率 ×{rateMultiplier}
            </span>
          )}
          {isAdmin && (
            <span className="ml-2 inline-flex items-center rounded-md bg-amber-50 dark:bg-amber-950/30 px-2 py-0.5 text-xs font-medium text-amber-700 dark:text-amber-300 gap-1">
              <Pencil className="h-3 w-3" />
              点击价格可直接编辑
            </span>
          )}
        </p>
      </div>

      {/* Toolbar */}
      <div className="flex flex-col sm:flex-row gap-3 items-start sm:items-center justify-between">
        <div className="flex flex-wrap items-center gap-2">
          {/* Provider filter pills */}
          <button
            onClick={() => setFilterProvider('all')}
            className={`inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition-colors ${
              filterProvider === 'all'
                ? 'border-primary-300 bg-primary-50 text-primary-700 dark:border-primary-800 dark:bg-primary-950/30 dark:text-primary-300'
                : 'border-gray-200 dark:border-dark-600 text-gray-500 dark:text-gray-400 hover:border-gray-300 dark:hover:border-dark-500'
            }`}
          >
            全部
            <span className="rounded bg-black/5 dark:bg-white/10 px-1.5 py-0.5 text-[10px] tabular-nums">{models.length}</span>
          </button>
          {providers.map(({ key, label, count }) => {
            const style = getProviderStyle(key)
            return (
              <button
                key={key}
                onClick={() => setFilterProvider(key)}
                className={`inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium capitalize transition-colors ${
                  filterProvider === key
                    ? `${style.border} ${style.bg} ${style.text}`
                    : 'border-gray-200 dark:border-dark-600 text-gray-500 dark:text-gray-400 hover:border-gray-300 dark:hover:border-dark-500'
                }`}
              >
                {label}
                <span className="rounded bg-black/5 dark:bg-white/10 px-1.5 py-0.5 text-[10px] tabular-nums">{count}</span>
              </button>
            )
          })}
        </div>

        <div className="flex items-center gap-2">
          <div className="relative">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-400" />
            <input
              className="h-9 w-56 rounded-lg border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 pl-8 pr-3 text-sm text-gray-700 dark:text-gray-200 placeholder:text-gray-400 outline-none focus:border-primary-400 focus:ring-1 focus:ring-primary-400/30 transition-colors"
              placeholder="搜索模型、名称或供应商..."
              value={search}
              onChange={e => setSearch(e.target.value)}
            />
          </div>
          <button
            onClick={() => refetch()}
            disabled={loading}
            className="h-9 rounded-lg border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 px-3 text-sm font-medium text-gray-600 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-dark-800 transition-colors inline-flex items-center gap-1.5 disabled:opacity-50"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
            刷新
          </button>
        </div>
      </div>

      {/* Models Grid */}
      {loading ? (
        <div className="flex items-center justify-center py-20">
          <RefreshCw className="h-6 w-6 animate-spin text-primary-500" />
          <span className="ml-3 text-gray-500 dark:text-dark-400">加载模型列表...</span>
        </div>
      ) : filtered.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-20 text-gray-400 dark:text-dark-500">
          <Cpu className="h-12 w-12 mb-3 opacity-50" />
          <p className="text-sm">{models.length === 0 ? '暂无可用模型' : '没有匹配的模型'}</p>
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {filtered.map(m => (
            <ModelCatalogCard
              key={m.id}
              model={m}
              isAdmin={isAdmin}
              copied={copiedId === m.id}
              onCopy={handleCopy}
              onPriceSaved={() => refetch()}
            />
          ))}
        </div>
      )}

      {/* Footer */}
      {!loading && filtered.length > 0 && (
        <div className="text-xs text-gray-400 dark:text-dark-500 text-center pb-2 tabular-nums">
          共 {filtered.length} 个模型{filterProvider !== 'all' ? `（${getProviderDisplayName(filterProvider)}）` : ''}
          {filtered.length !== models.length && ` / 总计 ${models.length} 个`}
        </div>
      )}
    </div>
  )
}
