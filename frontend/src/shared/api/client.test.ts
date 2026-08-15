import { describe, it, expect, vi, beforeEach } from 'vitest'
import { fetchApi, isAbortError, createApiClient, ApiError, apiClient, sdkClient } from '@/shared/api/client'
import { extractErrorMessage, errorMessage } from '@/shared/api/errors'
import { unwrapResponse } from '@/shared/api/unwrap'
import { useAuthStore } from '@/features/auth/auth_store'

// Mock global fetch
globalThis.fetch = vi.fn()

// Mock Zustand store
vi.mock('@/features/auth/auth_store', () => ({
  useAuthStore: {
    getState: vi.fn()
  }
}))

describe('fetchApi User API Client', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    
    // Default mock store
    vi.mocked(useAuthStore.getState).mockReturnValue({
      token: 'fake-jwt-token',
      logout: vi.fn(),
    } as unknown as ReturnType<typeof useAuthStore.getState>)
  })

  it('should attach Bearer token if present in auth store', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: 'success', message: 'ok' })
    } as unknown as Response)

    const res = await fetchApi('/test-endpoint')
    
    expect(globalThis.fetch).toHaveBeenCalledWith('/api/panel/test-endpoint', expect.objectContaining({
      headers: expect.any(Headers)
    }))
    
    const passedOptions = vi.mocked(globalThis.fetch).mock.calls[0]?.[1] || {}
    const headers = passedOptions.headers as Headers
    expect(headers.get('Authorization')).toBe('Bearer fake-jwt-token')
    expect(res).toEqual({ code: 0, data: 'success', message: 'ok' })
  })

  it('should handle standard wrapped response and return raw data if wrapped', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: { bal: 10 }, message: 'ok' })
    } as unknown as Response)

    const res = await fetchApi('/profile')
    // As per api.ts: `if (data && typeof data === 'object' && 'code' in data) { return data }`
    // So it should return the exact wrapped struct.
    expect(res).toEqual({ code: 0, data: { bal: 10 }, message: 'ok' })
  })

  it('should intercept 401 Unauthorized and implicitly call logout()', async () => {
    const logoutMock = vi.fn()
    vi.mocked(useAuthStore.getState).mockReturnValue({
      token: 'expired-token',
      logout: logoutMock,
    } as unknown as ReturnType<typeof useAuthStore.getState>)

    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: false,
      status: 401,
      statusText: 'Unauthorized',
      text: async () => JSON.stringify({ message: "token_expired" })
    } as unknown as Response)

    // fetchApi throws Error when not OK
    await expect(fetchApi('/secret-page')).rejects.toThrow('token_expired')
    expect(logoutMock).toHaveBeenCalled()
  })

  it('should bubble up custom Error messages from non-OK statuses', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: false,
      status: 400,
      text: async () => JSON.stringify({ error: { message: "Invalid parameters" } })
    } as unknown as Response)

    await expect(fetchApi('/submit')).rejects.toThrow("Invalid parameters")
  })
})

describe('isAbortError', () => {
  it('should return true for DOMException with name AbortError', () => {
    expect(isAbortError(new DOMException('aborted', 'AbortError'))).toBe(true)
  })

  it('should return true for Error with name AbortError', () => {
    const err = new Error('aborted')
    err.name = 'AbortError'
    expect(isAbortError(err)).toBe(true)
  })

  it('should return false for regular errors', () => {
    expect(isAbortError(new Error('network error'))).toBe(false)
  })

  it('should return false for non-error values', () => {
    expect(isAbortError(null)).toBe(false)
    expect(isAbortError('string')).toBe(false)
    expect(isAbortError(undefined)).toBe(false)
  })
})

describe('fetchApi signal support', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useAuthStore.getState).mockReturnValue({
      token: 'fake-jwt-token',
      logout: vi.fn(),
    } as unknown as ReturnType<typeof useAuthStore.getState>)
  })

  it('should pass signal to underlying fetch', async () => {
    const controller = new AbortController()
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: 'ok', message: 'ok' })
    } as unknown as Response)

    await fetchApi('/test', { signal: controller.signal })

    const passedOptions = vi.mocked(globalThis.fetch).mock.calls[0]?.[1] || {}
    expect(passedOptions.signal).toBe(controller.signal)
  })

  it('should propagate AbortError from fetch', async () => {
    const abortError = new DOMException('The operation was aborted.', 'AbortError')
    vi.mocked(globalThis.fetch).mockRejectedValueOnce(abortError)

    await expect(fetchApi('/test')).rejects.toThrow(abortError)
  })
})

// ---------------------------------------------------------------------------
// New typed API client tests
// ---------------------------------------------------------------------------

describe('createApiClient', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useAuthStore.getState).mockReturnValue({
      token: 'test-token',
      logout: vi.fn(),
    } as unknown as ReturnType<typeof useAuthStore.getState>)
  })

  it('should prepend basePrefix to endpoint', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: { id: 1 }, message: 'ok' })
    } as unknown as Response)

    const client = createApiClient({ basePrefix: '/api/panel' })
    await client.get('/users')

    expect(globalThis.fetch).toHaveBeenCalledWith('/api/panel/users', expect.anything())
  })

  it('should normalize endpoint without leading slash', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: null, message: 'ok' })
    } as unknown as Response)

    const client = createApiClient({ basePrefix: '/api/panel' })
    await client.get('users')

    expect(globalThis.fetch).toHaveBeenCalledWith('/api/panel/users', expect.anything())
  })

  it('should attach Bearer token from auth store', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: 'ok', message: 'ok' })
    } as unknown as Response)

    await apiClient.get('/test')

    const passedOptions = vi.mocked(globalThis.fetch).mock.calls[0]?.[1] || {}
    const headers = passedOptions.headers as Headers
    expect(headers.get('Authorization')).toBe('Bearer test-token')
  })

  it('should not attach Authorization header when token is null', async () => {
    vi.mocked(useAuthStore.getState).mockReturnValue({
      token: null,
      logout: vi.fn(),
    } as unknown as ReturnType<typeof useAuthStore.getState>)

    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: null, message: 'ok' })
    } as unknown as Response)

    await apiClient.get('/public')

    const passedOptions = vi.mocked(globalThis.fetch).mock.calls[0]?.[1] || {}
    const headers = passedOptions.headers as Headers
    expect(headers.get('Authorization')).toBeNull()
  })

  it('should unwrap response data from standard wrapper', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: { name: 'Alice' }, message: 'ok' })
    } as unknown as Response)

    const result = await apiClient.get<{ name: string }>('/user')
    expect(result).toEqual({ name: 'Alice' })
  })

  it('should pass through non-wrapper responses', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ items: [1, 2, 3] })
    } as unknown as Response)

    const result = await apiClient.get<{ items: number[] }>('/items')
    expect(result).toEqual({ items: [1, 2, 3] })
  })

  it('should throw ApiError on non-2xx response', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: false,
      status: 400,
      statusText: 'Bad Request',
      text: async () => JSON.stringify({ msg: '参数错误' })
    } as unknown as Response)

    try {
      await apiClient.post('/submit', { bad: true })
      expect.fail('should have thrown')
    } catch (err) {
      expect(err).toBeInstanceOf(ApiError)
      expect((err as ApiError).message).toBe('参数错误')
      expect((err as ApiError).status).toBe(400)
    }
  })

  it('should call logout on 401 response', async () => {
    const logoutMock = vi.fn()
    vi.mocked(useAuthStore.getState).mockReturnValue({
      token: 'expired',
      logout: logoutMock,
    } as unknown as ReturnType<typeof useAuthStore.getState>)

    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: false,
      status: 401,
      statusText: 'Unauthorized',
      text: async () => JSON.stringify({ message: 'token expired' })
    } as unknown as Response)

    await expect(apiClient.get('/protected')).rejects.toThrow('token expired')
    expect(logoutMock).toHaveBeenCalled()
  })

  it('should send POST body as JSON', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: { id: 42 }, message: 'ok' })
    } as unknown as Response)

    await apiClient.post('/create', { name: 'test' })

    const passedOptions = vi.mocked(globalThis.fetch).mock.calls[0]?.[1] || {}
    expect(passedOptions.body).toBe(JSON.stringify({ name: 'test' }))
    expect(passedOptions.method).toBe('POST')
  })

  it('should send PUT body as JSON', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: null, message: 'ok' })
    } as unknown as Response)

    await apiClient.put('/update/1', { name: 'updated' })

    const passedOptions = vi.mocked(globalThis.fetch).mock.calls[0]?.[1] || {}
    expect(passedOptions.body).toBe(JSON.stringify({ name: 'updated' }))
    expect(passedOptions.method).toBe('PUT')
  })

  it('should send PATCH body as JSON', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: null, message: 'ok' })
    } as unknown as Response)

    await apiClient.patch('/patch/1', { status: 'active' })

    const passedOptions = vi.mocked(globalThis.fetch).mock.calls[0]?.[1] || {}
    expect(passedOptions.body).toBe(JSON.stringify({ status: 'active' }))
    expect(passedOptions.method).toBe('PATCH')
  })

  it('should send DELETE request', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: null, message: 'ok' })
    } as unknown as Response)

    await apiClient.delete('/remove/1')

    const passedOptions = vi.mocked(globalThis.fetch).mock.calls[0]?.[1] || {}
    expect(passedOptions.method).toBe('DELETE')
  })
})

describe('sdkClient', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useAuthStore.getState).mockReturnValue({
      token: 'sdk-token',
      logout: vi.fn(),
    } as unknown as ReturnType<typeof useAuthStore.getState>)
  })

  it('should use SDK management base prefix', async () => {
    vi.mocked(globalThis.fetch).mockResolvedValueOnce({
      ok: true,
      text: async () => JSON.stringify({ code: 0, data: [], message: 'ok' })
    } as unknown as Response)

    await sdkClient.get('/providers')

    expect(globalThis.fetch).toHaveBeenCalledWith(
      '/api/panel/admin/sdk-management/providers',
      expect.anything()
    )
  })
})

// ---------------------------------------------------------------------------
// extractErrorMessage tests
// ---------------------------------------------------------------------------

describe('extractErrorMessage', () => {
  it('should return msg field first', () => {
    expect(extractErrorMessage({ msg: '错误', message: '其他' })).toBe('错误')
  })

  it('should return message field when msg is absent', () => {
    expect(extractErrorMessage({ message: '消息' })).toBe('消息')
  })

  it('should return error string field', () => {
    expect(extractErrorMessage({ error: '出错了' })).toBe('出错了')
  })

  it('should return nested error.message', () => {
    expect(extractErrorMessage({ error: { message: '嵌套错误' } })).toBe('嵌套错误')
  })

  it('should return fallback for null', () => {
    expect(extractErrorMessage(null)).toBe('请求异常')
  })

  it('should return fallback for non-object', () => {
    expect(extractErrorMessage('string')).toBe('请求异常')
    expect(extractErrorMessage(42)).toBe('请求异常')
  })

  it('should return custom fallback', () => {
    expect(extractErrorMessage(null, 'custom')).toBe('custom')
  })

  it('should skip empty string fields', () => {
    expect(extractErrorMessage({ msg: '', message: '', error: 'fallback error' })).toBe('fallback error')
  })
})

// ---------------------------------------------------------------------------
// errorMessage tests
// ---------------------------------------------------------------------------

describe('errorMessage', () => {
  it('should extract message from Error instance', () => {
    expect(errorMessage(new Error('something failed'))).toBe('something failed')
  })

  it('should return fallback for non-Error values', () => {
    expect(errorMessage('string')).toBe('请求异常')
    expect(errorMessage(null)).toBe('请求异常')
    expect(errorMessage(undefined)).toBe('请求异常')
  })

  it('should return custom fallback', () => {
    expect(errorMessage(42, '操作失败')).toBe('操作失败')
  })

  it('should extract message from ApiError', () => {
    expect(errorMessage(new ApiError('api failed', 500))).toBe('api failed')
  })
})

// ---------------------------------------------------------------------------
// unwrapResponse tests
// ---------------------------------------------------------------------------

describe('unwrapResponse', () => {
  it('should extract data from standard wrapper', () => {
    const wrapped = { code: 0, message: 'ok', data: { id: 1, name: 'test' } }
    expect(unwrapResponse(wrapped)).toEqual({ id: 1, name: 'test' })
  })

  it('should extract null data from wrapper', () => {
    const wrapped = { code: 0, message: 'ok', data: null }
    expect(unwrapResponse(wrapped)).toBeNull()
  })

  it('should pass through non-wrapper objects', () => {
    const raw = { items: [1, 2, 3], total: 3 }
    expect(unwrapResponse(raw)).toEqual({ items: [1, 2, 3], total: 3 })
  })

  it('should pass through null', () => {
    expect(unwrapResponse(null)).toBeNull()
  })

  it('should pass through primitive values', () => {
    expect(unwrapResponse('hello')).toBe('hello')
    expect(unwrapResponse(42)).toBe(42)
  })

  it('should handle wrapper with code but no data field as non-wrapper', () => {
    const partial = { code: 0, message: 'ok' }
    // Has 'code' but no 'data' → pass through
    expect(unwrapResponse(partial)).toEqual({ code: 0, message: 'ok' })
  })
})
