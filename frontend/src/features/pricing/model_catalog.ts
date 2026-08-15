import { getModelRegistryMetadata, resolveModelProvider } from './model_provider'

interface ModelThinkingInfo {
  min?: number
  max?: number
  zero_allowed?: boolean
  dynamic_allowed?: boolean
  levels?: string[]
}

export interface ModelCatalogItem {
  id: string
  object?: string
  owned_by?: string
  type?: string
  name?: string
  display_name?: string
  description?: string
  version?: string
  created?: number
  context_length?: number
  max_completion_tokens?: number
  inputTokenLimit?: number
  outputTokenLimit?: number
  thinking?: ModelThinkingInfo
  supported_parameters?: string[]
  supportedGenerationMethods?: string[]
  supportedInputModalities?: string[]
  supportedOutputModalities?: string[]
  input_price_per_1m?: number
  output_price_per_1m?: number
  reasoning_price_per_1m?: number
  cached_input_price_per_1m?: number
  base_price_input?: number
  rate_multiplier?: number
}

export interface ProviderOption {
  key: string
  label: string
  count: number
}

export interface ModelDetailMetric {
  label: string
  value: string
}

export interface ConfiguredModelInfo {
  name?: string
  alias?: string
  description?: string
}

export interface ProviderColor {
  bg: string
  text: string
  border: string
}

interface ProviderConfigEntry {
  label: string
  color: ProviderColor
  icon: string
}

export const PROVIDER_CONFIG: Record<string, ProviderConfigEntry> = {
  anthropic: {
    label: 'Anthropic',
    color: { bg: 'bg-orange-50 dark:bg-orange-950/20', text: 'text-orange-700 dark:text-orange-300', border: 'border-orange-200 dark:border-orange-800/40' },
    icon: 'anthropic',
  },
  openai: {
    label: 'OpenAI',
    color: { bg: 'bg-emerald-50 dark:bg-emerald-950/20', text: 'text-emerald-700 dark:text-emerald-300', border: 'border-emerald-200 dark:border-emerald-800/40' },
    icon: 'openai',
  },
  google: {
    label: 'Google',
    color: { bg: 'bg-blue-50 dark:bg-blue-950/20', text: 'text-blue-700 dark:text-blue-300', border: 'border-blue-200 dark:border-blue-800/40' },
    icon: 'google',
  },
  kimi: {
    label: 'Kimi',
    color: { bg: 'bg-slate-50 dark:bg-slate-950/20', text: 'text-slate-700 dark:text-slate-300', border: 'border-slate-200 dark:border-slate-800/40' },
    icon: 'kimi',
  },
  minimax: {
    label: 'MiniMax',
    color: { bg: 'bg-cyan-50 dark:bg-cyan-950/20', text: 'text-cyan-700 dark:text-cyan-300', border: 'border-cyan-200 dark:border-cyan-800/40' },
    icon: 'minimax',
  },
  glm: {
    label: 'GLM',
    color: { bg: 'bg-teal-50 dark:bg-teal-950/20', text: 'text-teal-700 dark:text-teal-300', border: 'border-teal-200 dark:border-teal-800/40' },
    icon: 'glm',
  },
  mimo: {
    label: 'MiMo',
    color: { bg: 'bg-rose-50 dark:bg-rose-950/20', text: 'text-rose-700 dark:text-rose-300', border: 'border-rose-200 dark:border-rose-800/40' },
    icon: 'mimo',
  },
  moonshot: {
    label: 'Moonshot',
    color: { bg: 'bg-indigo-50 dark:bg-indigo-950/20', text: 'text-indigo-700 dark:text-indigo-300', border: 'border-indigo-200 dark:border-indigo-800/40' },
    icon: 'moonshot',
  },
  antigravity: {
    label: 'Antigravity',
    color: { bg: 'bg-purple-50 dark:bg-purple-950/20', text: 'text-purple-700 dark:text-purple-300', border: 'border-purple-200 dark:border-purple-800/40' },
    icon: 'antigravity',
  },
  deepseek: {
    label: 'DeepSeek',
    color: { bg: 'bg-violet-50 dark:bg-violet-950/20', text: 'text-violet-700 dark:text-violet-300', border: 'border-violet-200 dark:border-violet-800/40' },
    icon: 'deepseek',
  },
  meta: {
    label: 'Meta',
    color: { bg: 'bg-sky-50 dark:bg-sky-950/20', text: 'text-sky-700 dark:text-sky-300', border: 'border-sky-200 dark:border-sky-800/40' },
    icon: 'meta',
  },
  other: {
    label: 'Other',
    color: { bg: 'bg-gray-50 dark:bg-dark-800', text: 'text-gray-600 dark:text-gray-300', border: 'border-gray-200 dark:border-dark-600' },
    icon: 'other',
  },
}

const DEFAULT_PROVIDER_COLOR: ProviderColor = { bg: 'bg-gray-50 dark:bg-dark-800', text: 'text-gray-600 dark:text-gray-300', border: 'border-gray-200 dark:border-dark-600' }

export function getProviderColor(provider: string): ProviderColor {
  return PROVIDER_CONFIG[provider]?.color ?? DEFAULT_PROVIDER_COLOR
}

export function getModelProviderKey(model: Pick<ModelCatalogItem, 'id' | 'owned_by' | 'type'>): string {
  return resolveModelProvider(model)
}

export function getProviderDisplayName(provider: string): string {
  return PROVIDER_CONFIG[provider]?.label || provider.replace(/(^|-)([a-z])/g, (_, prefix: string, letter: string) => `${prefix}${letter.toUpperCase()}`)
}

export function getProviderOptions(models: ModelCatalogItem[]): ProviderOption[] {
  const counts = new Map<string, number>()
  models.forEach((model) => {
    const provider = getModelProviderKey(model)
    counts.set(provider, (counts.get(provider) || 0) + 1)
  })

  return Array.from(counts.entries())
    .map(([key, count]) => ({ key, label: getProviderDisplayName(key), count }))
    .sort((a, b) => a.label.localeCompare(b.label))
}

export function matchesModelSearch(model: ModelCatalogItem, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true

  return [
    model.id,
    model.display_name,
    model.owned_by,
  ].some((value) => value?.toLowerCase().includes(q))
}

const registryMetadataFields = [
  'object',
  'owned_by',
  'type',
  'name',
  'display_name',
  'description',
  'version',
  'created',
  'context_length',
  'max_completion_tokens',
  'inputTokenLimit',
  'outputTokenLimit',
  'thinking',
  'supported_parameters',
  'supportedGenerationMethods',
  'supportedInputModalities',
  'supportedOutputModalities',
] as const

function hasPresentValue(value: unknown): boolean {
  if (value === undefined || value === null) return false
  if (typeof value === 'string') return value.trim() !== ''
  if (Array.isArray(value)) return value.length > 0
  return true
}

export function mergeModelCatalogMetadata(model: ModelCatalogItem, metadata?: Partial<ModelCatalogItem>): ModelCatalogItem {
  if (!metadata) return model

  const merged: ModelCatalogItem = { ...model }
  registryMetadataFields.forEach((field) => {
    const next = metadata[field]
    if (hasPresentValue(next)) {
      merged[field] = next as never
    }
  })
  return merged
}

export function enrichModelCatalogItem(model: ModelCatalogItem, relatedModelIds: string[] = []): ModelCatalogItem {
  const ids = [model.id, ...relatedModelIds]
    .map((id) => id?.trim())
    .filter((id): id is string => !!id)
  const metadata = ids
    .map((id) => getModelRegistryMetadata(id) as Partial<ModelCatalogItem> | undefined)
    .find(Boolean)
  return mergeModelCatalogMetadata(model, metadata)
}

export function configuredModelToCatalogItem(
  model: ConfiguredModelInfo,
  { provider, preferAliasAsId = false }: { provider?: string; preferAliasAsId?: boolean } = {},
): ModelCatalogItem {
  const name = model.name?.trim() || ''
  const alias = model.alias?.trim() || ''
  const id = (preferAliasAsId && alias) ? alias : name
  const base: ModelCatalogItem = {
    id,
    name,
    owned_by: provider,
  }

  if (alias && alias !== id) {
    base.display_name = alias
  }
  if (model.description?.trim()) {
    base.description = model.description.trim()
  }

  return enrichModelCatalogItem(base, [name, alias])
}

export function modelDefinitionToCatalogItem(model: Partial<ModelCatalogItem> & { name?: string }): ModelCatalogItem {
  const id = model.id?.trim() || model.name?.trim() || ''
  return enrichModelCatalogItem({ ...model, id }, [model.name || ''])
}

export function formatTokenLimit(value?: number): string {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return ''
  return Math.round(value).toLocaleString('en-US')
}

export function formatThinking(model: ModelCatalogItem): string {
  const thinking = model.thinking
  if (!thinking) return ''

  const parts: string[] = []
  if (thinking.levels?.length) {
    parts.push(thinking.levels.join(' / '))
  }
  if (thinking.min || thinking.max) {
    const min = formatTokenLimit(thinking.min)
    const max = formatTokenLimit(thinking.max)
    parts.push([min, max].filter(Boolean).join(' - '))
  }
  if (thinking.zero_allowed) {
    parts.push('可关闭')
  }
  if (thinking.dynamic_allowed) {
    parts.push('动态')
  }

  return parts.filter(Boolean).join(' · ')
}

export function getModelDetailMetrics(model: ModelCatalogItem): ModelDetailMetric[] {
  const rows: ModelDetailMetric[] = []
  const contextLimit = model.context_length ?? model.inputTokenLimit
  const outputLimit = model.max_completion_tokens ?? model.outputTokenLimit
  const thinking = formatThinking(model)

  if (model.version) {
    rows.push({ label: '版本', value: model.version })
  }
  if (model.type) {
    rows.push({ label: '类型', value: model.type })
  }
  if (contextLimit) {
    rows.push({ label: '上下文', value: formatTokenLimit(contextLimit) })
  }
  if (outputLimit) {
    rows.push({ label: '最大输出', value: formatTokenLimit(outputLimit) })
  }
  if (thinking) {
    rows.push({ label: 'Thinking', value: thinking })
  }

  return rows
}

export function getSupportedMethods(model: ModelCatalogItem): string[] {
  if (model.supported_parameters?.length) return model.supported_parameters
  if (model.supportedGenerationMethods?.length) return model.supportedGenerationMethods

  const input = model.supportedInputModalities?.map((method) => `输入 ${method}`) || []
  const output = model.supportedOutputModalities?.map((method) => `输出 ${method}`) || []
  return [...input, ...output]
}

export function hasModelDetails(model: ModelCatalogItem): boolean {
  return Boolean(
    (model.display_name && model.display_name !== model.id) ||
    model.description ||
    model.version ||
    model.type ||
    model.context_length ||
    model.max_completion_tokens ||
    model.inputTokenLimit ||
    model.outputTokenLimit ||
    formatThinking(model) ||
    model.supportedInputModalities?.length ||
    model.supportedOutputModalities?.length ||
    getSupportedMethods(model).length,
  )
}
