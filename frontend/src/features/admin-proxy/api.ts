import { sdkClient } from '@/shared/api/client'
import { useAuthStore } from '@/features/auth/auth_store'
import { extractErrorMessage } from '@/shared/api/errors'
import type { AuthFilesResponse, AuthFileStatusRequest, AuthFileUploadResponse } from './types'

// ── SDK Management API (uses sdkClient with base /api/panel/admin/sdk-management) ──

/**
 * Generic SDK management GET request.
 * Used for fetching provider configs, auth-files, logs, etc.
 */
export function fetchProviderConfig<T = unknown>(endpoint: string): Promise<T> {
  return sdkClient.get<T>(endpoint)
}

/**
 * Generic SDK management PUT request.
 * Used for updating provider arrays, config values, etc.
 */
export function updateProviderConfig<T = unknown>(endpoint: string, body: unknown): Promise<T> {
  return sdkClient.put<T>(endpoint, body)
}

/**
 * Generic SDK management DELETE request.
 */
export function deleteProviderConfig<T = unknown>(endpoint: string, body?: unknown): Promise<T> {
  if (body !== undefined) {
    return sdkClient.delete<T>(endpoint, { body: JSON.stringify(body) })
  }
  return sdkClient.delete<T>(endpoint)
}

/**
 * Generic SDK management POST request.
 */
export function postProviderConfig<T = unknown>(endpoint: string, body?: unknown): Promise<T> {
  return sdkClient.post<T>(endpoint, body)
}

/** SDK-style manual OAuth callback (antigravity, etc.) via redirect_url. */
export function submitSdkOAuthCallback(body: {
  provider: string
  redirect_url: string
}): Promise<unknown> {
  return sdkClient.post('/oauth-callback', body)
}

/** Gateway DB OAuth session completion with code + state. */
export function submitGatewayOAuthCallback(
  provider: string,
  body: { code: string; state: string }
): Promise<unknown> {
  return sdkClient.post(`/oauth-callback/${encodeURIComponent(provider)}`, body)
}

// ── Auth Files API ──────────────────────────────────────────────────────────

export function fetchAuthFiles(): Promise<AuthFilesResponse> {
  return sdkClient.get<AuthFilesResponse>('/auth-files')
}

export function toggleAuthFileStatus(name: string, disabled: boolean): Promise<unknown> {
  const body: AuthFileStatusRequest = { name, disabled }
  return sdkClient.patch('/auth-files/status', body)
}

export function deleteAuthFile(name: string): Promise<unknown> {
  return sdkClient.delete(`/auth-files?name=${encodeURIComponent(name)}`)
}

/**
 * Updates editable fields (label/prefix/proxy_url, project_id/location/base_url,
 * api_key/access_token/refresh_token/id_token, service_account) on a stored
 * auth record. Empty strings clear the field; masked previews are ignored
 * server-side so accidental re-submission of "abcd...wxyz" never overwrites
 * a real secret.
 */
export function updateAuthFile(
  id: string,
  fields: Record<string, unknown>
): Promise<unknown> {
  return sdkClient.put('/auth-files', { action: 'update', id, fields })
}

/**
 * Downloads a single auth record as an upload-compatible JSON file.
 * Returns a Blob so callers can trigger a browser save.
 */
export async function downloadAuthFile(target: { id?: string; name?: string }): Promise<{ blob: Blob; filename: string }> {
  const token = useAuthStore.getState().token
  const headers = new Headers()
  if (token) headers.set('Authorization', `Bearer ${token}`)
  const params = new URLSearchParams()
  if (target.id) params.set('id', target.id)
  else if (target.name) params.set('name', target.name)
  const response = await fetch(
    `/api/panel/admin/sdk-management/auth-files/download?${params.toString()}`,
    { headers }
  )
  if (!response.ok) {
    if (response.status === 401) useAuthStore.getState().logout()
    let message = response.statusText || '下载失败'
    try {
      const data = await response.json()
      message = extractErrorMessage(data, message)
    } catch {
      /* ignore */
    }
    throw new Error(message)
  }
  return { blob: await response.blob(), filename: extractFilename(response, 'auth-file.json') }
}

/**
 * Downloads a zip containing one JSON file per requested auth record.
 */
export async function exportAuthFiles(ids: string[]): Promise<{ blob: Blob; filename: string }> {
  const token = useAuthStore.getState().token
  const headers = new Headers()
  headers.set('Content-Type', 'application/json')
  if (token) headers.set('Authorization', `Bearer ${token}`)
  const response = await fetch('/api/panel/admin/sdk-management/auth-files/export', {
    method: 'POST',
    headers,
    body: JSON.stringify({ ids }),
  })
  if (!response.ok) {
    if (response.status === 401) useAuthStore.getState().logout()
    let message = response.statusText || '导出失败'
    try {
      const data = await response.json()
      message = extractErrorMessage(data, message)
    } catch {
      /* ignore */
    }
    throw new Error(message)
  }
  return { blob: await response.blob(), filename: extractFilename(response, 'auth-files.zip') }
}

function extractFilename(response: Response, fallback: string): string {
  const header = response.headers.get('Content-Disposition') || ''
  // Prefer RFC 5987 filename* if present, then the plain filename="...".
  const star = /filename\*=(?:UTF-8'')?([^;]+)/i.exec(header)
  if (star) {
    try {
      return decodeURIComponent(star[1].replace(/^"|"$/g, ''))
    } catch {
      /* fallthrough */
    }
  }
  const plain = /filename="?([^";]+)"?/i.exec(header)
  if (plain) return plain[1]
  return fallback
}

/**
 * Triggers a browser save dialog for the given blob.
 */
export function saveBlobToFile(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  document.body.appendChild(link)
  link.click()
  link.remove()
  URL.revokeObjectURL(url)
}

/**
 * Upload auth files via multipart/form-data.
 * Cannot use sdkClient (which sets Content-Type: application/json).
 * Uses raw fetch with auth token injection.
 */
export async function uploadAuthFiles(formData: FormData): Promise<AuthFileUploadResponse> {
  const token = useAuthStore.getState().token
  const headers = new Headers()
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }
  const response = await fetch('/api/panel/admin/sdk-management/auth-files', {
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
    const message = extractErrorMessage(data, response.statusText || '请求异常')
    throw new Error(message)
  }
  // Unwrap standard wrapper if present
  if (data && typeof data === 'object' && 'code' in (data as object) && 'data' in (data as object)) {
    return (data as { data: AuthFileUploadResponse }).data
  }
  return (data ?? {}) as AuthFileUploadResponse
}

// ── API Key Usage ───────────────────────────────────────────────────────────

export function fetchApiKeyUsage<T = unknown>(): Promise<T> {
  return sdkClient.get<T>('/api-key-usage')
}

// ── API Call (proxy request) ────────────────────────────────────────────────

export interface ApiCallRequest {
  method: string
  url: string
  header?: Record<string, string>
  data?: string
}

export interface ApiCallResult<T = unknown> {
  statusCode: number
  header: Record<string, string[]>
  bodyText: string
  body: T | null
}

export async function apiCallRequest(payload: ApiCallRequest): Promise<ApiCallResult> {
  const response = await sdkClient.post<unknown>('/api-call', payload)
  const data = typeof response === 'string' ? JSON.parse(response) : response

  const record = data as Record<string, unknown> | null
  const statusCode = Number(record?.status_code ?? record?.statusCode ?? 0)
  const header = (record?.header ?? record?.headers ?? {}) as Record<string, string[]>

  let bodyText = ''
  let body: unknown = null

  if (record?.body !== undefined) {
    if (typeof record.body === 'string') {
      bodyText = record.body
      try {
        body = JSON.parse(bodyText)
      } catch {
        body = bodyText
      }
    } else {
      body = record.body
      try {
        bodyText = JSON.stringify(body)
      } catch {
        bodyText = String(body)
      }
    }
  }

  return { statusCode, header, bodyText, body }
}

// ── Legacy wrappers (preserved for backward compatibility during migration) ──

/**
 * @deprecated Use specific API functions or sdkClient directly.
 * Legacy wrapper that routes through SDK management endpoint.
 */
export async function fetchMgmtApi(endpoint: string, options: RequestInit = {}) {
  const suffix = endpoint.startsWith('/') ? endpoint : `/${endpoint}`
  const method = (options.method || 'GET').toUpperCase()

  if (method === 'GET') {
    return sdkClient.get(suffix)
  }
  if (method === 'PUT') {
    const body = options.body ? JSON.parse(options.body as string) : undefined
    return sdkClient.put(suffix, body)
  }
  if (method === 'POST') {
    const body = options.body ? JSON.parse(options.body as string) : undefined
    return sdkClient.post(suffix, body)
  }
  if (method === 'PATCH') {
    const body = options.body ? JSON.parse(options.body as string) : undefined
    return sdkClient.patch(suffix, body)
  }
  if (method === 'DELETE') {
    if (options.body) {
      return sdkClient.delete(suffix, { body: options.body as string })
    }
    return sdkClient.delete(suffix)
  }

  return sdkClient.get(suffix)
}

/**
 * @deprecated Use uploadAuthFiles() instead.
 * Legacy multipart/form-data wrapper for SDK management endpoints.
 */
export async function fetchMgmtApiFormData(endpoint: string, formData: FormData) {
  const suffix = endpoint.startsWith('/') ? endpoint : `/${endpoint}`
  const token = useAuthStore.getState().token
  const headers = new Headers()
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }
  const response = await fetch(`/api/panel/admin/sdk-management${suffix}`, {
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
    const message = extractErrorMessage(data, response.statusText || '请求异常')
    throw new Error(message)
  }
  if (data && typeof data === 'object' && 'code' in (data as object)) {
    return data as { code: number; message?: string; data?: Record<string, unknown> }
  }
  return data
}
