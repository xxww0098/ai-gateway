import { useState, type ReactNode } from 'react'
import { Link, NavLink } from 'react-router-dom'
import { BookOpen, Moon, Sun } from 'lucide-react'
import { useAuthStore } from '@/features/auth/auth_store'
import { currentTheme, toggleTheme, type Theme } from '@/shared/theme'
import { userRoutes } from '@/shared/routes/user'
import { cn } from '@/shared/utils/utils'
import { PRODUCT_NAME, docsNav, navHref } from './guide'

type DocsLayoutProps = {
  children: ReactNode
}

export function DocsLayout({ children }: DocsLayoutProps) {
  const token = useAuthStore((s) => s.token)
  const isAuthenticated = Boolean(token)
  const [theme, setTheme] = useState<Theme>(() => currentTheme())

  return (
    <div className="min-h-screen bg-gradient-to-br from-gray-50 via-primary-50/20 to-gray-100 dark:from-dark-950 dark:via-dark-900 dark:to-dark-950">
      <header className="sticky top-0 z-30 border-b border-border bg-white/80 backdrop-blur-xl dark:bg-dark-900/80">
        <div className="mx-auto flex h-14 max-w-6xl items-center justify-between gap-4 px-4 sm:h-16 sm:px-6">
          <div className="flex min-w-0 items-center gap-6">
            <Link to="/" className="flex items-center gap-2.5 shrink-0">
              <img src="/icon.svg" alt={PRODUCT_NAME} className="h-8 w-8 rounded-xl" />
              <span className="text-base font-bold text-gray-900 dark:text-white sm:text-lg">
                {PRODUCT_NAME}
              </span>
            </Link>
            <nav className="hidden items-center gap-1 sm:flex" aria-label="文档分区">
              <span className="inline-flex items-center gap-1.5 rounded-full bg-primary-50 px-3 py-1 text-xs font-semibold text-primary-700 dark:bg-primary-900/30 dark:text-primary-300">
                <BookOpen className="h-3.5 w-3.5" />
                客户端接入
              </span>
            </nav>
          </div>

          <div className="flex items-center gap-2 sm:gap-3">
            <button
              type="button"
              onClick={() => {
                setTheme(toggleTheme())
              }}
              aria-label={theme === 'dark' ? '切换到亮色模式' : '切换到暗色模式'}
              className="rounded-xl p-2 text-gray-500 transition-colors hover:bg-gray-100 dark:text-dark-400 dark:hover:bg-dark-800"
            >
              {theme === 'dark' ? <Sun className="h-5 w-5 text-amber-500" /> : <Moon className="h-5 w-5" />}
            </button>
            {isAuthenticated ? (
              <Link to={userRoutes.dashboard} className="btn btn-primary btn-sm rounded-full px-4">
                控制台
              </Link>
            ) : (
              <Link
                to="/login"
                className="text-sm font-medium text-gray-600 transition-colors hover:text-gray-900 dark:text-gray-300 dark:hover:text-white"
              >
                登录
              </Link>
            )}
          </div>
        </div>
      </header>

      <div className="mx-auto flex max-w-6xl gap-8 px-4 py-6 sm:px-6 lg:py-10">
        <aside className="hidden w-52 shrink-0 md:block">
          <nav className="sticky top-24 space-y-6" aria-label="接入指南目录">
            {docsNav.map((group) => (
              <div key={group.title}>
                <div className="mb-2 px-2 text-[11px] font-semibold uppercase tracking-widest text-gray-400 dark:text-dark-400">
                  {group.title}
                </div>
                <ul className="space-y-0.5">
                  {group.items.map((item) => (
                    <li key={navHref(item)}>
                      <NavLink
                        to={navHref(item)}
                        end={item.slug === undefined}
                        className={({ isActive }) =>
                          cn(
                            'block rounded-xl px-2.5 py-2 text-sm font-medium transition-colors',
                            isActive
                              ? 'bg-primary-50 text-primary-700 dark:bg-primary-900/20 dark:text-primary-300'
                              : 'text-gray-600 hover:bg-gray-100 hover:text-gray-900 dark:text-dark-300 dark:hover:bg-dark-800 dark:hover:text-white',
                          )
                        }
                      >
                        {item.label}
                      </NavLink>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </nav>
        </aside>

        <div className="min-w-0 flex-1">
          <nav
            className="mb-6 flex gap-2 overflow-x-auto pb-1 md:hidden"
            aria-label="接入指南目录"
          >
            {docsNav.flatMap((group) =>
              group.items.map((item) => (
                <NavLink
                  key={navHref(item)}
                  to={navHref(item)}
                  end={item.slug === undefined}
                  className={({ isActive }) =>
                    cn(
                      'shrink-0 rounded-full px-3 py-1.5 text-xs font-semibold transition-colors',
                      isActive
                        ? 'bg-primary-600 text-white'
                        : 'bg-white text-gray-600 ring-1 ring-border dark:bg-dark-800 dark:text-dark-300',
                    )
                  }
                >
                  {item.label}
                </NavLink>
              )),
            )}
          </nav>
          {children}
        </div>
      </div>
    </div>
  )
}
