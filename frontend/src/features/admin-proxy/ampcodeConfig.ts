export interface AmpUpstreamAPIKeyEntry {
  'upstream-api-key': string
  'api-keys': string[]
}

export interface AmpModelMapping {
  from: string
  to: string
  regex?: boolean
}

export type AmpcodeConfigObject = Record<string, unknown>

export interface AmpUpstreamAPIKeyForm {
  upstreamApiKey: string
  apiKeysText: string
}

export function parseAmpUpstreamAPIKeyForm(form: AmpUpstreamAPIKeyForm): AmpUpstreamAPIKeyEntry {
  return {
    'upstream-api-key': form.upstreamApiKey.trim(),
    'api-keys': form.apiKeysText
      .split(/[\n,]+/)
      .map(item => item.trim())
      .filter(Boolean),
  }
}

export function buildAmpUpstreamAPIKeysPutPayload(entries: AmpUpstreamAPIKeyEntry[]) {
  return { value: normalizeAmpUpstreamAPIKeyEntries(entries) }
}

export function buildAmpUpstreamAPIKeysDeletePayload(upstreamKeys: string[]) {
  return {
    value: upstreamKeys.map(key => key.trim()).filter(Boolean),
  }
}

export function normalizeAmpUpstreamAPIKeyEntries(entries: AmpUpstreamAPIKeyEntry[]) {
  return entries
    .map(entry => ({
      'upstream-api-key': entry['upstream-api-key'].trim(),
      'api-keys': Array.from(new Set(entry['api-keys'].map(key => key.trim()).filter(Boolean))),
    }))
    .filter(entry => entry['upstream-api-key'])
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value)

export function extractAmpcodeConfig(value: unknown): AmpcodeConfigObject {
  if (!isRecord(value)) return {}
  return isRecord(value.ampcode) ? value.ampcode : value
}

export function normalizeAmpModelMappings(value: unknown): AmpModelMapping[] {
  if (!Array.isArray(value)) return []

  return value.map((entry) => {
    if (!isRecord(entry)) {
      return { from: '', to: '', regex: false }
    }
    return {
      from: typeof entry.from === 'string' ? entry.from.trim() : '',
      to: typeof entry.to === 'string' ? entry.to.trim() : '',
      regex: entry.regex === true,
    }
  })
}

export function validateAmpModelMappings(entries: AmpModelMapping[]): string[] {
  const errors: string[] = []
  const seen = new Set<string>()

  entries.forEach((entry, index) => {
    const row = index + 1
    const from = entry.from.trim()
    const to = entry.to.trim()

    if (!from && !to) return
    if (!from) errors.push(`第 ${row} 行缺少 from 模型`)
    if (!to) errors.push(`第 ${row} 行缺少 to 模型`)

    const key = from.toLowerCase()
    if (from && seen.has(key)) {
      errors.push(`第 ${row} 行 from 模型重复: ${from}`)
    }
    if (from) seen.add(key)
  })

  return errors
}

export function buildAmpModelMappingsPutPayload(entries: AmpModelMapping[]) {
  const normalized = normalizeAmpModelMappings(entries)
  const errors = validateAmpModelMappings(normalized)
  if (errors.length > 0) {
    throw new Error(errors[0])
  }

  return {
    value: normalized
      .filter(entry => entry.from && entry.to)
      .map(entry => ({
        from: entry.from,
        to: entry.to,
        ...(entry.regex ? { regex: true } : {}),
      })),
  }
}

export function isRegexValid(pattern: string): { valid: boolean; error?: string } {
  try {
    new RegExp(pattern)
    return { valid: true }
  } catch (err) {
    return {
      valid: false,
      error: err instanceof Error ? err.message : String(err),
    }
  }
}

export function simulateModelMapping(
  modelName: string,
  mappings: AmpModelMapping[],
): {
  matched: boolean
  targetModel: string
  ruleIndex: number
  matchedRule?: AmpModelMapping
} {
  const normalizedModel = (modelName ?? '').trim()
  if (!normalizedModel || !Array.isArray(mappings)) {
    return { matched: false, targetModel: modelName, ruleIndex: -1 }
  }

  for (let i = 0; i < mappings.length; i++) {
    const rule = mappings[i]
    if (!rule || !rule.from) continue

    if (rule.regex) {
      try {
        const re = new RegExp(rule.from)
        if (re.test(modelName)) {
          const targetModel = modelName.replace(re, rule.to)
          return {
            matched: true,
            targetModel,
            ruleIndex: i,
            matchedRule: rule,
          }
        }
      } catch {
        // treat as non-match
      }
    } else {
      if (rule.from.trim().toLowerCase() === normalizedModel.toLowerCase()) {
        return {
          matched: true,
          targetModel: rule.to,
          ruleIndex: i,
          matchedRule: rule,
        }
      }
    }
  }

  return { matched: false, targetModel: modelName, ruleIndex: -1 }
}

export function maskApiKey(key: string): string {
  if (!key) return ''
  if (key.length <= 8) return '••••••••'
  return `${key.slice(0, 6)}••••••••${key.slice(-4)}`
}

export function validateUpstreamUrl(url: string): {
  valid: boolean
  message?: string
  suggestedUrl?: string
} {
  const trimmed = (url ?? '').trim()
  if (!trimmed) {
    return {
      valid: false,
      message: '请输入上游地址',
    }
  }

  if (!/^https?:\/\//i.test(trimmed)) {
    const suggestedUrl = trimmed.includes('://')
      ? `https://${trimmed.replace(/^[a-zA-Z]+:\/\//, '')}`
      : `https://${trimmed}`

    return {
      valid: false,
      message: '上游地址必须以 http:// 或 https:// 开头',
      suggestedUrl,
    }
  }

  try {
    new URL(trimmed)
    return { valid: true }
  } catch {
    return {
      valid: false,
      message: '上游地址格式不正确',
    }
  }
}

