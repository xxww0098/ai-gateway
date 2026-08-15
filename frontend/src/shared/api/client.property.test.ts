/**
 * Property-based tests for API client core utilities.
 *
 * Uses fast-check to verify universal correctness properties across
 * all valid input combinations.
 *
 * Validates: Requirements 1.2, 1.5, 1.6, 1.7
 */

import { describe, it, expect, vi } from 'vitest'
import fc from 'fast-check'

// Mock auth store to avoid localStorage errors in test environment
vi.mock('@/features/auth/auth_store', () => ({
  useAuthStore: {
    getState: () => ({ token: null, logout: vi.fn() }),
  },
}))

import { buildUrl } from './client'
import { extractErrorMessage } from './errors'
import { unwrapResponse } from './unwrap'

// ---------------------------------------------------------------------------
// Property 1: URL Construction Correctness
// Validates: Requirements 1.2, 1.7
// ---------------------------------------------------------------------------

describe('Property 1: URL Construction Correctness', () => {
  /**
   * **Validates: Requirements 1.2**
   *
   * For any base prefix and endpoint, buildUrl produces `{basePrefix}/{endpoint}`
   * where endpoint is normalized to start with `/`.
   */
  it('produces {basePrefix}/{endpoint} for endpoints starting with /', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }).map(s => '/' + s.replace(/\//g, '')), // basePrefix like /api/panel
        fc.string({ minLength: 1 }).map(s => '/' + s.replace(/\//g, '')), // endpoint like /users
        (basePrefix, endpoint) => {
          const result = buildUrl(basePrefix, endpoint)
          expect(result).toBe(`${basePrefix}${endpoint}`)
        }
      )
    )
  })

  it('normalizes endpoints without leading slash by prepending /', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }).map(s => '/' + s.replace(/\//g, '')), // basePrefix
        fc.string({ minLength: 1 }).filter(s => !s.startsWith('/')),       // endpoint without /
        (basePrefix, endpoint) => {
          const result = buildUrl(basePrefix, endpoint)
          expect(result).toBe(`${basePrefix}/${endpoint}`)
        }
      )
    )
  })

  it('default client prefix /api/panel always produces URLs starting with /api/panel/', () => {
    const defaultPrefix = '/api/panel'
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        (endpoint) => {
          const result = buildUrl(defaultPrefix, endpoint)
          expect(result.startsWith('/api/panel/')).toBe(true)
        }
      )
    )
  })

  it('result always contains the basePrefix as a prefix', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }).map(s => '/' + s.replace(/\//g, '')),
        fc.string({ minLength: 1 }),
        (basePrefix, endpoint) => {
          const result = buildUrl(basePrefix, endpoint)
          expect(result.startsWith(basePrefix)).toBe(true)
        }
      )
    )
  })
})

// ---------------------------------------------------------------------------
// Property 4: Error Message Extraction
// Validates: Requirements 1.5
// ---------------------------------------------------------------------------

describe('Property 4: Error Message Extraction', () => {
  /**
   * **Validates: Requirements 1.5**
   *
   * Priority order: msg → message → error (string) → error.message → fallback
   */
  it('returns msg field when present (highest priority)', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        fc.string({ minLength: 1 }),
        fc.string({ minLength: 1 }),
        (msg, message, errorStr) => {
          const data = { msg, message, error: errorStr }
          expect(extractErrorMessage(data)).toBe(msg)
        }
      )
    )
  })

  it('returns message field when msg is absent', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        fc.string({ minLength: 1 }),
        (message, errorStr) => {
          const data = { message, error: errorStr }
          expect(extractErrorMessage(data)).toBe(message)
        }
      )
    )
  })

  it('returns error string when msg and message are absent', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        (errorStr) => {
          const data = { error: errorStr }
          expect(extractErrorMessage(data)).toBe(errorStr)
        }
      )
    )
  })

  it('returns error.message when msg, message, and error string are absent', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        (nestedMsg) => {
          const data = { error: { message: nestedMsg } }
          expect(extractErrorMessage(data)).toBe(nestedMsg)
        }
      )
    )
  })

  it('returns fallback for null/undefined/non-object inputs', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        (fallback) => {
          expect(extractErrorMessage(null, fallback)).toBe(fallback)
          expect(extractErrorMessage(undefined, fallback)).toBe(fallback)
          expect(extractErrorMessage(42, fallback)).toBe(fallback)
          expect(extractErrorMessage('string', fallback)).toBe(fallback)
        }
      )
    )
  })

  it('returns fallback for objects with no recognized error fields', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        (fallback) => {
          expect(extractErrorMessage({}, fallback)).toBe(fallback)
          expect(extractErrorMessage({ foo: 'bar' }, fallback)).toBe(fallback)
          expect(extractErrorMessage({ code: 500 }, fallback)).toBe(fallback)
        }
      )
    )
  })

  it('skips empty string fields and falls through to next priority', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        (message) => {
          // Empty msg should fall through to message
          const data = { msg: '', message }
          expect(extractErrorMessage(data)).toBe(message)
        }
      )
    )
  })
})

// ---------------------------------------------------------------------------
// Property 5: Response Unwrapping Round-Trip
// Validates: Requirements 1.6
// ---------------------------------------------------------------------------

describe('Property 5: Response Unwrapping Round-Trip', () => {
  /**
   * **Validates: Requirements 1.6**
   *
   * Wrapping any value T as { code, data: T } and passing through unwrapResponse
   * returns a value equal to T.
   */
  it('extracts .data from wrapper objects { code, data }', () => {
    fc.assert(
      fc.property(
        fc.anything(),
        fc.integer(),
        (value, code) => {
          const wrapped = { code, message: 'ok', data: value }
          const result = unwrapResponse(wrapped)
          expect(result).toEqual(value)
        }
      )
    )
  })

  it('passes through non-wrapper values unchanged', () => {
    fc.assert(
      fc.property(
        fc.oneof(
          fc.string(),
          fc.integer(),
          fc.boolean(),
          fc.constant(null)
        ),
        (value) => {
          const result = unwrapResponse(value)
          expect(result).toEqual(value)
        }
      )
    )
  })

  it('passes through objects without code field unchanged', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        fc.string({ minLength: 1 }),
        (key, val) => {
          // Object without 'code' field should pass through
          const obj = { [key]: val }
          if (!('code' in obj && 'data' in obj)) {
            const result = unwrapResponse(obj)
            expect(result).toEqual(obj)
          }
        }
      )
    )
  })

  it('passes through objects with code but without data field unchanged', () => {
    fc.assert(
      fc.property(
        fc.integer(),
        fc.string({ minLength: 1 }),
        (code, message) => {
          const obj = { code, message }
          const result = unwrapResponse(obj)
          expect(result).toEqual(obj)
        }
      )
    )
  })

  it('round-trip: wrap then unwrap returns original value', () => {
    fc.assert(
      fc.property(
        fc.oneof(
          fc.string(),
          fc.integer(),
          fc.double({ noNaN: true }),
          fc.boolean(),
          fc.constant(null),
          fc.array(fc.integer()),
          fc.dictionary(fc.string({ minLength: 1, maxLength: 5 }), fc.string())
        ),
        (originalValue) => {
          const wrapped = { code: 0, message: 'ok', data: originalValue }
          const unwrapped = unwrapResponse(wrapped)
          expect(unwrapped).toEqual(originalValue)
        }
      )
    )
  })
})
