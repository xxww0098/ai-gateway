import { useState } from 'react'
import { Moon, Sun } from 'lucide-react'
import { currentTheme, toggleTheme, type Theme } from '@/shared/theme'

/**
 * 亮/暗主题切换按钮：登录/注册页与首页导航共用同一实现，
 * 避免各页面手写第二份切换逻辑。
 */
export function ThemeToggleButton({ className }: { className?: string }) {
  const [theme, setTheme] = useState<Theme>(() => currentTheme())
  return (
    <button
      type="button"
      onClick={() => setTheme(toggleTheme())}
      aria-label={theme === 'dark' ? '切换到亮色模式' : '切换到暗色模式'}
      className={className ?? 'rounded-xl p-2 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'}
    >
      {theme === 'dark' ? <Sun className="h-5 w-5 text-amber-500" /> : <Moon className="h-5 w-5" />}
    </button>
  )
}
