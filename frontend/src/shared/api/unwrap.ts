/**
 * Response unwrapping utility for the standard backend response envelope.
 *
 * The backend wraps responses in `{ code, message, data }`. This utility
 * extracts the `data` field when the wrapper shape is detected, or passes
 * through non-wrapper values unchanged.
 */

/**
 * Unwraps the standard `{ code, data }` response envelope.
 * If the response matches the wrapper shape, returns the `data` field.
 * Otherwise returns the raw parsed value as-is.
 */
export function unwrapResponse(data: unknown): unknown {
  if (
    data !== null &&
    typeof data === 'object' &&
    'code' in data &&
    'data' in data
  ) {
    return (data as { data: unknown }).data
  }
  return data
}
