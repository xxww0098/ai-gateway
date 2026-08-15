import { describe, expect, it, vi } from 'vitest'
import { modelsApi, resolveSDKChannel } from './model_prices'

vi.mock('@/features/auth/auth_store', () => ({
  useAuthStore: {
    getState: vi.fn(() => ({ token: null, logout: vi.fn() })),
  },
}))

describe('model_prices', () => {
  it('does not map OpenAI compatibility providers to SDK static channels', () => {
    expect(resolveSDKChannel('OpenAI (兼容格式)')).toBe('')
    expect(resolveSDKChannel('openai compatibility')).toBe('')
  })

  it('keeps first-party SDK channels mapped to their registry channels', () => {
    expect(resolveSDKChannel('Claude')).toBe('claude')
    expect(resolveSDKChannel('Gemini')).toBe('gemini')
    expect(resolveSDKChannel('GCP Vertex AI')).toBe('vertex')
  })

  it('uses custom OpenAI-compatible model list URL before deriving /v1/models', () => {
    expect(modelsApi.buildOpenAIModelsEndpoint(
      'https://ark.cn-beijing.volces.com/api/coding/v3',
      'https://ark.cn-beijing.volces.com/api/v3/models',
    )).toBe('https://ark.cn-beijing.volces.com/api/v3/models')
    expect(modelsApi.buildOpenAIModelsEndpoint(
      'https://ark.cn-beijing.volces.com/api/coding/v3',
      '/api/v3/models',
    )).toBe('https://ark.cn-beijing.volces.com/api/v3/models')
  })
})
