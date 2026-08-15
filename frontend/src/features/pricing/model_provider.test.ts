import { describe, expect, it, beforeAll } from 'vitest'
import {
  getModelRegistryMetadata,
  resolveModelProvider,
  getProviderForModelId,
  loadModelRegistries,
  type ProviderAwareModel,
} from './model_provider'

describe('model_provider', () => {
  beforeAll(async () => {
    await loadModelRegistries()
  })
  describe('getProviderForModelId', () => {
    it('returns the correct provider for known model ids', () => {
      // claude models should resolve to 'anthropic'
      expect(getProviderForModelId('claude-opus-4-7')).toBe('anthropic')
      // openai models should resolve to 'openai'
      expect(getProviderForModelId('gpt-5.4-mini')).toBe('openai')
    })

    it('returns "other" for unknown model ids', () => {
      expect(getProviderForModelId('nonexistent-model-xyz')).toBe('other')
    })

    it('returns "other" for empty or whitespace-only ids', () => {
      expect(getProviderForModelId('')).toBe('other')
      expect(getProviderForModelId('   ')).toBe('other')
    })

    it('normalizes model id case for lookup', () => {
      // The registry stores lowercase ids; lookup should normalize
      const result = getProviderForModelId('CLAUDE-OPUS-4-7')
      expect(result).toBe('anthropic')
    })

    it('resolves provider aliases (moonshot → kimi, zai → glm, xiaomi → mimo)', () => {
      // moonshot alias maps to kimi
      expect(getProviderForModelId('kimi-k2')).toBe('kimi')
      // zai alias maps to glm
      expect(getProviderForModelId('glm-5.1')).toBe('glm')
      // xiaomi alias maps to mimo
      expect(getProviderForModelId('mimo-v2.5-pro')).toBe('mimo')
    })

    it('resolves providers from cpa-models-registry (minimax, deepseek)', () => {
      expect(getProviderForModelId('MiniMax-M2.7')).toBe('minimax')
      expect(getProviderForModelId('deepseek-v4-flash')).toBe('deepseek')
    })
  })

  describe('getModelRegistryMetadata', () => {
    it('returns metadata for known model ids', () => {
      const meta = getModelRegistryMetadata('claude-opus-4-7')
      expect(meta).toBeDefined()
      expect(meta?.id).toBe('claude-opus-4-7')
    })

    it('returns undefined for unknown model ids', () => {
      expect(getModelRegistryMetadata('nonexistent-model-xyz')).toBeUndefined()
    })

    it('returns undefined for empty or undefined input', () => {
      expect(getModelRegistryMetadata(undefined)).toBeUndefined()
      expect(getModelRegistryMetadata('')).toBeUndefined()
      expect(getModelRegistryMetadata('   ')).toBeUndefined()
    })

    it('returns metadata with expected fields for registry models', () => {
      const meta = getModelRegistryMetadata('gemini-2.5-pro')
      expect(meta).toBeDefined()
      expect(meta?.owned_by).toBe('google')
      expect(meta?.type).toBe('gemini')
    })

    it('returns metadata for cpa-models-registry models', () => {
      const meta = getModelRegistryMetadata('mimo-v2.5-pro')
      expect(meta).toBeDefined()
      expect(meta?.owned_by).toBe('xiaomi')
    })
  })

  describe('resolveModelProvider', () => {
    it('resolves provider from owned_by field directly', () => {
      const model: ProviderAwareModel = { id: 'test-model', owned_by: 'anthropic' }
      expect(resolveModelProvider(model)).toBe('anthropic')
    })

    it('resolves provider from type field when owned_by is absent', () => {
      const model: ProviderAwareModel = { id: 'test-model', type: 'gemini' }
      expect(resolveModelProvider(model)).toBe('google')
    })

    it('falls back to registry lookup by model id', () => {
      const model: ProviderAwareModel = { id: 'claude-opus-4-7' }
      expect(resolveModelProvider(model)).toBe('anthropic')
    })

    it('returns "other" when no provider can be determined', () => {
      const model: ProviderAwareModel = { id: 'nonexistent-model-xyz' }
      expect(resolveModelProvider(model)).toBe('other')
    })

    it('prioritizes owned_by over type and registry lookup', () => {
      const model: ProviderAwareModel = { id: 'claude-opus-4-7', owned_by: 'custom-provider', type: 'gemini' }
      expect(resolveModelProvider(model)).toBe('custom-provider')
    })

    it('resolves moonshot-owned models to kimi provider', () => {
      const model: ProviderAwareModel = { id: 'kimi-k2', owned_by: 'moonshot' }
      expect(resolveModelProvider(model)).toBe('kimi')
    })

    it('resolves zai-owned models to glm provider', () => {
      const model: ProviderAwareModel = { id: 'glm-5.1', owned_by: 'zai' }
      expect(resolveModelProvider(model)).toBe('glm')
    })

    it('resolves xiaomi-owned models to mimo provider', () => {
      const model: ProviderAwareModel = { id: 'mimo-v2.5-pro', owned_by: 'xiaomi' }
      expect(resolveModelProvider(model)).toBe('mimo')
    })

    it('resolves provider from type field for kimi models', () => {
      const model: ProviderAwareModel = { id: 'test-kimi', type: 'kimi' }
      expect(resolveModelProvider(model)).toBe('kimi')
    })

    it('returns raw value when not in alias map', () => {
      const model: ProviderAwareModel = { id: 'test-model', owned_by: 'antigravity' }
      expect(resolveModelProvider(model)).toBe('antigravity')
    })
  })

  describe('lazy initialization', () => {
    it('populates maps on first access and caches for subsequent calls', () => {
      // First call triggers initialization
      const first = getProviderForModelId('claude-opus-4-7')
      // Second call should use cached maps
      const second = getProviderForModelId('claude-opus-4-7')
      expect(first).toBe(second)
      expect(first).toBe('anthropic')
    })

    it('getModelRegistryMetadata returns consistent results across calls', () => {
      const first = getModelRegistryMetadata('gpt-5.4-mini')
      const second = getModelRegistryMetadata('gpt-5.4-mini')
      expect(first).toEqual(second)
    })
  })
})
