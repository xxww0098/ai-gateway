/**
 * Canonical user panel paths.
 * Legacy paths (balance/redeem/recharge/refund/apply) redirect via App.tsx.
 */
export const userRoutes = {
  dashboard: '/dashboard',
  keys: '/keys',
  models: '/models',
  subscriptions: '/subscriptions',
  orders: '/orders',
  usage: '/usage',
  finance: '/finance',
  financeTopup: '/finance?tab=topup',
  financeHistory: '/finance?tab=history',
  tickets: '/tickets',
  refunds: '/refunds',
  refundApply: '/refunds/apply',
} as const

export type UserRoutePath = (typeof userRoutes)[keyof typeof userRoutes]

export function userTicketPath(id: number | string): string {
  return `${userRoutes.tickets}/${id}`
}

export function userRefundApplyPath(subscriptionId?: number | string): string {
  if (subscriptionId == null || subscriptionId === '') return userRoutes.refundApply
  return `${userRoutes.refundApply}?subscription_id=${subscriptionId}`
}
