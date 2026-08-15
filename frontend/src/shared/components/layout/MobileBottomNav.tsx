import { Link, useLocation } from 'react-router-dom'
import {
  LayoutDashboard,
  Key,
  Wallet,
  Ticket,
  MoreHorizontal,
  Users,
  Network,
  CreditCard,
  ClipboardList,
  BarChart3,
} from 'lucide-react'
import { cn } from '@/shared/utils/utils'
import { userRoutes } from '@/shared/routes/user'
import { adminRoutes } from '@/shared/routes/admin'
import { useAppStore } from '@/shared/store/app_store'
import { useAuthStore } from '@/features/auth/auth_store'
import { isAdminPanelPath } from '@/shared/routes/admin'

type NavItem = {
  label: string
  path: string
  icon: typeof LayoutDashboard
  matchPath?: string
}

const userPrimary: NavItem[] = [
  { label: '总览', path: userRoutes.dashboard, icon: LayoutDashboard },
  { label: '密钥', path: userRoutes.keys, icon: Key },
  { label: '充值', path: userRoutes.financeTopup, icon: Wallet, matchPath: userRoutes.finance },
  { label: '工单', path: userRoutes.tickets, icon: Ticket },
]

const adminPrimary: NavItem[] = [
  { label: '用户', path: adminRoutes.users, icon: Users },
  { label: '渠道', path: adminRoutes.channels, icon: Network },
  { label: '计费', path: adminRoutes.billing, icon: CreditCard },
  { label: '交易', path: adminRoutes.commerce, icon: ClipboardList },
  { label: '工单', path: adminRoutes.tickets, icon: Ticket },
]

function pathActive(pathname: string, item: NavItem): boolean {
  const base = item.matchPath ?? item.path.split('?')[0]
  if (base === userRoutes.dashboard) {
    return pathname === userRoutes.dashboard || pathname === '/'
  }
  if (base === userRoutes.finance) {
    return (
      pathname === userRoutes.finance ||
      pathname === '/balance' ||
      pathname === '/redeem' ||
      pathname === '/recharge'
    )
  }
  if (base === userRoutes.tickets) {
    return pathname === userRoutes.tickets || pathname.startsWith(`${userRoutes.tickets}/`)
  }
  if (base === adminRoutes.commerce) {
    return (
      pathname === adminRoutes.commerce ||
      pathname === adminRoutes.orders ||
      pathname === adminRoutes.refunds ||
      pathname === '/order-management'
    )
  }
  if (base === adminRoutes.tickets) {
    return pathname === adminRoutes.tickets || pathname.startsWith(`${adminRoutes.tickets}/`)
  }
  if (base === adminRoutes.billing) {
    return (
      pathname === adminRoutes.billing ||
      pathname === '/billing' ||
      pathname === '/admin/pricing' ||
      pathname === '/admin/redeem-codes' ||
      pathname === '/admin/subscriptions'
    )
  }
  if (base === adminRoutes.channels) {
    return pathname === adminRoutes.channels || pathname.startsWith(`${adminRoutes.channels}/`)
  }
  return pathname === base || pathname.startsWith(`${base}/`)
}

/**
 * Thumb-zone primary nav (&lt;lg).
 * User: 4 keys + 更多; Admin on /admin/*: 5 ops + 更多.
 */
export function MobileBottomNav() {
  const location = useLocation()
  const setMobileOpen = useAppStore((s) => s.setMobileOpen)
  const role = useAuthStore((s) => s.user?.role)
  const onAdminSurface = role === 'admin' && isAdminPanelPath(location.pathname)
  const items = onAdminSurface ? adminPrimary : userPrimary
  const cols = onAdminSurface ? 6 : 5

  return (
    <nav
      className="fixed bottom-0 inset-x-0 z-30 border-t border-border bg-white/95 dark:bg-dark-900/95 backdrop-blur-xl lg:hidden"
      style={{ paddingBottom: 'env(safe-area-inset-bottom, 0px)' }}
      aria-label={onAdminSurface ? '管理主导航' : '主导航'}
    >
      <ul
        className="mx-auto grid h-14 max-w-lg"
        style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
      >
        {items.map((item) => {
          const active = pathActive(location.pathname, item)
          const Icon = item.icon
          return (
            <li key={item.path} className="min-w-0">
              <Link
                to={item.path}
                className={cn(
                  'flex h-full min-h-[44px] flex-col items-center justify-center gap-0.5 px-0.5 text-[10px] font-medium transition-colors',
                  active
                    ? 'text-primary-600 dark:text-primary-400'
                    : 'text-gray-500 dark:text-dark-400 active:text-gray-800 dark:active:text-gray-200'
                )}
              >
                <Icon className={cn('h-5 w-5', active && 'stroke-[2.25]')} aria-hidden />
                <span className="truncate max-w-full">{item.label}</span>
              </Link>
            </li>
          )
        })}
        <li className="min-w-0">
          <button
            type="button"
            onClick={() => setMobileOpen(true)}
            className="flex h-full min-h-[44px] w-full flex-col items-center justify-center gap-0.5 px-0.5 text-[10px] font-medium text-gray-500 dark:text-dark-400 active:text-gray-800 dark:active:text-gray-200"
          >
            {onAdminSurface ? (
              <BarChart3 className="h-5 w-5" aria-hidden />
            ) : (
              <MoreHorizontal className="h-5 w-5" aria-hidden />
            )}
            <span>更多</span>
          </button>
        </li>
      </ul>
    </nav>
  )
}
