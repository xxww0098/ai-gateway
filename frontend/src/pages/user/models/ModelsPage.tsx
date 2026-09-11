import { useState, useMemo, useCallback, useEffect } from 'react'
import { useAuthStore } from '@/features/auth/auth_store'
import { isStaff } from '@/shared/role_core'
import {
  getModelProviderKey,
  getProviderDisplayName,
  getProviderOptions,
  matchesModelSearch,
  type ModelCatalogItem,
} from '@/features/pricing/model_catalog'
import { ModelCatalogCard } from '@/features/pricing/components/ModelCatalogCard'
import { ModelCatalogTable } from '@/features/pricing/components/ModelCatalogTable'
import { ModelCatalogHeader } from '@/features/pricing/components/ModelCatalogHeader'
import {
  ModelCatalogFilters,
  type CapabilityFilter,
  type SortOption,
  type ViewMode,
} from '@/features/pricing/components/ModelCatalogFilters'
import { ModelEmptyState } from '@/features/pricing/components/ModelEmptyState'
import { ModelQuickCodeDialog } from '@/features/pricing/components/ModelQuickCodeDialog'
import { ModelCatalogSkeleton } from '@/features/pricing/components/ModelCatalogSkeleton'
import { useModels } from '@/features/pricing/hooks'

const VIEW_MODE_STORAGE_KEY = 'agw_models_view_mode'

export default function Models() {
  const [search, setSearch] = useState('')
  const [filterProvider, setFilterProvider] = useState('all')
  const [filterCapability, setFilterCapability] = useState<CapabilityFilter>('all')
  const [sort, setSort] = useState<SortOption>('default')
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    if (typeof window !== 'undefined') {
      const stored = localStorage.getItem(VIEW_MODE_STORAGE_KEY)
      if (stored === 'grid' || stored === 'table') return stored
    }
    return 'grid'
  })

  const [copiedId, setCopiedId] = useState<string | null>(null)
  const [quickCodeModel, setQuickCodeModel] = useState<ModelCatalogItem | null>(null)
  const [quickCodeOpen, setQuickCodeOpen] = useState(false)

  const user = useAuthStore((s) => s.user)
  const isAdmin = isStaff(user?.role)

  const { models, rateMultiplier, isLoading: loading, refetch } = useModels()

  const handleViewModeChange = useCallback((mode: ViewMode) => {
    setViewMode(mode)
    try {
      localStorage.setItem(VIEW_MODE_STORAGE_KEY, mode)
    } catch {
      // ignore storage errors
    }
  }, [])

  const providers = useMemo(() => {
    return getProviderOptions(models)
  }, [models])

  const filtered = useMemo(() => {
    let items = [...models]

    // 1. Provider filter
    if (filterProvider !== 'all') {
      items = items.filter((m) => getModelProviderKey(m) === filterProvider)
    }

    // 2. Capability filter
    if (filterCapability === 'reasoning') {
      items = items.filter((m) => Boolean(m.thinking || (m.reasoning_price_per_1m ?? 0) > 0))
    } else if (filterCapability === 'vision') {
      items = items.filter((m) =>
        Boolean(
          m.supportedInputModalities?.includes('image') ||
            m.supportedOutputModalities?.includes('image') ||
            m.id.toLowerCase().includes('vision') ||
            m.id.toLowerCase().includes('4o') ||
            m.id.toLowerCase().includes('omni')
        )
      )
    } else if (filterCapability === 'longContext') {
      items = items.filter((m) => {
        const limit = m.context_length ?? m.inputTokenLimit ?? 0
        return limit >= 128000
      })
    } else if (filterCapability === 'caching') {
      items = items.filter((m) => (m.cached_input_price_per_1m ?? 0) > 0)
    }

    // 3. Search filter
    if (search.trim()) {
      items = items.filter((m) => matchesModelSearch(m, search))
    }

    // 4. Sorting
    if (sort === 'price_input_asc') {
      items.sort((a, b) => (a.input_price_per_1m ?? 0) - (b.input_price_per_1m ?? 0))
    } else if (sort === 'price_input_desc') {
      items.sort((a, b) => (b.input_price_per_1m ?? 0) - (a.input_price_per_1m ?? 0))
    } else if (sort === 'price_output_asc') {
      items.sort((a, b) => (a.output_price_per_1m ?? 0) - (b.output_price_per_1m ?? 0))
    } else if (sort === 'context_desc') {
      items.sort((a, b) => {
        const aLimit = a.context_length ?? a.inputTokenLimit ?? 0
        const bLimit = b.context_length ?? b.inputTokenLimit ?? 0
        return bLimit - aLimit
      })
    } else if (sort === 'name_asc') {
      items.sort((a, b) => a.id.localeCompare(b.id))
    }

    return items
  }, [models, filterProvider, filterCapability, search, sort])

  const handleCopy = useCallback((id: string) => {
    navigator.clipboard.writeText(id)
    setCopiedId(id)
    setTimeout(() => setCopiedId(null), 1500)
  }, [])

  const handleOpenCode = useCallback((model: ModelCatalogItem) => {
    setQuickCodeModel(model)
    setQuickCodeOpen(true)
  }, [])

  const handleClearFilters = useCallback(() => {
    setSearch('')
    setFilterProvider('all')
    setFilterCapability('all')
    setSort('default')
  }, [])

  // Keyboard shortcut '/' to search
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === '/' && document.activeElement?.tagName !== 'INPUT' && document.activeElement?.tagName !== 'TEXTAREA') {
        e.preventDefault()
        const input = document.querySelector<HTMLInputElement>('input[placeholder*="搜索"]')
        input?.focus()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [])

  return (
    <div className="space-y-6">
      {/* Header Banner */}
      <ModelCatalogHeader
        totalModels={models.length}
        totalProviders={providers.length}
        rateMultiplier={rateMultiplier}
      />

      {/* Filters Toolbar */}
      <ModelCatalogFilters
        totalCount={models.length}
        providers={providers}
        selectedProvider={filterProvider}
        onSelectProvider={setFilterProvider}
        selectedCapability={filterCapability}
        onSelectCapability={setFilterCapability}
        search={search}
        onSearchChange={setSearch}
        sort={sort}
        onSortChange={setSort}
        viewMode={viewMode}
        onViewModeChange={handleViewModeChange}
        loading={loading}
        onRefresh={() => refetch()}
      />

      {/* Main Content Area */}
      {loading ? (
        <ModelCatalogSkeleton viewMode={viewMode} />
      ) : filtered.length === 0 ? (
        models.length === 0 ? (
          <ModelEmptyState isAdmin={isAdmin} />
        ) : (
          <ModelEmptyState
            isAdmin={isAdmin}
            isFilterMiss
            onClearFilter={handleClearFilters}
          />
        )
      ) : viewMode === 'table' ? (
        <ModelCatalogTable
          models={filtered}
          isAdmin={isAdmin}
          copiedId={copiedId}
          onCopy={handleCopy}
          onOpenCode={handleOpenCode}
          onPriceSaved={() => refetch()}
        />
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {filtered.map((m) => (
            <ModelCatalogCard
              key={m.id}
              model={m}
              isAdmin={isAdmin}
              copied={copiedId === m.id}
              onCopy={handleCopy}
              onOpenCode={handleOpenCode}
              onPriceSaved={() => refetch()}
            />
          ))}
        </div>
      )}

      {/* Footer Info */}
      {!loading && filtered.length > 0 && (
        <div className="flex flex-col sm:flex-row items-center justify-between gap-2 text-xs text-muted-foreground pt-2 border-t border-border/40 tabular-nums">
          <div>
            显示 {filtered.length} 个模型
            {filterProvider !== 'all' && `（${getProviderDisplayName(filterProvider)}）`}
            {filtered.length !== models.length && ` / 共 ${models.length} 个`}
          </div>
          <div>按真实 Token 精算 · 输入 / 输出 / 缓存 / 推理四列独立单价</div>
        </div>
      )}

      {/* Quick Integration Code Dialog */}
      <ModelQuickCodeDialog
        model={quickCodeModel}
        open={quickCodeOpen}
        onOpenChange={setQuickCodeOpen}
      />
    </div>
  )
}
