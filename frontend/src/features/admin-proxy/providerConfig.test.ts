import { describe, expect, it } from 'vitest'
import {
  type ProviderKind,
  type BaseChannelItem,
  type ConfigurableModelLike,
  PROVIDER_ENDPOINTS,
  providerResponseKey,
  providerLabel,
  extractProviderArray,
  buildProviderAddArray,
  buildProviderEditArray,
  buildProviderModelsArray,
  findApiKeyUsage,
  normalizeProviderItems,
  type ApiKeyUsageResponse,
} from './providerConfig'

describe('providerConfig', () => {
  it('preserves advanced fields when editing a credential', () => {
    const data = {
      'claude-api-key': [{
        'api-key': 'sk-old',
        'base-url': 'https://old.example.com',
        'proxy-url': 'direct',
        cloak: { mode: 'auto' },
      }],
    }

    const items = normalizeProviderItems('claude', data)
    const updated = buildProviderEditArray('claude', data, items[0], {
      apiKey: 'sk-new',
      baseUrl: 'https://new.example.com',
      advancedText: JSON.stringify(items[0].originalPayload),
    })

    expect(updated[0]).toMatchObject({
      'api-key': 'sk-new',
      'base-url': 'https://new.example.com',
      'proxy-url': 'direct',
      cloak: { mode: 'auto' },
    })
  })

  it('adds OpenAI compatibility keys into existing groups with per-key proxy', () => {
    const data = {
      'openai-compatibility': [{
        name: 'openrouter',
        'base-url': 'https://openrouter.ai/api/v1',
        'api-key-entries': [{ 'api-key': 'sk-a' }],
        models: [{ name: 'm1', alias: 'm1' }],
      }],
    }

    const updated = buildProviderAddArray('openai', data, {
      name: 'openrouter',
      baseUrl: 'https://openrouter.ai/api/v1',
      apiKey: 'sk-b',
      apiKeyProxyUrl: 'direct',
    })

    expect(updated).toHaveLength(1)
    expect(updated[0]['api-key-entries']).toEqual([
      { 'api-key': 'sk-a' },
      { 'api-key': 'sk-b', 'proxy-url': 'direct' },
    ])
    expect(updated[0].models).toEqual([{ name: 'm1', alias: 'm1' }])
  })

  it('matches api-key-usage by base-url and api-key with fallback provider buckets', () => {
    const usage = {
      unknown: {
        'https://api.example.com|sk-test': {
          success: 3,
          failed: 1,
        },
      },
    }

    expect(findApiKeyUsage(usage, 'gemini', 'https://api.example.com', 'sk-test')).toEqual({
      success: 3,
      failed: 1,
    })
  })

  it('writes discovered models back to the OpenAI compatibility group', () => {
    const data = {
      'openai-compatibility': [{
        name: 'MINIMAX',
        'base-url': 'https://api.minimaxi.com/v1',
        'api-key-entries': [{ 'api-key': 'sk-a' }],
        models: [],
      }],
    }

    const items = normalizeProviderItems('openai', data)
    const updated = buildProviderModelsArray('openai', data, items[0], [
      { name: 'MiniMax-Text-01' },
      { name: 'abab6.5-chat', alias: 'abab6.5-chat' },
      { name: 'speech-02-hd', alias: 'Speech HD' },
    ])

    expect(updated[0]).toMatchObject({
      name: 'MINIMAX',
      'base-url': 'https://api.minimaxi.com/v1',
      'api-key-entries': [{ 'api-key': 'sk-a' }],
      models: [
        { name: 'MiniMax-Text-01' },
        { name: 'abab6.5-chat' },
        { name: 'speech-02-hd', alias: 'Speech HD' },
      ],
    })
  })
})


const ALL_PROVIDER_KINDS: ProviderKind[] = ['openai', 'claude', 'gemini', 'codex', 'vertex']

// ─── Provider labels (admin channel labels, NOT model catalog labels) ───

describe('providerLabel', () => {
  it.each(ALL_PROVIDER_KINDS)('returns correct admin label for %s', (kind) => {
    const expected: Record<ProviderKind, string> = {
      openai: 'OpenAI (兼容格式)',
      claude: 'Claude',
      gemini: 'Gemini',
      codex: 'Codex GitHub Copilot',
      vertex: 'GCP Vertex AI',
    }
    expect(providerLabel(kind)).toBe(expected[kind])
  })
})

// ─── Provider endpoints ───

describe('PROVIDER_ENDPOINTS', () => {
  it('has an entry for every ProviderKind', () => {
    for (const kind of ALL_PROVIDER_KINDS) {
      expect(PROVIDER_ENDPOINTS[kind]).toBeDefined()
      expect(typeof PROVIDER_ENDPOINTS[kind]).toBe('string')
    }
  })

  it('maps each kind to its expected endpoint path', () => {
    expect(PROVIDER_ENDPOINTS.openai).toBe('/openai-compatibility')
    expect(PROVIDER_ENDPOINTS.claude).toBe('/claude-api-key')
    expect(PROVIDER_ENDPOINTS.gemini).toBe('/gemini-api-key')
    expect(PROVIDER_ENDPOINTS.codex).toBe('/codex-api-key')
    expect(PROVIDER_ENDPOINTS.vertex).toBe('/vertex-api-key')
  })
})

// ─── Response keys ───

describe('providerResponseKey', () => {
  it('returns the expected response key for each provider', () => {
    const expected: Record<ProviderKind, string> = {
      openai: 'openai-compatibility',
      claude: 'claude-api-key',
      gemini: 'gemini-api-key',
      codex: 'codex-api-key',
      vertex: 'vertex-api-key',
    }
    for (const kind of ALL_PROVIDER_KINDS) {
      expect(providerResponseKey(kind)).toBe(expected[kind])
    }
  })

  it('response key matches endpoint without leading slash', () => {
    for (const kind of ALL_PROVIDER_KINDS) {
      expect(providerResponseKey(kind)).toBe(PROVIDER_ENDPOINTS[kind].slice(1))
    }
  })
})

// ─── extractProviderArray ───

describe('extractProviderArray', () => {
  it('returns data directly if it is an array', () => {
    const data = [{ 'api-key': 'k1' }, { 'api-key': 'k2' }]
    expect(extractProviderArray('claude', data)).toEqual(data)
  })

  it('returns empty array for non-record, non-array input', () => {
    expect(extractProviderArray('claude', null)).toEqual([])
    expect(extractProviderArray('claude', 'string')).toEqual([])
    expect(extractProviderArray('claude', 42)).toEqual([])
  })

  it('extracts from response key when data is a record', () => {
    const items = [{ 'api-key': 'a' }]
    const data = { 'claude-api-key': items }
    expect(extractProviderArray('claude', data)).toEqual(items)
  })

  it('falls back to data.keys array', () => {
    const items = [{ 'api-key': 'a' }]
    const data = { keys: items, other: 'stuff' }
    expect(extractProviderArray('claude', data)).toEqual(items)
  })

  it('falls back to first array value in record', () => {
    const items = [{ 'api-key': 'a' }]
    const data = { notArray: 'skip', theArray: items }
    expect(extractProviderArray('claude', data)).toEqual(items)
  })

  it('filters out non-record entries from arrays', () => {
    const data = [{ 'api-key': 'a' }, null, 42, { 'api-key': 'b' }]
    const result = extractProviderArray('claude', data)
    expect(result).toHaveLength(2)
    expect(result[0]).toEqual({ 'api-key': 'a' })
    expect(result[1]).toEqual({ 'api-key': 'b' })
  })
})

// ─── normalizeProviderItems ───

describe('normalizeProviderItems', () => {
  it('normalizes non-openai provider items', () => {
    const data = [
      { 'api-key': 'sk-1', 'base-url': 'https://api.anthropic.com', priority: 1, prefix: 'p' },
      { 'api-key': 'sk-2', 'proxy-url': 'https://proxy.example.com' },
    ]
    const items = normalizeProviderItems('claude', data)
    expect(items).toHaveLength(2)
    expect(items[0].providerKind).toBe('claude')
    expect(items[0]._id).toBe('claude-0')
    expect(items[0].apiKey).toBe('sk-1')
    expect(items[0].baseUrl).toBe('https://api.anthropic.com')
    expect(items[0].priority).toBe(1)
    expect(items[0].prefix).toBe('p')
    expect(items[1]._id).toBe('claude-1')
    expect(items[1].proxyUrl).toBe('https://proxy.example.com')
  })

  it('sets websockets only for codex provider', () => {
    const codexData = [{ 'api-key': 'k', websockets: true }]
    const codexItems = normalizeProviderItems('codex', codexData)
    expect(codexItems[0].websockets).toBe(true)

    const claudeData = [{ 'api-key': 'k', websockets: true }]
    const claudeItems = normalizeProviderItems('claude', claudeData)
    expect(claudeItems[0].websockets).toBeUndefined()
  })

  it('sets experimentalCCHSigning only for claude provider', () => {
    const claudeData = [{ 'api-key': 'k', 'experimental-cch-signing': true }]
    const claudeItems = normalizeProviderItems('claude', claudeData)
    expect(claudeItems[0].experimentalCCHSigning).toBe(true)

    const geminiData = [{ 'api-key': 'k', 'experimental-cch-signing': true }]
    const geminiItems = normalizeProviderItems('gemini', geminiData)
    expect(geminiItems[0].experimentalCCHSigning).toBeUndefined()
  })

  it('normalizes openai provider items with api-key-entries', () => {
    const data = [
      {
        name: 'openrouter',
        'base-url': 'https://openrouter.ai',
        'models-url': 'https://openrouter.ai/api/v1/models',
        'api-key-entries': [
          { 'api-key': 'key-1' },
          { 'api-key': 'key-2', 'proxy-url': 'https://proxy' },
        ],
      },
    ]
    const items = normalizeProviderItems('openai', data)
    expect(items).toHaveLength(2)
    expect(items[0].providerKind).toBe('openai')
    expect(items[0].name).toBe('openrouter')
    expect(items[0].apiKey).toBe('key-1')
    expect(items[0].keyIndex).toBe(0)
    expect(items[0].baseUrl).toBe('https://openrouter.ai')
    expect(items[0].modelsUrl).toBe('https://openrouter.ai/api/v1/models')
    expect(items[1].apiKey).toBe('key-2')
    expect(items[1].keyIndex).toBe(1)
    expect(items[1].apiKeyProxyUrl).toBe('https://proxy')
  })

  it('saves OpenAI custom model list URL on provider group edits', () => {
    const data = [{ name: 'volcano', 'base-url': 'https://ark.cn-beijing.volces.com/api/coding/v3', 'api-key-entries': [{ 'api-key': 'k' }] }]
    const items = normalizeProviderItems('openai', data)
    const updated = buildProviderEditArray('openai', data, items[0], {
      name: 'volcano',
      baseUrl: 'https://ark.cn-beijing.volces.com/api/coding/v3',
      modelsUrl: 'https://ark.cn-beijing.volces.com/api/v3/models',
      advancedText: JSON.stringify(items[0].originalPayload),
    })

    expect(updated[0]['models-url']).toBe('https://ark.cn-beijing.volces.com/api/v3/models')
    expect(normalizeProviderItems('openai', updated)[0].modelsUrl).toBe('https://ark.cn-beijing.volces.com/api/v3/models')
  })

  it('handles openai group with empty api-key-entries', () => {
    const data = [{ name: 'test', 'base-url': 'https://test.com', 'api-key-entries': [] }]
    const items = normalizeProviderItems('openai', data)
    expect(items).toHaveLength(1)
    expect(items[0].keyIndex).toBeUndefined()
    expect(items[0].name).toBe('test')
  })

  it('handles openai group with no api-key-entries field', () => {
    const data = [{ name: 'test', 'base-url': 'https://test.com' }]
    const items = normalizeProviderItems('openai', data)
    expect(items).toHaveLength(1)
    expect(items[0].keyIndex).toBeUndefined()
  })

  it('assigns default Channel-N name for openai items without name', () => {
    const data = [{ 'base-url': 'https://test.com', 'api-key-entries': [{ 'api-key': 'k' }] }]
    const items = normalizeProviderItems('openai', data)
    expect(items[0].name).toBe('Channel-1')
  })
})

// ─── buildProviderModelsArray ───

describe('buildProviderModelsArray', () => {
  const makeData = (models?: unknown[]) => [{ 'api-key': 'k', models }]
  const makeItem = (index: number): BaseChannelItem => ({
    _id: 'test-0',
    providerKind: 'claude',
    index,
    apiKey: 'k',
    originalPayload: {},
  })

  it('writes normalized models to the target item', () => {
    const models: ConfigurableModelLike[] = [
      { name: 'claude-3-opus' },
      { name: 'claude-3-sonnet', alias: 'sonnet' },
    ]
    const result = buildProviderModelsArray('claude', makeData(), makeItem(0), models)
    expect(result[0].models).toEqual([
      { name: 'claude-3-opus' },
      { name: 'claude-3-sonnet', alias: 'sonnet' },
    ])
  })

  it('filters out models with empty names', () => {
    const models: ConfigurableModelLike[] = [
      { name: 'valid' },
      { name: '' },
      { name: '  ' },
      { name: 'also-valid', alias: 'av' },
    ]
    const result = buildProviderModelsArray('claude', makeData(), makeItem(0), models)
    expect(result[0].models).toEqual([
      { name: 'valid' },
      { name: 'also-valid', alias: 'av' },
    ])
  })

  it('omits alias when it equals name', () => {
    const models: ConfigurableModelLike[] = [{ name: 'same', alias: 'same' }]
    const result = buildProviderModelsArray('claude', makeData(), makeItem(0), models)
    expect(result[0].models).toEqual([{ name: 'same' }])
  })

  it('throws when item index is out of range', () => {
    expect(() =>
      buildProviderModelsArray('claude', makeData(), makeItem(5), [{ name: 'x' }]),
    ).toThrow('未找到要更新的渠道')
  })
})

// ─── findApiKeyUsage ───

describe('findApiKeyUsage', () => {
  it('returns undefined when usage is undefined', () => {
    expect(findApiKeyUsage(undefined, 'claude', 'https://api', 'key')).toBeUndefined()
  })

  it('returns undefined when apiKey is empty', () => {
    const usage: ApiKeyUsageResponse = { claude: { '|key': { success: 1, failed: 0 } } }
    expect(findApiKeyUsage(usage, 'claude', 'https://api', '')).toBeUndefined()
  })

  it('finds usage by composite key baseUrl|apiKey', () => {
    const entry = { success: 5, failed: 1 }
    const usage: ApiKeyUsageResponse = { claude: { 'https://api|sk-123': entry } }
    expect(findApiKeyUsage(usage, 'claude', 'https://api', 'sk-123')).toBe(entry)
  })

  it('finds usage by |apiKey fallback', () => {
    const entry = { success: 3, failed: 0 }
    const usage: ApiKeyUsageResponse = { anthropic: { '|sk-456': entry } }
    expect(findApiKeyUsage(usage, 'claude', '', 'sk-456')).toBe(entry)
  })

  it('checks provider usage hints (claude checks anthropic bucket)', () => {
    const entry = { success: 10, failed: 2 }
    const usage: ApiKeyUsageResponse = { anthropic: { 'https://api|sk-789': entry } }
    expect(findApiKeyUsage(usage, 'claude', 'https://api', 'sk-789')).toBe(entry)
  })

  it('checks provider usage hints (openai checks openai-compatibility bucket)', () => {
    const entry = { success: 7, failed: 0 }
    const usage: ApiKeyUsageResponse = { 'openai-compatibility': { 'https://api|key': entry } }
    expect(findApiKeyUsage(usage, 'openai', 'https://api', 'key')).toBe(entry)
  })
})
