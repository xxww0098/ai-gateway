import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { fetchMgmtApi, fetchMgmtApiFormData } from './api'

// Mock auth store to provide a token
vi.mock('@/features/auth/auth_store', () => ({
  useAuthStore: {
    getState: () => ({
      token: 'test-token',
      logout: vi.fn(),
    }),
  },
}))

describe('fetchMgmtApi CLI Proxy API SDK Client', () => {
  let fetchSpy: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    fetchSpy = vi.spyOn(globalThis, 'fetch')
  })

  afterEach(() => {
    fetchSpy.mockRestore()
    vi.clearAllMocks()
  })

  it('routes JSON SDK management requests through the CPA admin proxy', async () => {
    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify({ code: 0, message: 'ok', data: { success: true, message: 'config updated' } }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )

    const res = await fetchMgmtApi('/config')

    expect(fetchSpy).toHaveBeenCalledTimes(1)
    const [url] = fetchSpy.mock.calls[0]
    expect(url).toBe('/api/panel/admin/sdk-management/config')
    expect(res).toEqual({ success: true, message: 'config updated' })
  })

  it('normalizes SDK management paths without a leading slash', async () => {
    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify({ code: 0, message: 'ok', data: { users: [] } }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )

    await fetchMgmtApi('users')

    expect(fetchSpy).toHaveBeenCalledTimes(1)
    const [url] = fetchSpy.mock.calls[0]
    expect(url).toBe('/api/panel/admin/sdk-management/users')
  })

  it('routes multipart SDK management requests through the CPA admin proxy', async () => {
    const form = new FormData()
    form.append('file', new Blob(['{}'], { type: 'application/json' }), 'auth.json')

    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify({ code: 0, message: 'ok', data: { ok: true } }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )

    const res = await fetchMgmtApiFormData('/auth-files', form)

    expect(fetchSpy).toHaveBeenCalledTimes(1)
    const [url] = fetchSpy.mock.calls[0]
    expect(url).toBe('/api/panel/admin/sdk-management/auth-files')
    // The legacy fetchMgmtApiFormData returns the raw response (with wrapper if present)
    expect(res).toEqual({ code: 0, message: 'ok', data: { ok: true } })
  })

  it('attaches Authorization header with Bearer token', async () => {
    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify({ code: 0, message: 'ok', data: {} }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )

    await fetchMgmtApi('/config')

    const [, options] = fetchSpy.mock.calls[0]
    const headers = options?.headers as Headers
    expect(headers.get('Authorization')).toBe('Bearer test-token')
  })

  it('supports PUT method with body', async () => {
    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify({ code: 0, message: 'ok', data: null }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )

    await fetchMgmtApi('/config', { method: 'PUT', body: JSON.stringify({ value: true }) })

    const [, options] = fetchSpy.mock.calls[0]
    expect(options?.method).toBe('PUT')
  })
})
