import { useRef } from 'react'
import {
  Search,
  X,
  RefreshCw,
  LayoutGrid,
  Table as TableIcon,
  Brain,
  Eye,
  Layers,
  Database,
  ArrowUpDown,
  Sparkles,
} from 'lucide-react'
import type { ProviderOption } from '@/features/pricing/model_catalog'
import { getProviderStyle, getProviderBrandIcon } from '@/features/pricing/modelCatalogUtils'

export type CapabilityFilter = 'all' | 'reasoning' | 'vision' | 'longContext' | 'caching'
export type SortOption = 'default' | 'price_input_asc' | 'price_input_desc' | 'price_output_asc' | 'context_desc' | 'name_asc'
export type ViewMode = 'grid' | 'table'

interface ModelCatalogFiltersProps {
  totalCount: number
  providers: ProviderOption[]
  selectedProvider: string
  onSelectProvider: (provider: string) => void
  selectedCapability: CapabilityFilter
  onSelectCapability: (cap: CapabilityFilter) => void
  search: string
  onSearchChange: (value: string) => void
  sort: SortOption
  onSortChange: (sort: SortOption) => void
  viewMode: ViewMode
  onViewModeChange: (mode: ViewMode) => void
  loading: boolean
  onRefresh: () => void
}

export function ModelCatalogFilters({
  totalCount,
  providers,
  selectedProvider,
  onSelectProvider,
  selectedCapability,
  onSelectCapability,
  search,
  onSearchChange,
  sort,
  onSortChange,
  viewMode,
  onViewModeChange,
  loading,
  onRefresh,
}: ModelCatalogFiltersProps) {
  const searchInputRef = useRef<HTMLInputElement>(null)

  return (
    <div className="space-y-3.5">
      {/* Provider Filter Bar with Brand Icons */}
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => onSelectProvider('all')}
          className={`inline-flex items-center gap-2 rounded-xl border px-3.5 py-2 text-xs font-bold transition-all duration-150 active:scale-[0.98] ${
            selectedProvider === 'all'
              ? 'border-primary/40 bg-primary/15 text-primary shadow-2xs ring-1 ring-primary/30'
              : 'border-border/80 bg-card text-muted-foreground hover:text-foreground hover:bg-muted/60'
          }`}
        >
          <Sparkles className="h-3.5 w-3.5" />
          <span>全部厂商</span>
          <span className="rounded-md bg-foreground/10 px-1.5 py-0.5 text-[10px] font-bold tabular-nums">
            {totalCount}
          </span>
        </button>

        {providers.map(({ key, label, count }) => {
          const style = getProviderStyle(key)
          const BrandIcon = getProviderBrandIcon(key)
          const isSelected = selectedProvider === key

          return (
            <button
              key={key}
              type="button"
              onClick={() => onSelectProvider(key)}
              className={`inline-flex items-center gap-2 rounded-xl border px-3 py-2 text-xs font-bold transition-all duration-150 active:scale-[0.98] ${
                isSelected
                  ? `${style.border} ${style.bg} ${style.text} shadow-2xs ring-1 ring-primary/30`
                  : 'border-border/80 bg-card text-muted-foreground hover:text-foreground hover:bg-muted/60'
              }`}
            >
              {BrandIcon ? (
                <BrandIcon size={14} className="shrink-0" />
              ) : (
                <span className="h-2 w-2 rounded-full bg-primary/60" />
              )}
              <span>{label}</span>
              <span className="rounded-md bg-foreground/10 px-1.5 py-0.5 text-[10px] font-bold tabular-nums">
                {count}
              </span>
            </button>
          )
        })}
      </div>

      {/* Secondary Controls Row: Capability Tabs + Search + View Switcher */}
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        {/* Capability Tags */}
        <div className="flex flex-wrap items-center gap-1.5">
          <button
            type="button"
            onClick={() => onSelectCapability('all')}
            className={`inline-flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-xs font-medium transition ${
              selectedCapability === 'all'
                ? 'bg-secondary text-secondary-foreground font-semibold shadow-2xs'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground'
            }`}
          >
            全部特性
          </button>
          <button
            type="button"
            onClick={() => onSelectCapability('reasoning')}
            className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition ${
              selectedCapability === 'reasoning'
                ? 'bg-purple-100 text-purple-800 dark:bg-purple-950/40 dark:text-purple-300 font-semibold ring-1 ring-purple-500/30'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground'
            }`}
          >
            <Brain className="h-3 w-3 text-purple-500" />
            深度思考 / 推理
          </button>
          <button
            type="button"
            onClick={() => onSelectCapability('vision')}
            className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition ${
              selectedCapability === 'vision'
                ? 'bg-blue-100 text-blue-800 dark:bg-blue-950/40 dark:text-blue-300 font-semibold ring-1 ring-blue-500/30'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground'
            }`}
          >
            <Eye className="h-3 w-3 text-blue-500" />
            视觉多模态
          </button>
          <button
            type="button"
            onClick={() => onSelectCapability('longContext')}
            className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition ${
              selectedCapability === 'longContext'
                ? 'bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300 font-semibold ring-1 ring-amber-500/30'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground'
            }`}
          >
            <Layers className="h-3 w-3 text-amber-500" />
            长上下文 (≥128K)
          </button>
          <button
            type="button"
            onClick={() => onSelectCapability('caching')}
            className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition ${
              selectedCapability === 'caching'
                ? 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300 font-semibold ring-1 ring-emerald-500/30'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground'
            }`}
          >
            <Database className="h-3 w-3 text-emerald-500" />
            上下文缓存
          </button>
        </div>

        {/* Search, Sort, View Toggle, Refresh */}
        <div className="flex flex-wrap items-center gap-2">
          {/* Search Box */}
          <div className="relative min-w-[200px] flex-1 sm:w-64 sm:flex-initial">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <input
              ref={searchInputRef}
              type="text"
              className="h-9 w-full rounded-xl border border-border bg-card pl-9 pr-8 text-xs font-medium text-foreground placeholder:text-muted-foreground outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20 shadow-2xs"
              placeholder="搜索模型、标识或描述..."
              value={search}
              onChange={(e) => onSearchChange(e.target.value)}
            />
            {search && (
              <button
                type="button"
                onClick={() => {
                  onSearchChange('')
                  searchInputRef.current?.focus()
                }}
                className="absolute right-2.5 top-1/2 -translate-y-1/2 rounded-md p-0.5 text-muted-foreground hover:text-foreground"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>

          {/* Sort Dropdown */}
          <div className="relative">
            <select
              value={sort}
              onChange={(e) => onSortChange(e.target.value as SortOption)}
              className="h-9 rounded-xl border border-border bg-card px-2.5 pr-7 text-xs font-medium text-foreground shadow-2xs outline-none transition hover:bg-muted/60 focus:border-primary focus:ring-2 focus:ring-primary/20 appearance-none cursor-pointer"
            >
              <option value="default">默认推荐排序</option>
              <option value="price_input_asc">输入价格：低到高</option>
              <option value="price_input_desc">输入价格：高到低</option>
              <option value="price_output_asc">输出价格：低到高</option>
              <option value="context_desc">上下文：从大到小</option>
              <option value="name_asc">模型标识：A - Z</option>
            </select>
            <ArrowUpDown className="pointer-events-none absolute right-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground" />
          </div>

          {/* View Mode Switcher */}
          <div className="flex items-center rounded-xl border border-border bg-card p-0.5 shadow-2xs">
            <button
              type="button"
              onClick={() => onViewModeChange('grid')}
              className={`flex h-8 w-8 items-center justify-center rounded-lg transition ${
                viewMode === 'grid'
                  ? 'bg-muted text-foreground shadow-2xs font-semibold'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
              title="卡片网格视图"
            >
              <LayoutGrid className="h-4 w-4" />
            </button>
            <button
              type="button"
              onClick={() => onViewModeChange('table')}
              className={`flex h-8 w-8 items-center justify-center rounded-lg transition ${
                viewMode === 'table'
                  ? 'bg-muted text-foreground shadow-2xs font-semibold'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
              title="紧凑表格视图"
            >
              <TableIcon className="h-4 w-4" />
            </button>
          </div>

          {/* Refresh Button */}
          <button
            type="button"
            onClick={onRefresh}
            disabled={loading}
            className="flex h-9 items-center gap-1.5 rounded-xl border border-border bg-card px-3 text-xs font-semibold text-muted-foreground hover:text-foreground hover:bg-muted shadow-2xs transition disabled:opacity-50 active:scale-[0.98]"
            title="刷新模型列表"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
            <span className="hidden sm:inline">刷新</span>
          </button>
        </div>
      </div>
    </div>
  )
}
