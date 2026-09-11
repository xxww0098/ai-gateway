import { memo, useCallback, useState } from 'react'
import { Bell, Menu, Wallet } from 'lucide-react'
import { cn } from '@/shared/utils/utils'
import { useNavigate } from 'react-router-dom'
import { useAppStore } from '@/shared/store/app_store'
import { useAuthStore } from '@/features/auth/auth_store'
import { useProfile } from '@/features/auth/hooks'
import {
  useHeaderNotifications,
  type NotificationItem,
} from '@/features/user-notifications/hooks'
import { Button } from '@/shared/components/ui/button'
import { userRoutes } from '@/shared/routes/user'
import { formatRelativeTime, pathForNotification } from '@/shared/utils/notificationNav'
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuItem,
} from '@/shared/components/ui/dropdown-menu'

export const Header = memo(function Header() {
  const navigate = useNavigate()
  const setMobileOpen = useAppStore(s => s.setMobileOpen)
  const user = useAuthStore(s => s.user)

  // Subscribe directly to the profile query so the displayed balance reacts
  // immediately whenever any feature invalidates `queryKeys.auth.profile()`
  // (subscription purchase, redeem, top-up, etc.) or the 30s stale window
  // / window-focus refetch fires. Falls back to the auth store cache when
  // the profile query hasn't loaded yet.
  const { data: profile } = useProfile()
  const displayBalance =
    profile?.available_balance ?? (typeof user?.balance === 'number' ? user.balance : undefined)

  const [open, setOpen] = useState(false)
  const { items, unreadCount, loading, markRead, markAllRead } = useHeaderNotifications(open)

  const openNotification = useCallback(
    (item: NotificationItem) => {
      if (!item.is_read) markRead(item.id)
      setOpen(false)
      navigate(pathForNotification(item))
    },
    [navigate, markRead],
  )

  return (
    <header
      className="sticky top-0 z-20 flex h-14 items-center justify-between border-b border-border bg-background/80 px-3 backdrop-blur-xl sm:h-16 sm:px-4 lg:px-8"
      style={{ paddingTop: 'env(safe-area-inset-top, 0px)' }}
    >
      <div className="flex min-w-0 items-center gap-2">
        <button
          type="button"
          onClick={() => setMobileOpen(true)}
          className="lg:hidden -ml-1 flex h-11 w-11 items-center justify-center rounded-xl text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          aria-label="打开菜单"
        >
          <Menu className="h-5 w-5" />
        </button>

        <div className="flex min-w-0 items-center gap-2 lg:hidden">
          <img src="/icon.svg" alt="AI-GateWay" className="h-7 w-7 shrink-0 rounded-lg" />
          <span className="hidden max-w-[8rem] truncate text-sm font-semibold text-foreground xs:inline">
            控制台
          </span>
        </div>
      </div>

      <div className="flex items-center gap-1.5 sm:gap-3">
        {user && (
          <div className="flex items-center gap-1.5 sm:gap-3">
            {/* Notification Bell */}
            <DropdownMenu open={open} onOpenChange={setOpen}>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="relative rounded-xl h-11 w-11"
                  aria-label={unreadCount > 0 ? `通知，${unreadCount} 条未读` : '通知'}
                >
                  <Bell className="h-5 w-5 text-muted-foreground" />
                  {unreadCount > 0 && (
                    <span className="absolute top-1.5 right-1.5 min-w-[16px] h-4 px-1 rounded-full bg-red-500 text-white text-[10px] font-bold flex items-center justify-center leading-none">
                      {unreadCount > 99 ? '99+' : unreadCount}
                    </span>
                  )}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent
                align="end"
                className="w-[min(20rem,calc(100vw-1.5rem))] max-h-[min(70vh,420px)] overflow-y-auto"
              >
                <DropdownMenuLabel className="flex items-center justify-between sticky top-0 bg-popover z-10">
                  <span>通知</span>
                  {unreadCount > 0 && (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-auto py-0.5 px-2 text-xs font-normal text-primary hover:text-primary"
                      onPointerDown={(e) => e.preventDefault()}
                      onClick={(e) => {
                        e.preventDefault()
                        markAllRead()
                      }}
                    >
                      全部已读
                    </Button>
                  )}
                </DropdownMenuLabel>
                <DropdownMenuSeparator />

                {loading ? (
                  <div className="space-y-2 px-2 py-3" role="status" aria-label="加载通知">
                    <div className="h-10 rounded-lg bg-muted animate-pulse" />
                    <div className="h-10 rounded-lg bg-muted/70 animate-pulse" />
                  </div>
                ) : items.length === 0 ? (
                  <div className="px-3 py-8 text-center">
                    <p className="text-sm font-medium text-foreground">还没有通知</p>
                    <p className="mt-1 text-xs text-muted-foreground">充值到账、工单回复会出现在这里。</p>
                  </div>
                ) : (
                  items.map(item => (
                    <DropdownMenuItem
                      key={item.id}
                      onSelect={(e) => {
                        e.preventDefault()
                        openNotification(item)
                      }}
                      className={cn(
                        'flex flex-col items-start gap-1 cursor-pointer py-2.5',
                        !item.is_read && 'bg-accent/50'
                      )}
                    >
                      <div className="flex items-start justify-between w-full gap-2">
                        <span
                          className={cn(
                            'text-sm font-medium line-clamp-1',
                            !item.is_read ? 'text-foreground' : 'text-muted-foreground'
                          )}
                        >
                          {!item.is_read && (
                            <span
                              className="inline-block w-1.5 h-1.5 rounded-full bg-primary-500 mr-1.5 align-middle"
                              aria-hidden
                            />
                          )}
                          {item.title}
                        </span>
                        <span className="text-[10px] text-muted-foreground shrink-0 tabular-nums pt-0.5">
                          {formatRelativeTime(item.created_at)}
                        </span>
                      </div>
                      <span className="text-xs text-muted-foreground line-clamp-2 w-full">
                        {item.content}
                      </span>
                      {!item.is_read && (
                        <button
                          type="button"
                          className="text-[11px] text-primary-700 hover:underline dark:text-primary-400"
                          onPointerDown={(e) => e.preventDefault()}
                          onClick={(e) => {
                            e.preventDefault()
                            e.stopPropagation()
                            markRead(item.id)
                          }}
                        >
                          仅标记已读
                        </button>
                      )}
                    </DropdownMenuItem>
                  ))
                )}
              </DropdownMenuContent>
            </DropdownMenu>

            {/* Balance — always visible; compact on mobile */}
            {displayBalance !== undefined && (
              <button
                type="button"
                onClick={() => navigate(userRoutes.financeTopup)}
                title="充值 / 查看财务"
                className="flex min-h-11 cursor-pointer items-center gap-1.5 rounded-xl border border-border bg-card px-2.5 py-1.5 shadow-2xs transition-all duration-150 hover:bg-muted/60 hover:border-border/80 active:scale-[0.98] sm:gap-2 sm:px-3"
              >
                <Wallet className="hidden h-4 w-4 shrink-0 text-emerald-600 dark:text-emerald-400 sm:block" />
                <span className="text-xs font-bold tracking-tight tabular-nums text-foreground sm:text-sm">
                  ${Number(displayBalance).toFixed(displayBalance < 1 ? 4 : 2)}
                </span>
              </button>
            )}
          </div>
        )}
      </div>
    </header>
  )
})
