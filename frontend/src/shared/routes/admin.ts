/**
 * Canonical admin panel paths (all under /admin/*).
 * Legacy paths redirect here via App.tsx — do not link to old paths in new code.
 */
export const adminRoutes = {
  users: '/admin/users',
  channels: '/admin/channels',
  billing: '/admin/billing',
  usageLogs: '/admin/usage-logs',
  /** Unified host: payment orders + refunds */
  commerce: '/admin/commerce',
  /** @deprecated use commerce + tab; kept for redirects */
  orders: '/admin/orders',
  /** @deprecated use commerce + tab; kept for redirects */
  refunds: '/admin/refunds',
  tickets: '/admin/tickets',
  settings: '/admin/settings',
  auditLogs: '/admin/audit-logs',
} as const

export type AdminRoutePath = (typeof adminRoutes)[keyof typeof adminRoutes]

export function adminTicketPath(id: number | string): string {
  return `${adminRoutes.tickets}/${id}`
}

export function adminCommerceTab(tab: 'orders' | 'refunds'): string {
  return `${adminRoutes.commerce}?tab=${tab}`
}

export function adminBillingTab(tab: 'pricing' | 'redeem' | 'subscriptions'): string {
  return `${adminRoutes.billing}?tab=${tab}`
}

export function adminSettingsTab(
  tab: 'config' | 'logs' | 'announcements' | 'ticket-replies' | 'payment'
): string {
  return `${adminRoutes.settings}?tab=${tab}`
}

export function adminChannelsTab(
  tab: 'providers' | 'oauth' | 'credentials' | 'ampcode'
): string {
  return `${adminRoutes.channels}?tab=${tab}`
}

/** True when pathname is an admin panel route (canonical or still-supported legacy). */
export function isAdminPanelPath(pathname: string): boolean {
  if (pathname === '/admin' || pathname.startsWith('/admin/')) return true
  // Legacy mounts kept only for client-side redirects
  return (
    pathname === '/users' ||
    pathname === '/channels' ||
    pathname.startsWith('/channels/') ||
    pathname === '/billing' ||
    pathname.startsWith('/billing/') ||
    pathname === '/usage-logs' ||
    pathname === '/settings' ||
    pathname.startsWith('/settings/') ||
    pathname === '/order-management' ||
    pathname === '/payment-config'
  )
}
