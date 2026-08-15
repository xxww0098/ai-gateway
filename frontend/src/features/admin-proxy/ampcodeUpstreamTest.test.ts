import { describe, expect, it, vi } from 'vitest'
import type { ApiCallRequest, ApiCallResult } from '@/features/admin-proxy/api'
import {
  buildAmpcodeControlPlaneEndpoints,
  normalizeAmpcodeGatewayBase,
  testAmpcodeUpstream,
} from './ampcodeUpstreamTest'

const apiResult = (statusCode: number, body: unknown = null): ApiCallResult => ({
  statusCode,
  header: {},
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
  })

  it('marks the upstream connected when the Amp control plane responds successfully', async () => {
    const request = vi.fn(async () =>
      apiResult(200, { id: 'user-1' })
    )

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com/',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('connected')
    expect(result.endpoint).toBe('https://api.example.com/api/user')
    expect(request).toHaveBeenCalledWith(expect.objectContaining({
      method: 'GET',
      url: 'https://api.example.com/api/user',
      header: expect.objectContaining({
        Authorization: 'Bearer sk-test',
        'X-Api-Key': 'sk-test',
      }),
    }))
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
    expect(result.message).toContain('认证失败')
    expect(request).toHaveBeenCalledTimes(1)
  })

  it('fails when neither control plane endpoint is present', async () => {
    const request = vi.fn()
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))
      .mockResolvedValueOnce(apiResult(404, { error: 'not found' }))

    const result = await testAmpcodeUpstream({
      upstreamUrl: 'https://api.example.com',
      upstreamApiKey: 'sk-test',
    }, request)

    expect(result.status).toBe('failed')
    expect(result.endpoint).toBe('https://api.example.com/api/auth')
    expect(result.message).toContain('不是 Amp control-plane')
  })
})
