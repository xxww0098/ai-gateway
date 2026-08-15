import { userRoutes, userTicketPath } from '@/shared/routes/user'

export type NotificationNavInput = {
  notification_type?: string | null
  related_id?: number | null
  title?: string | null
  content?: string | null
}

/**
 * Map a panel notification to a user-facing path.
 * Types are forward-compatible: backend may only send "system" today.
 */
export function pathForNotification(item: NotificationNavInput): string {
  const type = (item.notification_type || 'system').toLowerCase().trim()
  const id = item.related_id
  const blob = `${item.title ?? ''} ${item.content ?? ''}`.toLowerCase()

  if (
    type === 'ticket' ||
    type === 'ticket_reply' ||
    type === 'ticket_update' ||
    type.includes('ticket')
  ) {
    return id != null && id > 0 ? userTicketPath(id) : userRoutes.tickets
  }

  if (
    type === 'refund' ||
    type === 'refund_status' ||
    type === 'refund_approved' ||
    type === 'refund_rejected' ||
    type.includes('refund')
  ) {
    return userRoutes.refunds
  }

  if (
    type === 'payment' ||
    type === 'order' ||
    type === 'payment_order' ||
    type.includes('payment') ||
    type.includes('order')
  ) {
    return userRoutes.orders
  }

  if (
    type === 'subscription' ||
    type === 'subscription_expiring' ||
    type.includes('subscription')
  ) {
    return userRoutes.subscriptions
  }

  if (
    type === 'balance' ||
    type === 'deposit' ||
    type === 'redeem' ||
    type.includes('balance') ||
    type.includes('deposit')
  ) {
    return type.includes('redeem') || blob.includes('兑换')
      ? userRoutes.financeTopup
      : userRoutes.financeHistory
  }

  if (type === 'announcement' || type === 'system') {
    // Heuristic fallback when type is generic but copy mentions a domain
    if (blob.includes('工单') || blob.includes('ticket')) {
      return id != null && id > 0 ? userTicketPath(id) : userRoutes.tickets
    }
    if (blob.includes('退款') || blob.includes('退订')) return userRoutes.refunds
    if (blob.includes('充值') || blob.includes('订单')) return userRoutes.orders
    if (blob.includes('订阅')) return userRoutes.subscriptions
    if (blob.includes('余额') || blob.includes('兑换')) return userRoutes.finance
    return userRoutes.dashboard
  }

  return userRoutes.dashboard
}

/** Relative Chinese time for notification list. */
export function formatRelativeTime(iso: string, now = Date.now()): string {
  const t = new Date(iso).getTime()
  if (Number.isNaN(t)) return ''
  const diffSec = Math.round((now - t) / 1000)
  if (diffSec < 60) return '刚刚'
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)} 分钟前`
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)} 小时前`
  if (diffSec < 86400 * 7) return `${Math.floor(diffSec / 86400)} 天前`
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}
