import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQueryClient } from '@tanstack/react-query'
import { LogOut, Moon, ShieldCheck, Sun, User } from 'lucide-react'
import { useAuthStore } from '@/features/auth/auth_store'
import { logout as serverLogout } from '@/features/auth/api'
import { cn } from '@/shared/utils/utils'
import { currentTheme, toggleTheme, type Theme } from '@/shared/theme'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/shared/components/ui/dropdown-menu'

/**
 * Account control for the header's top-right corner.
 *
 * Deliberately holds only what the main nav can't: the identity it belongs to,
 * the theme switch, and sign-out. 财务 / 工单 / 密钥 live in the sidebar nav and
 * 接入指南 on the public home page — duplicating them here just split the entry
 * points in two.
 */
export function AccountMenu() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const user = useAuthStore((s) => s.user)
  const logout = useAuthStore((s) => s.logout)
  const [theme, setTheme] = useState<Theme>(() => currentTheme())

  if (!user) return null

  const isAdmin = user.role === 'admin'
  const roleLabel = isAdmin ? '管理员' : '用户'

  const handleLogout = async () => {
    try {
      await serverLogout()
    } catch {
      /* ignore */
    }
    logout()
    queryClient.clear()
    void navigate('/login')
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          title={user.email}
          aria-label="个人中心"
          className={cn(
            'h-11 w-11 rounded-full flex items-center justify-center shadow-sm transition-colors',
            'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2',
            isAdmin
              ? 'bg-emerald-100 dark:bg-emerald-900/30 border border-emerald-200 dark:border-emerald-800 text-emerald-600 dark:text-emerald-400 hover:bg-emerald-200/80 dark:hover:bg-emerald-900/50 focus-visible:ring-emerald-500'
              : 'bg-primary-100 dark:bg-primary-900/30 border border-primary-200 dark:border-primary-800 text-primary-600 dark:text-primary-400 hover:bg-primary-200/80 dark:hover:bg-primary-900/50 focus-visible:ring-primary-500'
          )}
        >
          {isAdmin ? (
            <ShieldCheck className="w-4 h-4" aria-hidden />
          ) : (
            <User className="w-4 h-4" aria-hidden />
          )}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        side="bottom"
        align="end"
        sideOffset={8}
        className="w-56 rounded-xl p-1.5"
      >
        <DropdownMenuLabel className="font-normal space-y-1">
          <div className="flex items-center gap-2">
            {isAdmin ? (
              <ShieldCheck className="h-4 w-4 text-emerald-600 dark:text-emerald-400 shrink-0" aria-hidden />
            ) : (
              <User className="h-4 w-4 text-primary-600 dark:text-primary-400 shrink-0" aria-hidden />
            )}
            <p className="text-xs font-semibold text-muted-foreground">{roleLabel}</p>
          </div>
          <p className="text-sm font-medium text-foreground break-all">{user.email}</p>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="cursor-pointer rounded-lg"
          onSelect={() => {
            setTheme(toggleTheme())
          }}
        >
          {theme === 'dark' ? <Sun className="text-amber-500" /> : <Moon />}
          {theme === 'dark' ? '亮色模式' : '暗色模式'}
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
