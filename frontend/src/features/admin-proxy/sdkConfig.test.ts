import { describe, expect, it } from 'vitest'
import {
  ROUTING_STRATEGY_OPTIONS,
  buildSDKExtraConfigPatch,
  normalizeDisableImageGeneration,
} from './sdkConfig'

describe('sdkConfig', () => {
  it('only exposes supported routing strategies', () => {
    expect(ROUTING_STRATEGY_OPTIONS.map(item => item.value)).toEqual(['round-robin', 'fill-first'])
  })

  it('builds the allow-listed extra config patch', () => {
    expect(buildSDKExtraConfigPatch({
      maxRetryCredentials: -1,
      redisUsageQueueRetentionSeconds: 9999,
      disableImageGeneration: 'chat',
      routing: {
        sessionAffinity: true,
        sessionAffinityTtl: ' 30m ',
      },
    })).toEqual({
      maxRetryCredentials: 0,
      redisUsageQueueRetentionSeconds: 3600,
      disableImageGeneration: 'chat',
      routing: {
        sessionAffinity: true,
        sessionAffinityTtl: '30m',
      },
    })
  })

  it('normalizes disable-image-generation values from SDK config', () => {
    expect(normalizeDisableImageGeneration(false)).toBe('off')
    expect(normalizeDisableImageGeneration(true)).toBe('all')
    expect(normalizeDisableImageGeneration('chat')).toBe('chat')
  })
})
