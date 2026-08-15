export const ROUTING_STRATEGY_OPTIONS = [
  { label: '轮询法 (Round-Robin)', value: 'round-robin' },
  { label: '填满优先 (Fill-First)', value: 'fill-first' },
] as const

export type DisableImageGenerationMode = 'off' | 'all' | 'chat'

export interface SDKExtraConfigForm {
  maxRetryCredentials?: number
  redisUsageQueueRetentionSeconds?: number
  disableImageGeneration?: DisableImageGenerationMode
  routing?: {
    sessionAffinity?: boolean
    sessionAffinityTtl?: string
  }
}

export function buildSDKExtraConfigPatch(form: SDKExtraConfigForm) {
  const patch: SDKExtraConfigForm = {}

  if (typeof form.maxRetryCredentials === 'number') {
    patch.maxRetryCredentials = Math.max(0, Math.trunc(form.maxRetryCredentials))
  }
  if (typeof form.redisUsageQueueRetentionSeconds === 'number') {
    patch.redisUsageQueueRetentionSeconds = Math.min(3600, Math.max(1, Math.trunc(form.redisUsageQueueRetentionSeconds)))
  }
  if (form.disableImageGeneration) {
    patch.disableImageGeneration = form.disableImageGeneration
  }

  const routing: NonNullable<SDKExtraConfigForm['routing']> = {}
  if (typeof form.routing?.sessionAffinity === 'boolean') {
    routing.sessionAffinity = form.routing.sessionAffinity
  }
  if (typeof form.routing?.sessionAffinityTtl === 'string') {
    routing.sessionAffinityTtl = form.routing.sessionAffinityTtl.trim()
  }
  if (Object.keys(routing).length > 0) {
    patch.routing = routing
  }

  return patch
}

export function normalizeDisableImageGeneration(value: unknown): DisableImageGenerationMode {
  if (value === true) return 'all'
  if (value === false || value == null) return 'off'
  const str = String(value).trim().toLowerCase()
  if (str === 'chat') return 'chat'
  if (str === 'true' || str === 'all' || str === 'on' || str === '1') return 'all'
  return 'off'
}

export function configValue<T>(config: Record<string, unknown>, hyphenKey: string, fallback: T): T {
  if (hyphenKey in config) return config[hyphenKey] as T
  const camelKey = hyphenKey.replace(/-([a-z])/g, (_, ch: string) => ch.toUpperCase())
  if (camelKey in config) return config[camelKey] as T
  const snakeKey = hyphenKey.replaceAll('-', '_')
  if (snakeKey in config) return config[snakeKey] as T
  return fallback
}
