import { describe, expect, it, vi } from 'vitest'
import type { ApiCallRequest, ApiCallResult } from '@/features/admin-proxy/api'
import {
  buildAmpcodeControlPlaneEndpoints,
  buildAmpcodeFallbackEndpoints,
  normalizeAmpcodeGatewayBase,
  testAmpcodeUpstream,
} from './ampcodeUpstreamTest'

const apiResult = (
  statusCode: number,
  body: unknown = null,
  header: Record<string, string[]> = {},
): ApiCallResult => ({
  statusCode,
  header,
  body,
  bodyText: body === null ? '' : JSON.stringify(body),
})

describe('ampcodeUpstreamTest', () => {
  it('normalizes pasted API paths to gateway base URL', () => {
    expect(normalizeAmpcodeGatewayBase('https://api.example.com/v1/chat/completions')).toBe('https://api.example.com')
    expect(normalizeAmpcodeGatewayBase('https://api.example.com/proxy/v1/models?debug=1')).toBe('https://api.example.com/proxy')
    expect(normalizeAmpcodeGatewayBase('https://api.example.com/api/user?debug=1')).toBe('https://api.example.com')
    expect(buildAmpcodeControlPlaneEndpoints('https://api.example.com/')).toEqual([
      'https://api.example.com/api/user',
      'https://api.example.com/api/auth',
    ])
    expect(buildAmpcodeFallbackEndpoints('https://api.example.com/')).toEqual([
      'https://api.example.com/v1/models',
      'https://api.example.com/models',
    ])
  })

  it('returns diagnosticStep on URL validation failure', async () => {
    const request = vi.fn()
    const emptyResult = await testAmpcodeUpstream({
      upstreamUrl: '',
      upstreamApiKey: '',
    }, request)
    expect(emptyResult.status).toBe('failed')
    expect(emptyResult.diagnosticStep).toBe('validate_url')

    const invalidResult = await testAmpcodeUpstream({
      upstreamUrl: 'ftp://example.com',
      upstreamApiKey: '',
    }, request)
    expect(invalidResult.status).toBe('failed')
    expect(invalidResult.diagnosticStep).toBe('validate_url')
    expect(request).not.toHaveBeenCalled()
  })

  it('marks the upstream connected when the Amp control plane responds successfully', async () => {
    const request = vi.fn(async () =>
      apiResult(200, { id: 'user-1' }, {
        'Content-Type': ['application/json'],
        'X-Request-Id': ['req-abc-123'],
      })
    )

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com/',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('connected')
    expect(result.endpoint).toBe('https://api.example.com/api/user')
    expect(result.diagnosticStep).toBe('control_plane_probe')
    expect(result.isLatencyHealthy).toBe(true)
    expect(result.headersPreview).toEqual({
      'content-type': 'application/json',
      'x-request-id': 'req-abc-123',
    })
    expect(request).toHaveBeenCalledWith(expect.objectContaining({
      method: 'GET',
      url: 'https://api.example.com/api/user',
      header: expect.objectContaining({
        Authorization: 'Bearer sk-test',
        'X-Api-Key': 'sk-test',
      }),
    }))
  })

  it('marks latency as unhealthy when elapsed time >= 1000ms', async () => {
    vi.useFakeTimers()
    const request = vi.fn(async () => {
      vi.advanceTimersByTime(1200)
      return apiResult(200, { id: 'user-1' })
    })

    const resultPromise = testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    const result = await resultPromise
    expect(result.isLatencyHealthy).toBe(false)
    vi.useRealTimers()
  })

  it('falls through to /api/auth when /api/user is missing', async () => {
    const request = vi.fn()
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(200, { status: 'ok' }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('connected')
    expect(result.endpoint).toBe('https://api.example.com/api/auth')
    expect(result.diagnosticStep).toBe('control_plane_probe')
    expect(request.mock.calls.map((call) => (call[0] as ApiCallRequest).url)).toEqual([
      'https://api.example.com/api/user',
      'https://api.example.com/api/auth',
    ])
  })

  it('treats non-auth non-404 control plane responses as reachable but not confirmed', async () => {
    const request = vi.fn()
      .mockResolvedValueOnce(apiResult(500, { error: 'server error' }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('reachable')
    expect(result.endpoint).toBe('https://api.example.com/api/user')
    expect(result.diagnosticStep).toBe('control_plane_probe')
    expect(result.message).toContain('control-plane 未确认可用')
  })

  it('reports authentication failures without masking them as healthy', async () => {
    const request = vi.fn().mockResolvedValueOnce(apiResult(401, { error: 'invalid key' }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('failed')
    expect(result.statusCode).toBe(401)
    expect(result.diagnosticStep).toBe('control_plane_auth')
    expect(result.message).toContain('认证失败')
    expect(request).toHaveBeenCalledTimes(1)
  })

  it('falls through to secondary fallback /v1/models when both control plane endpoints return 404', async () => {
    const request = vi.fn()
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(200, { data: [{ id: 'model-1' }] }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('connected')
    expect(result.endpoint).toBe('https://api.example.com/v1/models')
    expect(result.diagnosticStep).toBe('models_fallback_probe')
    expect(result.message).toContain('上游模型接口已响应')
    expect(request.mock.calls.map((call) => (call[0] as ApiCallRequest).url)).toEqual([
      'https://api.example.com/api/user',
      'https://api.example.com/api/auth',
      'https://api.example.com/v1/models',
    ])
  })

  it('falls through to secondary fallback /models when /v1/models returns 404', async () => {
    const request = vi.fn()
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(200, { data: [{ id: 'model-2' }] }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('connected')
    expect(result.endpoint).toBe('https://api.example.com/models')
    expect(result.diagnosticStep).toBe('models_fallback_probe')
  })

  it('reports auth failure on fallback endpoint', async () => {
    const request = vi.fn()
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(401, { error: 'unauthorized' }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('failed')
    expect(result.statusCode).toBe(401)
    expect(result.diagnosticStep).toBe('models_fallback_auth')
    expect(result.message).toContain('认证失败')
  })

  it('fails when neither control plane nor fallback endpoint is present', async () => {
    const request = vi.fn()
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('failed')
    expect(result.endpoint).toBe('https://api.example.com/models')
    expect(result.message).toContain('不是 Amp control-plane')
    expect(result.diagnosticStep).toBe('probe_failed')
  })

  it('does not probe fallback endpoints when allowFallback is false', async () => {
    const request = vi.fn()
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
      allowFallback: false,
    }, request)

    expect(result.status).toBe('failed')
    expect(result.endpoint).toBe('https://api.example.com/api/auth')
    expect(result.message).toContain('不是 Amp control-plane')
    expect(request).toHaveBeenCalledTimes(2)
  })
})

