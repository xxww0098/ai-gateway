import { useAuthStore } from "@/features/auth/auth_store"
import { extractErrorMessage } from "./errors"
import { unwrapResponse } from "./unwrap"

// Re-export utilities for convenience
export { errorMessage } from "./errors"

// ---------------------------------------------------------------------------
// ApiError
// ---------------------------------------------------------------------------

/** Typed API error thrown on non-2xx responses. */
export class ApiError extends Error {
  status: number
  code?: number

  constructor(message: string, status: number, code?: number) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

// ---------------------------------------------------------------------------
// Typed API Client Factory
// ---------------------------------------------------------------------------

/** Configuration for API client instances */
export interface ClientConfig {
  basePrefix: string
}

/** Typed API client interface */
export interface TypedApiClient {
  get: <T>(endpoint: string, options?: RequestInit) => Promise<T>
  post: <T>(endpoint: string, body?: unknown, options?: RequestInit) => Promise<T>
  put: <T>(endpoint: string, body?: unknown, options?: RequestInit) => Promise<T>
  patch: <T>(endpoint: string, body?: unknown, options?: RequestInit) => Promise<T>
  delete: <T>(endpoint: string, options?: RequestInit) => Promise<T>
}

/**
 * Constructs a full URL from a base prefix and endpoint.
 * Exported for testability; used internally by createApiClient.
 */
export function buildUrl(basePrefix: string, endpoint: string): string {
  const path = endpoint.startsWith('/') ? endpoint : `/${endpoint}`
  return `${basePrefix}${path}`
}

/**
 * Creates a typed API client with configurable base prefix.
 * Handles auth token injection, 401 logout, and response unwrapping.
 */
export function createApiClient(config: ClientConfig = { basePrefix: '/api/panel' }): TypedApiClient {
  const { basePrefix } = config

  async function request<T>(endpoint: string, options: RequestInit = {}): Promise<T> {
    const token = useAuthStore.getState().token
    const headers = new Headers(options.headers || {})
    headers.set('Content-Type', 'application/json')
    if (token) {
      headers.set('Authorization', `Bearer ${token}`)
    }

    const response = await fetch(buildUrl(basePrefix, endpoint), { ...options, headers })
    const rawText = await response.text()
    let data: unknown
    try { data = JSON.parse(rawText) } catch { data = null }

    if (!response.ok) {
      if (response.status === 401) {
        useAuthStore.getState().logout()
      }
      const message = extractErrorMessage(data, response.statusText || '请求异常')
      throw new ApiError(message, response.status)
    }

    return unwrapResponse(data) as T
  }

  return {
    get: <T>(endpoint: string, options?: RequestInit) =>
      request<T>(endpoint, { ...options, method: 'GET' }),
    post: <T>(endpoint: string, body?: unknown, options?: RequestInit) =>
      request<T>(endpoint, { ...options, method: 'POST', body: JSON.stringify(body) }),
    put: <T>(endpoint: string, body?: unknown, options?: RequestInit) =>
      request<T>(endpoint, { ...options, method: 'PUT', body: JSON.stringify(body) }),
    patch: <T>(endpoint: string, body?: unknown, options?: RequestInit) =>
      request<T>(endpoint, { ...options, method: 'PATCH', body: JSON.stringify(body) }),
    delete: <T>(endpoint: string, options?: RequestInit) =>
      request<T>(endpoint, { ...options, method: 'DELETE' }),
  }
}

/** Default panel API client (basePrefix: '/api/panel') */
export const apiClient = createApiClient()

/** SDK management API client (basePrefix: '/api/panel/admin/sdk-management') */
export const sdkClient = createApiClient({ basePrefix: '/api/panel/admin/sdk-management' })

// ---------------------------------------------------------------------------
// Legacy API — preserved for backward compatibility during migration
// ---------------------------------------------------------------------------

/** Returns true if the error is from an aborted fetch request. */
export function isAbortError(err: unknown): boolean {
  return typeof err === 'object' && err !== null && 'name' in err && err.name === 'AbortError'
}

/**
 * Legacy fetch wrapper. Preserved for backward compatibility during migration.
 * New code should use `apiClient` or `sdkClient` instead.
 */
export async function fetchApi(endpoint: string, options: RequestInit = {}) {
  const token = useAuthStore.getState().token
  
  const headers = new Headers(options.headers || {})
  headers.set('Content-Type', 'application/json')
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }

  const response = await fetch(`/api/panel${endpoint}`, {
    ...options,
    headers,
  })

  const rawText = await response.text()
  let data
  try {
    data = JSON.parse(rawText)
  } catch {
    data = null
  }

  if (!response.ok) {
    if (response.status === 401) {
      useAuthStore.getState().logout()
    }
    const errField = data?.error
    const errStr = typeof errField === 'string' ? errField : errField?.message
    const message = data?.msg || data?.message || errStr || response.statusText || '请求异常'
    throw new Error(message)
  }

  // Assuming our standard wrapper { code: 0, message: "成功", data: ... }
  if (data && typeof data === 'object' && 'code' in data) {
    return data
  }
  
  return data
}

/** multipart/form-data，勿设置 Content-Type（由浏览器带 boundary） */
export async function fetchApiFormData(endpoint: string, formData: FormData) {
  const token = useAuthStore.getState().token
  const headers = new Headers()
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }
  const response = await fetch(`/api/panel${endpoint}`, {
    method: 'POST',
    body: formData,
    headers,
  })
  const rawText = await response.text()
  let data: unknown
  try {
    data = JSON.parse(rawText)
  } catch {
    data = null
  }
  if (!response.ok) {
    if (response.status === 401) {
      useAuthStore.getState().logout()
    }
    const d = data as { msg?: string; message?: string; error?: string | { message?: string } } | null
    const errField = d?.error
    const errStr = typeof errField === 'string' ? errField : errField?.message
    const message = d?.msg || d?.message || errStr || response.statusText || '请求异常'
    throw new Error(message)
  }
  if (data && typeof data === 'object' && 'code' in (data as object)) {
    return data as { code: number; message?: string; data?: Record<string, unknown> }
  }
  return data
}

/** 已登录下载二进制（如同源相对路径 /api/panel/...） */
export async function fetchApiBlob(pathOrUrl: string): Promise<Blob> {
  const token = useAuthStore.getState().token
  const headers = new Headers()
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }
  const url =
    pathOrUrl.startsWith('http://') || pathOrUrl.startsWith('https://')
      ? pathOrUrl
      : pathOrUrl.startsWith('/api/')
        ? pathOrUrl
        : `/api/panel${pathOrUrl.startsWith('/') ? pathOrUrl : `/${pathOrUrl}`}`
  const response = await fetch(url, { headers })
  if (!response.ok) {
    if (response.status === 401) {
      useAuthStore.getState().logout()
    }
    throw new Error(response.statusText || '加载失败')
  }
  return response.blob()
}
