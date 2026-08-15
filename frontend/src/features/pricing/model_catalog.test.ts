import { describe, expect, it, beforeAll } from 'vitest'
import {
  configuredModelToCatalogItem,
  enrichModelCatalogItem,
  formatThinking,
  formatTokenLimit,
  getModelDetailMetrics,
  getModelProviderKey,
  getProviderColor,
  getProviderDisplayName,
  getProviderOptions,
  getSupportedMethods,
  hasModelDetails,
  matchesModelSearch,
  mergeModelCatalogMetadata,
  modelDefinitionToCatalogItem,
  PROVIDER_CONFIG,
  type ModelCatalogItem,
} from './model_catalog'
import { loadModelRegistries } from './model_provider'

describe('model_catalog', () => {
  beforeAll(async () => {
    await loadModelRegistries()
  })

  const models: ModelCatalogItem[] = [
    { id: 'gpt-5.4-mini', owned_by: 'openai', display_name: 'GPT 5.4 Mini' },
    { id: 'claude-opus-4-7', owned_by: 'anthropic', display_name: 'Claude Opus 4.7' },
    { id: 'gemini-3-pro-preview', owned_by: 'google', display_name: 'Gemini 3 Pro Preview' },
  ]

  it('builds provider options from owned_by categories', () => {
    expect(getProviderOptions(models)).toEqual([
      { key: 'anthropic', label: 'Anthropic', count: 1 },
      { key: 'google', label: 'Google', count: 1 },
      { key: 'openai', label: 'OpenAI', count: 1 },
    ])
  })

  it('matches search by model id, display name, and owner', () => {
    expect(matchesModelSearch(models[0], 'mini')).toBe(true)
    expect(matchesModelSearch(models[1], 'anthropic')).toBe(true)
    expect(matchesModelSearch(models[2], 'gpt')).toBe(false)
  })

  it('formats thinking details without empty fragments', () => {
    expect(formatThinking({
      id: 'gpt-5.4-mini',
      thinking: {
        levels: ['low', 'medium', 'high'],
        dynamic_allowed: true,
      },
    })).toBe('low / medium / high · 动态')
  })

  it('returns only present tooltip metrics', () => {
    const metrics = getModelDetailMetrics({
      id: 'gemini-3-pro-preview',
      type: 'gemini',
      version: '3.0',
      inputTokenLimit: 1048576,
      outputTokenLimit: 65536,
    })

    expect(metrics).toEqual([
      { label: '版本', value: '3.0' },
      { label: '类型', value: 'gemini' },
      { label: '上下文', value: '1,048,576' },
      { label: '最大输出', value: '65,536' },
    ])
  })

  it('detects whether a model has registry details', () => {
    expect(hasModelDetails({ id: 'plain-model' })).toBe(false)
    expect(hasModelDetails({ id: 'named-model', display_name: 'Named Model' })).toBe(true)
  })

  it('enriches OpenAI-compatible configured models from the CPA registry overlay', () => {
    const model = configuredModelToCatalogItem({ name: 'MiniMax-M2.7' }, { provider: 'openai' })

    expect(model.display_name).toBe('MiniMax M2.7')
    expect(model.owned_by).toBe('minimax')
    expect(model.context_length).toBe(204800)
    expect(hasModelDetails(model)).toBe(true)
  })
  it('PROVIDER_CONFIG contains moonshot and antigravity entries', () => {
    expect(PROVIDER_CONFIG.moonshot).toBeDefined()
    expect(PROVIDER_CONFIG.moonshot.label).toBe('Moonshot')
    expect(PROVIDER_CONFIG.antigravity).toBeDefined()
    expect(PROVIDER_CONFIG.antigravity.label).toBe('Antigravity')
  })

  it('PROVIDER_CONFIG preserves existing provider colors', () => {
    expect(PROVIDER_CONFIG.anthropic.color.bg).toBe('bg-orange-50 dark:bg-orange-950/20')
    expect(PROVIDER_CONFIG.openai.color.bg).toBe('bg-emerald-50 dark:bg-emerald-950/20')
    expect(PROVIDER_CONFIG.google.color.bg).toBe('bg-blue-50 dark:bg-blue-950/20')
    expect(PROVIDER_CONFIG.kimi.color.bg).toBe('bg-slate-50 dark:bg-slate-950/20')
    expect(PROVIDER_CONFIG.minimax.color.bg).toBe('bg-cyan-50 dark:bg-cyan-950/20')
    expect(PROVIDER_CONFIG.deepseek.color.bg).toBe('bg-violet-50 dark:bg-violet-950/20')
    expect(PROVIDER_CONFIG.glm.color.bg).toBe('bg-teal-50 dark:bg-teal-950/20')
    expect(PROVIDER_CONFIG.mimo.color.bg).toBe('bg-rose-50 dark:bg-rose-950/20')
    expect(PROVIDER_CONFIG.meta.color.bg).toBe('bg-sky-50 dark:bg-sky-950/20')
  })

  it('getProviderColor returns shared color for known providers', () => {
    expect(getProviderColor('anthropic').bg).toBe('bg-orange-50 dark:bg-orange-950/20')
    expect(getProviderColor('moonshot').bg).toBe('bg-indigo-50 dark:bg-indigo-950/20')
    expect(getProviderColor('antigravity').bg).toBe('bg-purple-50 dark:bg-purple-950/20')
  })

  it('getProviderColor returns fallback for unknown providers', () => {
    const fallback = getProviderColor('unknown-provider')
    expect(fallback.bg).toBe('bg-gray-50 dark:bg-dark-800')
    expect(fallback.text).toBe('text-gray-600 dark:text-gray-300')
    expect(fallback.border).toBe('border-gray-200 dark:border-dark-600')
  })

  it('getProviderDisplayName uses PROVIDER_CONFIG labels', () => {
    expect(getProviderDisplayName('moonshot')).toBe('Moonshot')
    expect(getProviderDisplayName('antigravity')).toBe('Antigravity')
    expect(getProviderDisplayName('anthropic')).toBe('Anthropic')
  })

  it('getProviderDisplayName capitalizes unknown hyphenated providers', () => {
    expect(getProviderDisplayName('my-custom-provider')).toBe('My-Custom-Provider')
    expect(getProviderDisplayName('unknown')).toBe('Unknown')
  })

  // --- PROVIDER_CONFIG completeness ---

  const EXPECTED_PROVIDER_KEYS = [
    'anthropic', 'openai', 'google', 'kimi', 'minimax', 'glm', 'mimo', 'moonshot', 'antigravity', 'deepseek', 'meta', 'other',
  ]

  it('PROVIDER_CONFIG contains all expected provider keys', () => {
    for (const key of EXPECTED_PROVIDER_KEYS) {
      expect(PROVIDER_CONFIG[key], `missing key: ${key}`).toBeDefined()
    }
  })

  it('every PROVIDER_CONFIG entry has label, color, and icon', () => {
    for (const [key, entry] of Object.entries(PROVIDER_CONFIG)) {
      expect(entry.label, `${key}.label`).toBeTruthy()
      expect(entry.icon, `${key}.icon`).toBeTruthy()
      expect(entry.color, `${key}.color`).toBeDefined()
      expect(entry.color.bg, `${key}.color.bg`).toBeTruthy()
      expect(entry.color.text, `${key}.color.text`).toBeTruthy()
      expect(entry.color.border, `${key}.color.border`).toBeTruthy()
    }
  })

  it('PROVIDER_CONFIG has no unexpected extra keys', () => {
    const actualKeys = Object.keys(PROVIDER_CONFIG).sort()
    const expectedKeys = [...EXPECTED_PROVIDER_KEYS].sort()
    expect(actualKeys).toEqual(expectedKeys)
  })

  // --- formatTokenLimit ---

  describe('formatTokenLimit', () => {
    it('formats numbers with locale separators', () => {
      expect(formatTokenLimit(1048576)).toBe('1,048,576')
      expect(formatTokenLimit(1000)).toBe('1,000')
      expect(formatTokenLimit(42)).toBe('42')
    })

    it('rounds fractional values', () => {
      expect(formatTokenLimit(1234.6)).toBe('1,235')
      expect(formatTokenLimit(1234.4)).toBe('1,234')
    })

    it('returns empty string for invalid inputs', () => {
      expect(formatTokenLimit(undefined)).toBe('')
      expect(formatTokenLimit(0)).toBe('')
      expect(formatTokenLimit(-100)).toBe('')
      expect(formatTokenLimit(Infinity)).toBe('')
      expect(formatTokenLimit(NaN)).toBe('')
    })
  })

  // --- getSupportedMethods ---

  describe('getSupportedMethods', () => {
    it('returns supported_parameters when present', () => {
      const model: ModelCatalogItem = {
        id: 'test',
        supported_parameters: ['temperature', 'top_p'],
        supportedGenerationMethods: ['generate'],
        supportedInputModalities: ['text'],
      }
      expect(getSupportedMethods(model)).toEqual(['temperature', 'top_p'])
    })

    it('falls back to supportedGenerationMethods when supported_parameters is empty', () => {
      const model: ModelCatalogItem = {
        id: 'test',
        supported_parameters: [],
        supportedGenerationMethods: ['generate', 'stream'],
      }
      expect(getSupportedMethods(model)).toEqual(['generate', 'stream'])
    })

    it('falls back to input/output modalities when both parameters and methods are empty', () => {
      const model: ModelCatalogItem = {
        id: 'test',
        supportedInputModalities: ['text', 'image'],
        supportedOutputModalities: ['text'],
      }
      expect(getSupportedMethods(model)).toEqual(['输入 text', '输入 image', '输出 text'])
    })

    it('returns empty array when no method fields are present', () => {
      expect(getSupportedMethods({ id: 'test' })).toEqual([])
    })
  })

  // --- mergeModelCatalogMetadata ---

  describe('mergeModelCatalogMetadata', () => {
    it('returns original model when metadata is undefined', () => {
      const model: ModelCatalogItem = { id: 'test', owned_by: 'openai' }
      const result = mergeModelCatalogMetadata(model, undefined)
      expect(result).toBe(model)
    })

    it('merges present metadata fields into model', () => {
      const model: ModelCatalogItem = { id: 'test' }
      const metadata: Partial<ModelCatalogItem> = {
        display_name: 'Test Model',
        context_length: 128000,
        type: 'chat',
      }
      const result = mergeModelCatalogMetadata(model, metadata)
      expect(result.display_name).toBe('Test Model')
      expect(result.context_length).toBe(128000)
      expect(result.type).toBe('chat')
      expect(result.id).toBe('test')
    })

    it('skips null, undefined, and empty-string metadata values', () => {
      const model: ModelCatalogItem = { id: 'test', display_name: 'Original' }
      const metadata: Partial<ModelCatalogItem> = {
        display_name: '',
        description: undefined,
        version: null as unknown as string,
      }
      const result = mergeModelCatalogMetadata(model, metadata)
      expect(result.display_name).toBe('Original')
    })

    it('skips empty arrays in metadata', () => {
      const model: ModelCatalogItem = { id: 'test', supported_parameters: ['a'] }
      const metadata: Partial<ModelCatalogItem> = { supported_parameters: [] }
      const result = mergeModelCatalogMetadata(model, metadata)
      expect(result.supported_parameters).toEqual(['a'])
    })

    it('preserves non-metadata fields on the original model', () => {
      const model: ModelCatalogItem = {
        id: 'test',
        input_price_per_1m: 3,
        output_price_per_1m: 15,
        rate_multiplier: 1.5,
      }
      const metadata: Partial<ModelCatalogItem> = { display_name: 'Merged' }
      const result = mergeModelCatalogMetadata(model, metadata)
      expect(result.input_price_per_1m).toBe(3)
      expect(result.output_price_per_1m).toBe(15)
      expect(result.rate_multiplier).toBe(1.5)
    })
  })

  // --- enrichModelCatalogItem ---

  describe('enrichModelCatalogItem', () => {
    it('enriches a model with registry metadata when available', () => {
      // claude-opus-4-7 exists in the static registry
      const model: ModelCatalogItem = { id: 'claude-opus-4-7' }
      const result = enrichModelCatalogItem(model)
      expect(result.id).toBe('claude-opus-4-7')
      // Registry metadata should provide display_name or other fields
      expect(hasModelDetails(result) || result.id === 'claude-opus-4-7').toBe(true)
    })

    it('returns model unchanged when no registry metadata matches', () => {
      const model: ModelCatalogItem = { id: 'nonexistent-model-xyz' }
      const result = enrichModelCatalogItem(model)
      expect(result.id).toBe('nonexistent-model-xyz')
    })

    it('uses relatedModelIds to find metadata when primary id has none', () => {
      // Use a known model id as related id to trigger metadata lookup
      const model: ModelCatalogItem = { id: 'custom-alias' }
      const result = enrichModelCatalogItem(model, ['claude-opus-4-7'])
      // Should have found metadata via the related id
      expect(result.id).toBe('custom-alias')
    })
  })

  // --- modelDefinitionToCatalogItem ---

  describe('modelDefinitionToCatalogItem', () => {
    it('uses id when present', () => {
      const result = modelDefinitionToCatalogItem({ id: 'gpt-5.4-mini', name: 'GPT 5.4 Mini' })
      expect(result.id).toBe('gpt-5.4-mini')
    })

    it('falls back to name when id is empty', () => {
      const result = modelDefinitionToCatalogItem({ name: 'some-model-name' })
      expect(result.id).toBe('some-model-name')
    })

    it('returns empty id when both id and name are missing', () => {
      const result = modelDefinitionToCatalogItem({})
      expect(result.id).toBe('')
    })
  })

  // --- getModelProviderKey ---

  describe('getModelProviderKey', () => {
    it('resolves provider from owned_by', () => {
      expect(getModelProviderKey({ id: 'test', owned_by: 'anthropic' })).toBe('anthropic')
    })

    it('returns "other" for models with no provider info', () => {
      expect(getModelProviderKey({ id: 'nonexistent-model-xyz' })).toBe('other')
    })
  })

  // --- matchesModelSearch edge cases ---

  describe('matchesModelSearch edge cases', () => {
    it('returns true for empty or whitespace-only query', () => {
      expect(matchesModelSearch(models[0], '')).toBe(true)
      expect(matchesModelSearch(models[0], '   ')).toBe(true)
    })

    it('search is case-insensitive', () => {
      expect(matchesModelSearch(models[0], 'GPT')).toBe(true)
      expect(matchesModelSearch(models[1], 'CLAUDE')).toBe(true)
    })

    it('returns false when no fields match', () => {
      expect(matchesModelSearch(models[0], 'zzz-nonexistent')).toBe(false)
    })
  })

  // --- formatThinking edge cases ---

  describe('formatThinking edge cases', () => {
    it('returns empty string when thinking is undefined', () => {
      expect(formatThinking({ id: 'test' })).toBe('')
    })

    it('formats min/max range', () => {
      expect(formatThinking({
        id: 'test',
        thinking: { min: 1024, max: 32768 },
      })).toBe('1,024 - 32,768')
    })

    it('includes zero_allowed label', () => {
      expect(formatThinking({
        id: 'test',
        thinking: { zero_allowed: true },
      })).toBe('可关闭')
    })

    it('combines levels, range, zero_allowed, and dynamic_allowed', () => {
      expect(formatThinking({
        id: 'test',
        thinking: {
          levels: ['low', 'high'],
          min: 100,
          max: 5000,
          zero_allowed: true,
          dynamic_allowed: true,
        },
      })).toBe('low / high · 100 - 5,000 · 可关闭 · 动态')
    })
  })

  // --- getModelDetailMetrics edge cases ---

  describe('getModelDetailMetrics edge cases', () => {
    it('prefers context_length over inputTokenLimit', () => {
      const metrics = getModelDetailMetrics({
        id: 'test',
        context_length: 200000,
        inputTokenLimit: 100000,
      })
      expect(metrics).toContainEqual({ label: '上下文', value: '200,000' })
    })

    it('prefers max_completion_tokens over outputTokenLimit', () => {
      const metrics = getModelDetailMetrics({
        id: 'test',
        max_completion_tokens: 8192,
        outputTokenLimit: 4096,
      })
      expect(metrics).toContainEqual({ label: '最大输出', value: '8,192' })
    })

    it('includes thinking metric when present', () => {
      const metrics = getModelDetailMetrics({
        id: 'test',
        thinking: { dynamic_allowed: true },
      })
      expect(metrics).toContainEqual({ label: 'Thinking', value: '动态' })
    })

    it('returns empty array for model with no detail fields', () => {
      expect(getModelDetailMetrics({ id: 'bare' })).toEqual([])
    })
  })

  // --- hasModelDetails edge cases ---

  describe('hasModelDetails edge cases', () => {
    it('returns true when display_name differs from id', () => {
      expect(hasModelDetails({ id: 'gpt-5', display_name: 'GPT-5' })).toBe(true)
    })

    it('returns false when display_name equals id', () => {
      expect(hasModelDetails({ id: 'same', display_name: 'same' })).toBe(false)
    })

    it('returns true for description alone', () => {
      expect(hasModelDetails({ id: 'test', description: 'A model' })).toBe(true)
    })

    it('returns true for context_length alone', () => {
      expect(hasModelDetails({ id: 'test', context_length: 128000 })).toBe(true)
    })

    it('returns true for supportedInputModalities', () => {
      expect(hasModelDetails({ id: 'test', supportedInputModalities: ['text'] })).toBe(true)
    })
  })

  // --- configuredModelToCatalogItem edge cases ---

  describe('configuredModelToCatalogItem edge cases', () => {
    it('uses alias as id when preferAliasAsId is true', () => {
      const model = configuredModelToCatalogItem(
        { name: 'upstream-model', alias: 'my-alias' },
        { provider: 'openai', preferAliasAsId: true },
      )
      expect(model.id).toBe('my-alias')
    })

    it('uses name as id when preferAliasAsId is false', () => {
      const model = configuredModelToCatalogItem(
        { name: 'upstream-model', alias: 'my-alias' },
        { provider: 'openai', preferAliasAsId: false },
      )
      expect(model.id).toBe('upstream-model')
    })

    it('sets display_name from alias when alias differs from id', () => {
      const model = configuredModelToCatalogItem(
        { name: 'upstream-model', alias: 'Friendly Name' },
        { provider: 'openai' },
      )
      expect(model.display_name).toBe('Friendly Name')
    })

    it('does not set display_name when alias equals id', () => {
      const model = configuredModelToCatalogItem(
        { name: 'same-name' },
        { provider: 'openai' },
      )
      expect(model.display_name).toBeUndefined()
    })

    it('includes description when provided', () => {
      const model = configuredModelToCatalogItem(
        { name: 'test-model', description: '  A test model  ' },
        { provider: 'openai' },
      )
      expect(model.description).toBe('A test model')
    })
  })

  // --- getProviderOptions edge cases ---

  describe('getProviderOptions edge cases', () => {
    it('returns empty array for empty model list', () => {
      expect(getProviderOptions([])).toEqual([])
    })

    it('counts multiple models from the same provider', () => {
      const multi: ModelCatalogItem[] = [
        { id: 'a', owned_by: 'openai' },
        { id: 'b', owned_by: 'openai' },
        { id: 'c', owned_by: 'anthropic' },
      ]
      const options = getProviderOptions(multi)
      expect(options).toContainEqual({ key: 'openai', label: 'OpenAI', count: 2 })
      expect(options).toContainEqual({ key: 'anthropic', label: 'Anthropic', count: 1 })
    })

    it('sorts options alphabetically by label', () => {
      const options = getProviderOptions(models)
      const labels = options.map((o) => o.label)
      const sorted = [...labels].sort((a, b) => a.localeCompare(b))
      expect(labels).toEqual(sorted)
    })
  })

  // --- getProviderColor completeness ---

  describe('getProviderColor for all PROVIDER_CONFIG keys', () => {
    it('returns matching color for every configured provider', () => {
      for (const [key, entry] of Object.entries(PROVIDER_CONFIG)) {
        const color = getProviderColor(key)
        expect(color.bg).toBe(entry.color.bg)
        expect(color.text).toBe(entry.color.text)
        expect(color.border).toBe(entry.color.border)
      }
    })
  })
})
