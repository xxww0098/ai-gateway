import { describe, it, expect } from 'vitest'
import {
  GATEWAY_INFERENCE_PROTOCOLS,
  GATEWAY_SDK_BASES,
  protocolEndpointUrl,
  sdkBaseUrl,
} from './endpoints'

describe('gateway inference protocols', () => {
  it('exposes the three live POST entries with SDK-correct bases', () => {
    expect(GATEWAY_INFERENCE_PROTOCOLS.map((item) => item.path)).toEqual([
      '/v1/chat/completions',
      '/v1/responses',
      '/v1/messages',
    ])
    expect(GATEWAY_INFERENCE_PROTOCOLS.every((item) => item.method === 'POST')).toBe(true)

    const origin = 'https://gw.example'
    expect(protocolEndpointUrl(origin, '/v1/chat/completions')).toBe(
      'https://gw.example/v1/chat/completions',
    )
    expect(sdkBaseUrl(origin, 'openai')).toBe('https://gw.example/v1')
    expect(sdkBaseUrl(origin, 'anthropic')).toBe('https://gw.example')

    expect(GATEWAY_SDK_BASES.map((item) => item.id)).toEqual(['openai', 'anthropic'])
  })
})
