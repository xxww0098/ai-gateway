/**
 * Error extraction utilities for API responses.
 *
 * Handles various backend response formats and provides consistent
 * error message extraction for both response bodies and caught errors.
 */

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
