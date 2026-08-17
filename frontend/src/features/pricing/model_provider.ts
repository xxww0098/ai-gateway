export interface ProviderAwareModel {
  id?: string
  owned_by?: string
  type?: string
  [key: string]: unknown
}

type ModelRegistry = Record<string, ProviderAwareModel[]>

const PROVIDER_ALIAS_MAP: Record<string, string> = {
  claude: 'anthropic',
  anthropic: 'anthropic',
  openai: 'openai',
  gemini: 'google',
  google: 'google',
  kimi: 'kimi',
  moonshot: 'kimi',
  minimax: 'minimax',
  deepseek: 'deepseek',
  glm: 'glm',
  zai: 'glm',
  zhipu: 'glm',
  bigmodel: 'glm',
  mimo: 'mimo',
  xiaomi: 'mimo',
  // 百炼是平台，qwen 是模型家族：owned_by=qwen 仍归 qwen，不并进 bailian。
  bailian: 'bailian',
  dashscope: 'bailian',
  llama: 'meta',
  meta: 'meta',
}

function normalizeProvider(value?: string): string {
  const lower = value?.trim().toLowerCase() || ''
  if (!lower) return ''
  return PROVIDER_ALIAS_MAP[lower] || lower
}

// Lazy-initialized maps — populated on first access via async registry loading
let _providerByModelId: Map<string, string> | null = null
let _registryByModelId: Map<string, ProviderAwareModel> | null = null
let _registryLoadPromise: Promise<void> | null = null

function buildMapsFromRegistries(registries: ModelRegistry[]): void {
  const providerByModelId = new Map<string, string>()
  const registryByModelId = new Map<string, ProviderAwareModel>()

  registries.forEach((registry) => {
    Object.values(registry).forEach((models) => {
      if (!Array.isArray(models)) return
      models.forEach((model) => {
        const modelId = model.id?.trim().toLowerCase()
        if (!modelId) return
        const provider = normalizeProvider(model.owned_by) || normalizeProvider(model.type)
        registryByModelId.set(modelId, model)
        if (provider) {
          providerByModelId.set(modelId, provider)
        }
      })
    })
  })

  _providerByModelId = providerByModelId
  _registryByModelId = registryByModelId
}

/**
 * Loads model registry JSON files asynchronously via dynamic import().
 * Returns a promise that resolves once the maps are populated.
 * Subsequent calls return the same promise (singleton).
 */
export function loadModelRegistries(): Promise<void> {
  if (_providerByModelId && _registryByModelId) {
    return Promise.resolve()
  }
  if (_registryLoadPromise) {
    return _registryLoadPromise
  }
  _registryLoadPromise = Promise.all([
    import('@/data/gateway-models-registry.json').then((m) => m.default as unknown as ModelRegistry),
    import('@/data/agw-models-registry.json').then((m) => m.default as unknown as ModelRegistry),
  ]).then(([modelRegistry, customModelRegistry]) => {
    buildMapsFromRegistries([modelRegistry, customModelRegistry])
  })
  return _registryLoadPromise
}

function ensureMaps(): { providerByModelId: Map<string, string>; registryByModelId: Map<string, ProviderAwareModel> } {
  if (_providerByModelId && _registryByModelId) {
    return { providerByModelId: _providerByModelId, registryByModelId: _registryByModelId }
  }
  // Fallback: return empty maps if registries haven't loaded yet.
  // Callers should await loadModelRegistries() before using these functions for full data.
  return { providerByModelId: new Map(), registryByModelId: new Map() }
}

export function getModelRegistryMetadata(modelId?: string): ProviderAwareModel | undefined {
  const key = modelId?.trim().toLowerCase() || ''
  if (!key) return undefined
  return ensureMaps().registryByModelId.get(key)
}

export function resolveModelProvider(model: ProviderAwareModel): string {
  return normalizeProvider(model.owned_by) ||
    normalizeProvider(model.type) ||
    ensureMaps().providerByModelId.get(model.id?.trim().toLowerCase() || '') ||
    'other'
}

/**
 * Returns the resolved provider key for a given model id string.
 * Useful for callers that only have the model id, not a full ProviderAwareModel object.
 */
export function getProviderForModelId(modelId: string): string {
  const key = modelId?.trim().toLowerCase() || ''
  if (!key) return 'other'
  return ensureMaps().providerByModelId.get(key) || 'other'
}
