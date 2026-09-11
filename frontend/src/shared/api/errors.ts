/**
 * Error extraction utilities for API responses.
 *
 * Handles various backend response formats and provides consistent
 * error message extraction for both response bodies and caught errors.
 */

/** Vite/nginx 在网关未监听时返回的空 body 5xx，不能只显示「请求异常」。 */
export const GATEWAY_UNREACHABLE =
  '无法连接后端。请确认网关已在 8888 端口启动。'

/**
 * Fallback when the response body has no extractable message.
 * 502/503/504 with an empty Vite proxy body is the local-dev "gateway down" case.
 */
export function httpFallbackMessage(status: number, statusText = ''): string {
  if (status === 502 || status === 503 || status === 504) {
    return GATEWAY_UNREACHABLE
  }
  return statusText || '请求异常'
}

/**
 * Extracts a human-readable error message from various backend response formats.
 * Priority order: msg → message → error (string) → error.message (nested) → fallback
 */
export function extractErrorMessage(data: unknown, fallback = '请求异常'): string {
  if (!data || typeof data !== 'object') return fallback
  const d = data as Record<string, unknown>
  if (typeof d.msg === 'string' && d.msg) return d.msg
  if (typeof d.message === 'string' && d.message) return d.message
  if (typeof d.error === 'string' && d.error) return d.error
  if (typeof d.error === 'object' && d.error !== null) {
    const nested = d.error as Record<string, unknown>
    if (typeof nested.message === 'string' && nested.message) return nested.message
  }
  return fallback
}

/**
 * Extracts error message from any caught error value.
 * Useful in catch blocks where the error type is unknown.
 */
export function errorMessage(error: unknown, fallback = '请求异常'): string {
  if (error instanceof Error) return error.message
  return fallback
}
