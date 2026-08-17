import { Link, useLocation, useNavigate } from 'react-router-dom'
import { useAuthStore } from '@/features/auth/auth_store'
import { logout as serverLogout } from '@/features/auth/api'
import { useAppStore } from '@/shared/store/app_store'
import {
  LayoutDashboard, Key, LogOut,
  Settings, PanelLeftClose, PanelLeft,
  Sun, Moon, Cpu, FileBarChart, Crown,
  ShoppingCart, Ticket, Wallet,
  Users, Network, CreditCard, BarChart3,
  ClipboardList, ShieldAlert, BookOpen,
} from 'lucide-react'
import { useEffect, useState } from 'react'
import type { LucideIcon } from 'lucide-react'
import { cn } from '@/shared/utils/utils'
import { currentTheme, toggleTheme, type Theme } from '@/shared/theme'
import { adminRoutes } from '@/shared/routes/admin'
import { userRoutes } from '@/shared/routes/user'
import { docsRoutes } from '@/shared/routes/docs'

type NavLinkItem = {
  label: string
  path: string
  icon: LucideIcon
  /** 侧栏收起时 `title` 提示（缩写项用完整说法） */
  hint?: string
}

export function Sidebar() {
  const logout = useAuthStore(s => s.logout)
  const user = useAuthStore(s => s.user)
  const sidebarCollapsed = useAppStore(s => s.sidebarCollapsed)
  const mobileOpen = useAppStore(s => s.mobileOpen)
  const setMobileOpen = useAppStore(s => s.setMobileOpen)
  const toggleSidebar = useAppStore(s => s.toggleSidebar)
  const location = useLocation()
  const navigate = useNavigate()
  
  const [theme, setTheme] = useState<Theme>(() => currentTheme())
  const handleToggleTheme = () => setTheme(toggleTheme())

  // Lock body scroll while the mobile drawer is open
  useEffect(() => {
    if (!mobileOpen) return
    const prev = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    return () => {
      document.body.style.overflow = prev
    }
  }, [mobileOpen])

  // Escape closes the mobile drawer
  useEffect(() => {
    if (!mobileOpen) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMobileOpen(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [mobileOpen, setMobileOpen])

  // User nav: 8 items — finance merges 流水+兑换; 退款降级为二级入口
  const panelUserNavs: NavLinkItem[] = [
    { label: '总览', path: userRoutes.dashboard, icon: LayoutDashboard },
    { label: '密钥', path: userRoutes.keys, icon: Key, hint: 'API 密钥' },
    { label: '模型', path: userRoutes.models, icon: Cpu },
    { label: '订阅', path: userRoutes.subscriptions, icon: Crown },
    { label: '订单', path: userRoutes.orders, icon: ShoppingCart, hint: '充值订单' },
    { label: '用量', path: userRoutes.usage, icon: FileBarChart, hint: '使用明细' },
    { label: '财务', path: userRoutes.finance, icon: Wallet, hint: '充值与流水' },
    { label: '工单', path: userRoutes.tickets, icon: Ticket },
  ]

  // Admin nav: 8 items — commerce merges 支付订单 + 退款审核
  const adminNavs: NavLinkItem[] = user?.role === 'admin'
    ? [
        { label: '用户', path: adminRoutes.users, icon: Users, hint: '用户管理' },
        { label: '渠道', path: adminRoutes.channels, icon: Network, hint: '渠道管理' },
        { label: '计费', path: adminRoutes.billing, icon: CreditCard, hint: '倍率 / 卡密 / 订阅' },
        { label: '用量日志', path: adminRoutes.usageLogs, icon: BarChart3, hint: '全站用量' },
        { label: '交易', path: adminRoutes.commerce, icon: ClipboardList, hint: '支付订单 / 退款' },
        { label: '工单处理', path: adminRoutes.tickets, icon: Ticket, hint: '工单处理' },
        { label: '系统', path: adminRoutes.settings, icon: Settings, hint: '配置 / 日志 / 支付' },
        { label: '审计', path: adminRoutes.auditLogs, icon: ShieldAlert, hint: '审计日志' },
      ]
    : []

  const handleLinkClick = () => {
    if (mobileOpen) setMobileOpen(false)
  }

  const isActive = (path: string) => {
    if (path === userRoutes.dashboard) {
      return location.pathname === userRoutes.dashboard || location.pathname === '/'
    }
    if (path === userRoutes.finance) {
      return (
        location.pathname === userRoutes.finance ||
        location.pathname === '/balance' ||
        location.pathname === '/redeem' ||
        location.pathname === '/recharge'
      )
    }
    if (path === userRoutes.tickets) {
      return (
        location.pathname === userRoutes.tickets ||
        location.pathname.startsWith(`${userRoutes.tickets}/`)
      )
    }
    if (path === adminRoutes.billing) {
      return (
        location.pathname === adminRoutes.billing ||
        location.pathname === '/billing' ||
        location.pathname === '/admin/pricing' ||
        location.pathname === '/admin/redeem-codes' ||
        location.pathname === '/admin/subscriptions'
      )
    }
    if (path === adminRoutes.settings) {
      return (
        location.pathname === adminRoutes.settings ||
        location.pathname === '/settings' ||
        location.pathname === '/payment-config'
      )
    }
    if (path === adminRoutes.channels) {
      return (
        location.pathname === adminRoutes.channels ||
        location.pathname === '/channels' ||
        location.pathname.startsWith('/channels/')
      )
    }
    if (path === adminRoutes.commerce) {
      return (
        location.pathname === adminRoutes.commerce ||
        location.pathname === adminRoutes.orders ||
        location.pathname === adminRoutes.refunds ||
        location.pathname === '/order-management'
      )
    }
    if (path === adminRoutes.tickets) {
      return (
        location.pathname === adminRoutes.tickets ||
        location.pathname.startsWith(`${adminRoutes.tickets}/`)
      )
    }
    return location.pathname === path || location.pathname.startsWith(path + '/')
  }

  const navRowClass = (active: boolean) =>
    cn(
      'flex w-full items-center gap-2.5 rounded-xl px-2.5 py-2 text-sm font-medium transition-all duration-200 group',
      active
        ? 'bg-primary-50 dark:bg-primary-900/20 text-primary-600 dark:text-primary-400'
        : 'text-gray-500 dark:text-dark-400 hover:bg-gray-50 dark:hover:bg-dark-800 hover:text-gray-900 dark:hover:text-white'
    )

  const navIconClass = (active: boolean) =>
    cn(
      'h-[18px] w-[18px] flex-shrink-0 transition-colors',
      active
        ? 'text-primary-500'
        : 'text-gray-400 dark:text-dark-500 group-hover:text-gray-600 dark:group-hover:text-dark-300'
    )

  // Mobile drawer always shows labels; desktop respects sidebarCollapsed
  const showLabels = !sidebarCollapsed

  return (
    <>
      <aside
        className={cn(
          'fixed inset-y-0 left-0 z-40 flex w-[min(18rem,85vw)] flex-col border-r border-border bg-white transition-transform duration-300 dark:bg-dark-900 lg:w-48',
          // Desktop collapse only at lg+
          sidebarCollapsed && 'lg:w-[72px]',
          !mobileOpen ? '-translate-x-full lg:translate-x-0' : 'translate-x-0'
        )}
        style={{
          paddingTop: 'env(safe-area-inset-top, 0px)',
          paddingBottom: 'env(safe-area-inset-bottom, 0px)',
        }}
        aria-hidden={!mobileOpen ? undefined : false}
      >
        {/* Brand Header */}
        <div className="h-14 sm:h-16 flex items-center px-3 border-b border-border flex-shrink-0 gap-2.5 max-w-full overflow-hidden">
          <img src="/icon.svg" alt="AI-GateWay" className="w-9 h-9 rounded-xl shrink-0" />
          {/* Always show title on mobile drawer; hide only when desktop-collapsed */}
          <span
            className={cn(
              'text-base font-bold text-gray-900 dark:text-white truncate',
              sidebarCollapsed && 'lg:hidden'
            )}
          >
            AI-GateWay
          </span>
        </div>

        {/* 导航与管理在同一滚动列内；分隔线仅作分组，不把侧栏切成上下两个独立区域 */}
        <nav className="flex flex-1 flex-col min-h-0 overflow-hidden">
          <div className="flex-1 min-h-0 overflow-y-auto py-3 px-2 scrollbar-hide">
            <div className="space-y-1">
              <div
                className={cn(
                  'px-2 mb-2 text-[11px] font-semibold uppercase tracking-widest text-gray-400 dark:text-dark-400',
                  sidebarCollapsed && 'lg:hidden'
                )}
              >
                导航
              </div>
              {sidebarCollapsed && <div className="hidden lg:block h-px bg-border mx-3 mb-3" />}

              {panelUserNavs.map((item) => {
                const active = isActive(item.path)
                return (
                  <Link
                    key={item.path}
                    to={item.path}
                    onClick={handleLinkClick}
                    title={sidebarCollapsed ? (item.hint ?? item.label) : undefined}
                    className={cn(
                      navRowClass(active),
                      'min-h-11 py-2.5',
                      sidebarCollapsed && 'lg:py-2 lg:justify-center lg:min-h-0'
                    )}
                  >
                    <item.icon className={navIconClass(active)} />
                    <span className={cn('truncate', sidebarCollapsed && 'lg:hidden')}>{item.label}</span>
                  </Link>
                )
              })}
            </div>

            {adminNavs.length > 0 && (
              <>
                <div
                  className={cn(
                    'mx-3 my-2 border-t border-gray-200/80 dark:border-dark-800',
                    sidebarCollapsed && 'lg:mx-1'
                  )}
                  role="presentation"
                />
                <div className="space-y-1">
                  <div
                    className={cn(
                      'px-2 mb-2 text-[11px] font-semibold uppercase tracking-widest text-gray-400 dark:text-dark-400',
                      sidebarCollapsed && 'lg:hidden'
                    )}
                  >
                    管理
                  </div>
                  {sidebarCollapsed && <div className="hidden lg:block h-px bg-border mx-3 mb-2" />}
                  {adminNavs.map((item) => {
                    const active = isActive(item.path)
                    return (
                      <Link
                        key={item.path}
                        to={item.path}
                        onClick={handleLinkClick}
                        title={sidebarCollapsed ? (item.hint ?? item.label) : undefined}
                        className={cn(
                          navRowClass(active),
                          'min-h-11 py-2.5',
                          sidebarCollapsed && 'lg:py-2 lg:justify-center lg:min-h-0'
                        )}
                      >
                        <item.icon className={navIconClass(active)} />
                        <span className={cn('truncate', sidebarCollapsed && 'lg:hidden')}>{item.label}</span>
                      </Link>
                    )
                  })}
                </div>
              </>
            )}
          </div>
        </nav>

        {/* Footer actions */}
        <div className="p-2 border-t border-border flex flex-col gap-0.5">
          <Link
            to={docsRoutes.root}
            onClick={handleLinkClick}
            title={sidebarCollapsed ? '接入指南' : undefined}
            className="flex min-h-11 items-center gap-2.5 rounded-xl px-2.5 py-2 text-sm font-medium text-gray-500 dark:text-dark-400 hover:bg-gray-50 dark:hover:bg-dark-800 hover:text-gray-700 dark:hover:text-white transition-colors w-full"
          >
            <BookOpen className="h-[18px] w-[18px] flex-shrink-0" />
            <span className={cn(sidebarCollapsed && 'lg:hidden')}>接入指南</span>
          </Link>
          <button
            type="button"
            onClick={handleToggleTheme}
            title={sidebarCollapsed ? (theme === 'dark' ? '亮色模式' : '暗色模式') : undefined}
            className="flex min-h-11 items-center gap-2.5 rounded-xl px-2.5 py-2 text-sm font-medium text-gray-500 dark:text-dark-400 hover:bg-gray-50 dark:hover:bg-dark-800 hover:text-gray-700 dark:hover:text-white transition-colors w-full"
          >
            {theme === 'dark' ? <Sun className="h-[18px] w-[18px] text-amber-500" /> : <Moon className="h-[18px] w-[18px]" />}
            <span className={cn(sidebarCollapsed && 'lg:hidden')}>{theme === 'dark' ? '亮色' : '暗色'}</span>
          </button>

          <button
            type="button"
            onClick={toggleSidebar}
            title={sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'}
            className="hidden lg:flex items-center gap-2.5 rounded-xl px-2.5 py-2 text-sm font-medium text-gray-500 dark:text-dark-400 hover:bg-gray-50 dark:hover:bg-dark-800 hover:text-gray-700 dark:hover:text-white transition-colors w-full"
          >
            {sidebarCollapsed ? <PanelLeft className="h-[18px] w-[18px]" /> : <PanelLeftClose className="h-[18px] w-[18px]" />}
            {showLabels && <span>收起</span>}
          </button>

          <button
            type="button"
            onClick={async () => {
              try {
                await serverLogout()
              } catch {
                /* ignore */
              }
              logout()
              navigate('/login')
            }}
            title={sidebarCollapsed ? '退出登录' : undefined}
            className="flex min-h-11 items-center gap-2.5 rounded-xl px-2.5 py-2 text-sm font-medium text-gray-400 dark:text-dark-500 hover:bg-red-50 dark:hover:bg-red-900/20 hover:text-red-600 dark:hover:text-red-400 transition-colors w-full"
          >
            <LogOut className="h-[18px] w-[18px] flex-shrink-0" />
            <span className={cn(sidebarCollapsed && 'lg:hidden')}>退出</span>
          </button>
        </div>
      </aside>

      {/* Mobile Overlay */}
      {mobileOpen && (
        <div
          className="fixed inset-0 z-30 bg-black/50 backdrop-blur-sm lg:hidden transition-opacity"
          onClick={() => setMobileOpen(false)}
          aria-hidden
        />
      )}
    </>
  )
}
