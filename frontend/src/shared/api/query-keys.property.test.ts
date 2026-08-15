/**
 * Property-based tests for query key factory determinism.
 *
 * Uses fast-check to verify that calling the same query key factory function
 * with the same parameters always produces structurally identical arrays (deep equality).
 *
 * Validates: Requirements 3.2
 */

import { describe, it, expect } from 'vitest'
import fc from 'fast-check'
import { queryKeys } from './query-keys'

// ---------------------------------------------------------------------------
// Property 6: Query Key Factory Determinism
// Validates: Requirements 3.2
// ---------------------------------------------------------------------------

describe('Property 6: Query Key Factory Determinism', () => {
  /**
   * **Validates: Requirements 3.2**
   *
   * For any feature module and any set of parameters, calling the same query key
   * factory function with the same parameters always produces an identical array value.
   */

  it('auth.profile() always returns the same structure', () => {
    fc.assert(
      fc.property(
        fc.constant(undefined),
        () => {
          const result1 = queryKeys.auth.profile()
          const result2 = queryKeys.auth.profile()
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('users.all() always returns the same structure', () => {
    fc.assert(
      fc.property(
        fc.constant(undefined),
        () => {
          const result1 = queryKeys.users.all()
          const result2 = queryKeys.users.all()
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('users.list() is deterministic for any page/pageSize params', () => {
    fc.assert(
      fc.property(
        fc.nat({ max: 10000 }),
        fc.nat({ max: 100 }).map(n => n + 1),
        (page, pageSize) => {
          const params = { page, pageSize }
          const result1 = queryKeys.users.list(params)
          const result2 = queryKeys.users.list(params)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('users.detail() is deterministic for any user id', () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 1000000 }),
        (id) => {
          const result1 = queryKeys.users.detail(id)
          const result2 = queryKeys.users.detail(id)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('subscriptions.list() is deterministic for any page/pageSize params', () => {
    fc.assert(
      fc.property(
        fc.nat({ max: 10000 }),
        fc.nat({ max: 100 }).map(n => n + 1),
        (page, pageSize) => {
          const params = { page, pageSize }
          const result1 = queryKeys.subscriptions.list(params)
          const result2 = queryKeys.subscriptions.list(params)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('apiKeys.all() and apiKeys.list() always return the same structure', () => {
    fc.assert(
      fc.property(
        fc.constant(undefined),
        () => {
          expect(queryKeys.apiKeys.all()).toEqual(queryKeys.apiKeys.all())
          expect(queryKeys.apiKeys.list()).toEqual(queryKeys.apiKeys.list())
        }
      )
    )
  })

  it('tickets.list() is deterministic for any page/pageSize params', () => {
    fc.assert(
      fc.property(
        fc.nat({ max: 10000 }),
        fc.nat({ max: 100 }).map(n => n + 1),
        (page, pageSize) => {
          const params = { page, pageSize }
          const result1 = queryKeys.tickets.list(params)
          const result2 = queryKeys.tickets.list(params)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('orders.list() is deterministic for any page/pageSize params', () => {
    fc.assert(
      fc.property(
        fc.nat({ max: 10000 }),
        fc.nat({ max: 100 }).map(n => n + 1),
        (page, pageSize) => {
          const params = { page, pageSize }
          const result1 = queryKeys.orders.list(params)
          const result2 = queryKeys.orders.list(params)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('usage.logs() is deterministic for any record params', () => {
    fc.assert(
      fc.property(
        fc.dictionary(
          fc.string({ minLength: 1, maxLength: 10 }),
          fc.oneof(fc.string(), fc.integer(), fc.boolean())
        ),
        (params) => {
          const result1 = queryKeys.usage.logs(params)
          const result2 = queryKeys.usage.logs(params)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('usage.summary() always returns the same structure', () => {
    fc.assert(
      fc.property(
        fc.constant(undefined),
        () => {
          const result1 = queryKeys.usage.summary()
          const result2 = queryKeys.usage.summary()
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('pricing factories always return the same structure', () => {
    fc.assert(
      fc.property(
        fc.constant(undefined),
        () => {
          expect(queryKeys.pricing.all()).toEqual(queryKeys.pricing.all())
          expect(queryKeys.pricing.models()).toEqual(queryKeys.pricing.models())
        }
      )
    )
  })

  it('groups factories always return the same structure', () => {
    fc.assert(
      fc.property(
        fc.constant(undefined),
        () => {
          expect(queryKeys.groups.all()).toEqual(queryKeys.groups.all())
          expect(queryKeys.groups.list()).toEqual(queryKeys.groups.list())
        }
      )
    )
  })

  it('proxy.authFiles() is deterministic for any optional record params', () => {
    fc.assert(
      fc.property(
        fc.option(
          fc.dictionary(
            fc.string({ minLength: 1, maxLength: 10 }),
            fc.oneof(fc.string(), fc.integer(), fc.boolean())
          ),
          { nil: undefined }
        ),
        (params) => {
          const result1 = queryKeys.proxy.authFiles(params)
          const result2 = queryKeys.proxy.authFiles(params)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('proxy.providers() always returns the same structure', () => {
    fc.assert(
      fc.property(
        fc.constant(undefined),
        () => {
          const result1 = queryKeys.proxy.providers()
          const result2 = queryKeys.proxy.providers()
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('payment.wechatOrder() is deterministic for any orderId', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1, maxLength: 50 }),
        (orderId) => {
          const result1 = queryKeys.payment.wechatOrder(orderId)
          const result2 = queryKeys.payment.wechatOrder(orderId)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('payment.wechatStatus() is deterministic for any orderId', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1, maxLength: 50 }),
        (orderId) => {
          const result1 = queryKeys.payment.wechatStatus(orderId)
          const result2 = queryKeys.payment.wechatStatus(orderId)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('dashboard.trend() is deterministic for any days value', () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 365 }),
        (days) => {
          const result1 = queryKeys.dashboard.trend(days)
          const result2 = queryKeys.dashboard.trend(days)
          expect(result1).toEqual(result2)
        }
      )
    )
  })

  it('dashboard parameterless factories always return the same structure', () => {
    fc.assert(
      fc.property(
        fc.constant(undefined),
        () => {
          expect(queryKeys.dashboard.all()).toEqual(queryKeys.dashboard.all())
          expect(queryKeys.dashboard.stats()).toEqual(queryKeys.dashboard.stats())
          expect(queryKeys.dashboard.recentUsage()).toEqual(queryKeys.dashboard.recentUsage())
        }
      )
    )
  })

  it('different parameters produce different keys (discrimination)', () => {
    fc.assert(
      fc.property(
        fc.nat({ max: 10000 }),
        fc.nat({ max: 10000 }),
        fc.nat({ max: 100 }).map(n => n + 1),
        (page1, page2, pageSize) => {
          fc.pre(page1 !== page2)
          const result1 = queryKeys.users.list({ page: page1, pageSize })
          const result2 = queryKeys.users.list({ page: page2, pageSize })
          expect(result1).not.toEqual(result2)
        }
      )
    )
  })

  it('keys are arrays with correct module prefix', () => {
    fc.assert(
      fc.property(
        fc.nat({ max: 10000 }),
        fc.nat({ max: 100 }).map(n => n + 1),
        (page, pageSize) => {
          const params = { page, pageSize }
          expect(Array.isArray(queryKeys.users.list(params))).toBe(true)
          expect(queryKeys.users.list(params)[0]).toBe('users')
          expect(Array.isArray(queryKeys.tickets.list(params))).toBe(true)
          expect(queryKeys.tickets.list(params)[0]).toBe('tickets')
          expect(Array.isArray(queryKeys.orders.list(params))).toBe(true)
          expect(queryKeys.orders.list(params)[0]).toBe('orders')
        }
      )
    )
  })
})
