import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQueryClient } from '@tanstack/react-query'
import {
  BookOpen,
  ChevronsUpDown,
  Key,
  LogOut,
  Moon,
  PanelLeft,
  PanelLeftClose,
  ShieldCheck,
  Sun,
  Ticket,
  User,
  Wallet,
} from 'lucide-react'
import { useAuthStore } from '@/features/auth/auth_store'
import { logout as serverLogout } from '@/features/auth/api'
import { useAppStore } from '@/shared/store/app_store'
import { cn } from '@/shared/utils/utils'
import { currentTheme, toggleTheme, type Theme } from '@/shared/theme'
import { docsRoutes } from '@/shared/routes/docs'
import { userRoutes } from '@/shared/routes/user'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/shared/components/ui/dropdown-menu'

type SidebarAccountMenuProps = {
  collapsed: boolean
  onNavigate: () => void
}

export function SidebarAccountMenu({ collapsed, onNavigate }: SidebarAccountMenuProps) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const user = useAuthStore((s) => s.user)
  const logout = useAuthStore((s) => s.logout)
  const sidebarCollapsed = useAppStore((s) => s.sidebarCollapsed)
  const toggleSidebar = useAppStore((s) => s.toggleSidebar)
  const [theme, setTheme] = useState<Theme>(() => currentTheme())

  if (!user) return null

  const isAdmin = user.role === 'admin'
  const roleLabel = isAdmin ? '管理员' : '用户'

  const go = (path: string) => {
    onNavigate()
    navigate(path)
  }

  const handleLogout = async () => {
    try {
      await serverLogout()
    } catch {
      /* ignore */
    }
    logout()
    queryClient.clear()
    onNavigate()
    navigate('/login')
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          title={user.email}
          aria-label="个人中心"
          className={cn(
            'flex w-full min-h-11 items-center gap-2.5 rounded-xl px-2.5 py-2 text-left transition-colors',
            'text-gray-700 dark:text-dark-200 hover:bg-gray-50 dark:hover:bg-dark-800',
            'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-2',
            collapsed && 'lg:justify-center lg:px-1.5'
          )}
        >
          <span
            className={cn(
              'flex h-8 w-8 shrink-0 items-center justify-center rounded-full shadow-sm',
              isAdmin
                ? 'bg-emerald-100 text-emerald-600 dark:bg-emerald-900/30 dark:text-emerald-400'
                : 'bg-primary-100 text-primary-600 dark:bg-primary-900/30 dark:text-primary-400'
            )}
            aria-hidden
          >
            {isAdmin ? <ShieldCheck className="h-4 w-4" /> : <User className="h-4 w-4" />}
          </span>
          <span className={cn('min-w-0 flex-1', collapsed && 'lg:hidden')}>
            <span className="block truncate text-sm font-medium text-gray-900 dark:text-white">
              {user.email}
            </span>
            <span className="block truncate text-[11px] text-gray-400 dark:text-dark-400">
              {roleLabel}
            </span>
          </span>
          <ChevronsUpDown
            className={cn('h-4 w-4 shrink-0 text-gray-400 dark:text-dark-500', collapsed && 'lg:hidden')}
            aria-hidden
          />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        side="top"
        align="start"
        sideOffset={8}
        className="w-60 rounded-xl p-1.5"
      >
        <DropdownMenuLabel className="font-normal space-y-1">
          <p className="text-sm font-medium text-foreground break-all">{user.email}</p>
          <p className="text-xs text-muted-foreground">{roleLabel}</p>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="cursor-pointer rounded-lg" onSelect={() => go(userRoutes.finance)}>
          <Wallet />
          财务中心
        </DropdownMenuItem>
        <DropdownMenuItem className="cursor-pointer rounded-lg" onSelect={() => go(userRoutes.tickets)}>
          <Ticket />
          我的工单
        </DropdownMenuItem>
        <DropdownMenuItem className="cursor-pointer rounded-lg" onSelect={() => go(userRoutes.keys)}>
          <Key />
          API 密钥
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="cursor-pointer rounded-lg" onSelect={() => go(docsRoutes.root)}>
          <BookOpen />
          接入指南
        </DropdownMenuItem>
        <DropdownMenuItem
          className="cursor-pointer rounded-lg"
          onSelect={() => setTheme(toggleTheme())}
        >
          {theme === 'dark' ? <Sun className="text-amber-500" /> : <Moon />}
          {theme === 'dark' ? '亮色模式' : '暗色模式'}
        </DropdownMenuItem>
        <DropdownMenuItem
          className="hidden cursor-pointer rounded-lg lg:flex"
          onSelect={() => toggleSidebar()}
        >
          {sidebarCollapsed ? <PanelLeft /> : <PanelLeftClose />}
          {sidebarCollapsed ? '展开侧边栏' : '收起侧边栏'}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="cursor-pointer rounded-lg text-red-600 focus:bg-red-50 focus:text-red-600 dark:text-red-400 dark:focus:bg-red-900/20 dark:focus:text-red-400"
          onSelect={() => {
            void handleLogout()
          }}
        >
          <LogOut />
          退出登录
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
