import { describe, expect, it } from 'vitest'
import { formatRelativeTime, pathForNotification } from './notificationNav'

describe('pathForNotification', () => {
  it('routes ticket types to ticket detail when related_id present', () => {
    expect(pathForNotification({ notification_type: 'ticket_reply', related_id: 42 })).toBe(
      '/tickets/42'
    )
  })

  it('routes ticket without id to list', () => {
    expect(pathForNotification({ notification_type: 'ticket' })).toBe('/tickets')
  })

  it('routes refund and payment types', () => {
    expect(pathForNotification({ notification_type: 'refund_approved' })).toBe('/refunds')
    expect(pathForNotification({ notification_type: 'payment_order' })).toBe('/orders')
  })

  it('uses system heuristics on Chinese copy', () => {
    expect(
      pathForNotification({
        notification_type: 'system',
        title: '工单有新回复',
        related_id: 9,
      })
    ).toBe('/tickets/9')
    expect(
      pathForNotification({
        notification_type: 'system',
        content: '您的退款申请已通过',
      })
    ).toBe('/refunds')
  })

  it('defaults welcome/system to dashboard', () => {
    expect(
      pathForNotification({
        notification_type: 'system',
        title: '欢迎使用 CPA Gateway',
        content: '账户面板已就绪',
      })
    ).toBe('/dashboard')
  })
})

describe('formatRelativeTime', () => {
  it('formats recent times', () => {
    const now = Date.parse('2026-07-10T12:00:00Z')
    expect(formatRelativeTime('2026-07-10T11:59:30Z', now)).toBe('刚刚')
    expect(formatRelativeTime('2026-07-10T11:30:00Z', now)).toBe('30 分钟前')
    expect(formatRelativeTime('2026-07-10T09:00:00Z', now)).toBe('3 小时前')
  })
})
