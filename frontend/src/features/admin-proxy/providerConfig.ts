// Admin proxy channel kinds differ from model catalog provider keys (openai/claude/gemini/codex/vertex
// vs anthropic/openai/google/etc.). Shared provider display config lives in
// @/features/pricing/model_catalog.ts (PROVIDER_CONFIG). Admin labels below are intentionally
// distinct because they describe channel integration types, not model providers.
//
// Single source of truth for admin channel metadata: response key, endpoint, label, usage hints.
// RESPONSE_KEYS, PROVIDER_ENDPOINTS, providerLabel(), and providerUsageHints() all derive from this.

export type ProviderKind = 'openai' | 'claude' | 'gemini' | 'codex' | 'vertex'

export interface ApiKeyUsageEntry {
  success: number
  failed: number
  recent_requests?: Array<{ success?: number; failed?: number; start?: string; end?: string }>
}

export type ApiKeyUsageResponse = Record<string, Record<string, ApiKeyUsageEntry>>

export interface BaseChannelItem {
  _id: string
  providerKind: ProviderKind
  index: number
  keyIndex?: number
  name?: string
  apiKey: string
  apiKeyProxyUrl?: string
  baseUrl?: string
  modelsUrl?: string
  proxyUrl?: string
  priority?: number
  prefix?: string
  disabled?: boolean
  websockets?: boolean
  experimentalCCHSigning?: boolean
  headers?: Record<string, string>
  models?: unknown[]
  excludedModels?: string[]
  originalPayload: Record<string, unknown>
  keyPayload?: Record<string, unknown>
  usage?: ApiKeyUsageEntry
}

export interface ProviderStructuredForm {
  name?: string
  apiKey?: string
  baseUrl?: string
  modelsUrl?: string
  proxyUrl?: string
  apiKeyProxyUrl?: string
  priority?: string
  prefix?: string
  headersText?: string
  modelsText?: string
  excludedModelsText?: string
  disabled?: boolean
  websockets?: boolean
  experimentalCCHSigning?: boolean
  advancedText?: string
}

export interface ConfigurableModelLike {
  name: string
  alias?: string
}

interface AdminProviderMeta {
  responseKey: string
  endpoint: string
  label: string
  usageHints: string[]
}

const ADMIN_PROVIDER_META: Record<ProviderKind, AdminProviderMeta> = {
  openai: {
    responseKey: 'openai-compatibility',
    endpoint: '/openai-compatibility',
    label: 'OpenAI (兼容格式)',
    usageHints: ['openai', 'openai-compatibility', 'openai_compatibility'],
  },
  claude: {
    responseKey: 'claude-api-key',
    endpoint: '/claude-api-key',
    label: 'Claude',
    usageHints: ['claude', 'anthropic'],
  },
  gemini: {
    responseKey: 'gemini-api-key',
    endpoint: '/gemini-api-key',
    label: 'Gemini',
    usageHints: ['gemini'],
  },
  codex: {
    responseKey: 'codex-api-key',
    endpoint: '/codex-api-key',
    label: 'Codex GitHub Copilot',
    usageHints: ['codex'],
  },
  vertex: {
    responseKey: 'vertex-api-key',
    endpoint: '/vertex-api-key',
    label: 'GCP Vertex AI',
    usageHints: ['vertex', 'vertex-ai', 'vertex_api'],
  },
}

export const PROVIDER_ENDPOINTS: Record<ProviderKind, string> = Object.fromEntries(
  (Object.entries(ADMIN_PROVIDER_META) as [ProviderKind, AdminProviderMeta][]).map(
    ([kind, meta]) => [kind, meta.endpoint],
  ),
) as Record<ProviderKind, string>

export function providerResponseKey(provider: ProviderKind) {
  return ADMIN_PROVIDER_META[provider].responseKey
}

export function providerLabel(provider: ProviderKind) {
  return ADMIN_PROVIDER_META[provider].label
}

export function extractProviderArray(provider: ProviderKind, data: unknown): Record<string, unknown>[] {
  if (Array.isArray(data)) return data.map(toRecord).filter(Boolean) as Record<string, unknown>[]
  if (!isRecord(data)) return []
  const direct = data[providerResponseKey(provider)]
  if (Array.isArray(direct)) return direct.map(toRecord).filter(Boolean) as Record<string, unknown>[]
  const keys = data.keys
  if (Array.isArray(keys)) return keys.map(toRecord).filter(Boolean) as Record<string, unknown>[]
  const firstArray = Object.values(data).find(Array.isArray)
  return Array.isArray(firstArray) ? firstArray.map(toRecord).filter(Boolean) as Record<string, unknown>[] : []
}

export function normalizeProviderItems(provider: ProviderKind, data: unknown, usage?: ApiKeyUsageResponse): BaseChannelItem[] {
  const rawArray = extractProviderArray(provider, data)
  const out: BaseChannelItem[] = []

  rawArray.forEach((item, index) => {
    if (provider === 'openai') {
      const entries = arrayOfRecords(item['api-key-entries'])
      if (entries.length === 0) {
        out.push(buildOpenAIItem(provider, item, index, undefined, undefined, usage))
        return
      }
      entries.forEach((entry, keyIndex) => {
        out.push(buildOpenAIItem(provider, item, index, entry, keyIndex, usage))
      })
      return
    }

    const apiKey = stringValue(item['api-key'])
    out.push({
      _id: `${provider}-${index}`,
      providerKind: provider,
      index,
      apiKey,
      baseUrl: stringValue(item['base-url']),
      proxyUrl: stringValue(item['proxy-url']),
      priority: numberValue(item.priority),
      prefix: stringValue(item.prefix),
      websockets: provider === 'codex' ? booleanValue(item.websockets) : undefined,
      experimentalCCHSigning: provider === 'claude' ? booleanValue(item['experimental-cch-signing']) : undefined,
      headers: recordOfStrings(item.headers),
      models: Array.isArray(item.models) ? item.models : undefined,
      excludedModels: arrayOfStrings(item['excluded-models']),
      originalPayload: item,
      usage: findApiKeyUsage(usage, provider, stringValue(item['base-url']), apiKey),
    })
  })

  return out
}

export function buildProviderAddArray(provider: ProviderKind, data: unknown, form: ProviderStructuredForm): Record<string, unknown>[] {
  const rawArray = cloneArray(extractProviderArray(provider, data))
  const patch = structuredPatch(provider, form, { forAdd: true })
  const advanced = parseObjectText(form.advancedText, '高级 JSON')

  if (provider === 'openai') {
    const name = stringValue(patch.name || form.name)
    const baseUrl = stringValue(patch['base-url'] || form.baseUrl)
    const apiKey = stringValue(form.apiKey)
    if (!name) throw new Error('OpenAI 兼容渠道名称不能为空')
    if (!baseUrl) throw new Error('OpenAI 兼容 Base URL 不能为空')
    if (!apiKey) throw new Error('API Key 不能为空')

    const keyEntry: Record<string, unknown> = { 'api-key': apiKey }
    if (form.apiKeyProxyUrl?.trim()) keyEntry['proxy-url'] = form.apiKeyProxyUrl.trim()

    const groupIndex = rawArray.findIndex(item => stringValue(item.name) === name || stringValue(item['base-url']) === baseUrl)
    if (groupIndex >= 0) {
      const current = rawArray[groupIndex]
      const entries = arrayOfRecords(current['api-key-entries'])
      rawArray[groupIndex] = {
        ...current,
        ...advanced,
        ...patch,
        name,
        'base-url': baseUrl,
        'api-key-entries': [...entries, keyEntry],
      }
    } else {
      rawArray.push({
        ...advanced,
        ...patch,
        name,
        'base-url': baseUrl,
        'api-key-entries': [keyEntry],
      })
    }
    return rawArray
  }

  const apiKey = stringValue(form.apiKey)
  if (!apiKey) throw new Error('API Key 不能为空')
  rawArray.push({
    ...advanced,
    ...patch,
    'api-key': apiKey,
  })
  return rawArray
}

export function buildProviderDeleteArray(provider: ProviderKind, data: unknown, item: BaseChannelItem): Record<string, unknown>[] {
  const rawArray = cloneArray(extractProviderArray(provider, data))
  if (provider === 'openai') {
    const group = rawArray[item.index]
    if (!group) return rawArray
    const entries = arrayOfRecords(group['api-key-entries'])
    if (item.keyIndex == null) {
      rawArray.splice(item.index, 1)
      return rawArray
    }
    const nextEntries = entries.filter((_, index) => index !== item.keyIndex)
    if (nextEntries.length === 0) {
      rawArray.splice(item.index, 1)
    } else {
      rawArray[item.index] = { ...group, 'api-key-entries': nextEntries }
    }
    return rawArray
  }
  rawArray.splice(item.index, 1)
  return rawArray
}

export function buildProviderEditArray(provider: ProviderKind, data: unknown, item: BaseChannelItem, form: ProviderStructuredForm): Record<string, unknown>[] {
  const rawArray = cloneArray(extractProviderArray(provider, data))
  const patch = structuredPatch(provider, form, { forAdd: false })
  const advanced = parseObjectText(form.advancedText, '高级 JSON')

  if (provider === 'openai') {
    const group = rawArray[item.index]
    if (!group) throw new Error('未找到要更新的渠道')
    const nextGroup = { ...group, ...advanced, ...patch }
    const entries = arrayOfRecords(nextGroup['api-key-entries'])
    if (item.keyIndex != null && entries[item.keyIndex]) {
      entries[item.keyIndex] = {
        ...entries[item.keyIndex],
        'api-key': stringValue(form.apiKey || entries[item.keyIndex]['api-key']),
        ...optionalStringField('proxy-url', form.apiKeyProxyUrl),
      }
      nextGroup['api-key-entries'] = entries
    }
    rawArray[item.index] = nextGroup
    return rawArray
  }

  const current = rawArray[item.index]
  if (!current) throw new Error('未找到要更新的凭证')
  rawArray[item.index] = {
    ...current,
    ...advanced,
    ...patch,
    ...optionalStringField('api-key', form.apiKey),
  }
  return rawArray
}

export function buildProviderModelsArray(provider: ProviderKind, data: unknown, item: BaseChannelItem, models: ConfigurableModelLike[]): Record<string, unknown>[] {
  const rawArray = cloneArray(extractProviderArray(provider, data))
  if (item.index < 0 || item.index >= rawArray.length) {
    throw new Error('未找到要更新的渠道')
  }

  const normalizedModels = models
    .map((model) => {
      const name = String(model.name || '').trim()
      if (!name) return null
      const entry: Record<string, unknown> = { name }
      const alias = String(model.alias || '').trim()
      if (alias && alias !== name) {
        entry.alias = alias
      }
      return entry
    })
    .filter(Boolean) as Record<string, unknown>[]

  rawArray[item.index] = {
    ...rawArray[item.index],
    models: normalizedModels,
  }

  return rawArray
}

export function findApiKeyUsage(usage: ApiKeyUsageResponse | undefined, provider: ProviderKind, baseUrl: string, apiKey: string): ApiKeyUsageEntry | undefined {
  if (!usage || !apiKey) return undefined
  const composite = `${baseUrl || ''}|${apiKey}`
  const providerHints = providerUsageHints(provider)
  const buckets = [
    ...providerHints.map(hint => usage[hint]).filter(Boolean),
    ...Object.entries(usage)
      .filter(([name]) => !providerHints.includes(name))
      .map(([, bucket]) => bucket),
  ]

  for (const bucket of buckets) {
    if (!bucket) continue
    if (bucket[composite]) return bucket[composite]
    if (bucket[`|${apiKey}`]) return bucket[`|${apiKey}`]
    const loose = Object.entries(bucket).find(([key]) => key.endsWith(`|${apiKey}`))
    if (loose) return loose[1]
  }
  return undefined
}

function buildOpenAIItem(provider: ProviderKind, group: Record<string, unknown>, index: number, keyEntry: Record<string, unknown> | undefined, keyIndex: number | undefined, usage?: ApiKeyUsageResponse): BaseChannelItem {
  const apiKey = stringValue(keyEntry?.['api-key'])
  return {
    _id: `openai-${index}-${keyIndex ?? 'empty'}`,
    providerKind: provider,
    index,
    keyIndex,
    name: stringValue(group.name) || `Channel-${index + 1}`,
    apiKey,
    apiKeyProxyUrl: stringValue(keyEntry?.['proxy-url']),
    baseUrl: stringValue(group['base-url']),
    modelsUrl: stringValue(group['models-url'] || group['model-url'] || group.models_url),
    priority: numberValue(group.priority),
    prefix: stringValue(group.prefix),
    disabled: booleanValue(group.disabled),
    headers: recordOfStrings(group.headers),
    models: Array.isArray(group.models) ? group.models : undefined,
    originalPayload: group,
    keyPayload: keyEntry,
    usage: findApiKeyUsage(usage, provider, stringValue(group['base-url']), apiKey),
  }
}

function structuredPatch(provider: ProviderKind, form: ProviderStructuredForm, opts: { forAdd: boolean }) {
  const out: Record<string, unknown> = {}
  if (provider === 'openai') {
    assignString(out, 'name', form.name)
    assignString(out, 'base-url', form.baseUrl)
    assignString(out, 'models-url', form.modelsUrl)
    assignString(out, 'prefix', form.prefix)
    assignNumber(out, 'priority', form.priority)
    if (typeof form.disabled === 'boolean') out.disabled = form.disabled
  } else {
    assignString(out, 'base-url', form.baseUrl)
    assignString(out, 'proxy-url', form.proxyUrl)
    assignString(out, 'prefix', form.prefix)
    assignNumber(out, 'priority', form.priority)
  }
  if (provider === 'codex' && typeof form.websockets === 'boolean') out.websockets = form.websockets
  if (provider === 'claude' && typeof form.experimentalCCHSigning === 'boolean') out['experimental-cch-signing'] = form.experimentalCCHSigning

  const headers = parseObjectText(form.headersText, 'Headers JSON')
  if (headers) out.headers = headers
  const models = parseArrayText(form.modelsText, 'Models JSON')
  if (models) out.models = models
  if (provider !== 'openai') {
    const excluded = parseStringList(form.excludedModelsText)
    if (excluded || (!opts.forAdd && form.excludedModelsText != null)) out['excluded-models'] = excluded || []
  }
  return out
}

function providerUsageHints(provider: ProviderKind) {
  return ADMIN_PROVIDER_META[provider].usageHints
}

function parseObjectText(text: string | undefined, label: string): Record<string, unknown> | undefined {
  const trimmed = text?.trim()
  if (!trimmed) return undefined
  const parsed = JSON.parse(trimmed) as unknown
  if (!isRecord(parsed) || Array.isArray(parsed)) throw new Error(`${label} 必须是 JSON 对象`)
  return parsed
}

function parseArrayText(text: string | undefined, label: string): unknown[] | undefined {
  const trimmed = text?.trim()
  if (!trimmed) return undefined
  const parsed = JSON.parse(trimmed) as unknown
  if (!Array.isArray(parsed)) throw new Error(`${label} 必须是 JSON 数组`)
  return parsed
}

function parseStringList(text: string | undefined): string[] | undefined {
  if (text == null) return undefined
  return text.split(/[\n,]+/).map(item => item.trim()).filter(Boolean)
}

function assignString(out: Record<string, unknown>, key: string, value: string | undefined) {
  if (value == null) return
  const trimmed = value.trim()
  if (trimmed || key === 'base-url' || key === 'models-url' || key === 'proxy-url' || key === 'prefix') out[key] = trimmed
}

function assignNumber(out: Record<string, unknown>, key: string, value: string | undefined) {
  if (value == null || value.trim() === '') return
  const n = Number(value)
  if (!Number.isFinite(n)) throw new Error(`${key} 必须是数字`)
  out[key] = Math.trunc(n)
}

function optionalStringField(key: string, value: string | undefined) {
  if (value == null) return {}
  return { [key]: value.trim() }
}

function cloneArray(arr: Record<string, unknown>[]) {
  return arr.map(item => JSON.parse(JSON.stringify(item)) as Record<string, unknown>)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function toRecord(value: unknown) {
  return isRecord(value) ? value : null
}

function arrayOfRecords(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.map(toRecord).filter(Boolean) as Record<string, unknown>[] : []
}

function arrayOfStrings(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) return undefined
  return value.map(item => String(item).trim()).filter(Boolean)
}

function recordOfStrings(value: unknown): Record<string, string> | undefined {
  if (!isRecord(value)) return undefined
  const out: Record<string, string> = {}
  Object.entries(value).forEach(([key, item]) => {
    if (typeof item === 'string') out[key] = item
  })
  return Object.keys(out).length > 0 ? out : undefined
}

function stringValue(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function numberValue(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function booleanValue(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined
}
