import { Outlet } from 'react-router-dom'
import { useState } from 'react'
import { Moon, Sun } from 'lucide-react'
import { currentTheme, toggleTheme, type Theme } from '@/shared/theme'

export function AuthLayout() {
  const [theme, setTheme] = useState<Theme>(() => currentTheme())
  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-950 p-4">
      <button
        type="button"
        onClick={() => setTheme(toggleTheme())}
        aria-label={theme === 'dark' ? '切换到亮色模式' : '切换到暗色模式'}
        className="fixed top-4 right-4 rounded-xl p-2 text-gray-500 dark:text-dark-400 hover:bg-gray-100 dark:hover:bg-dark-800 transition-colors"
      >
        {theme === 'dark' ? <Sun className="h-5 w-5 text-amber-500" /> : <Moon className="h-5 w-5" />}
      </button>
      <div className="w-full max-w-md animate-in fade-in zoom-in-95 duration-500" style={{ willChange: 'transform, opacity' }}>
        {/* Logo or Brand */}
        <div className="flex justify-center mb-8 shrink-0">
          <img src="/icon.svg" alt="CPA Gateway" className="w-14 h-14 rounded-2xl shadow-sm" />
        </div>

        {/* Card Content Container */}
        <div className="glass-card w-full shadow-2xl p-8">
          <Outlet />
        </div>
        
        {/* Footer info inside auth layout typically */}
        <div className="mt-8 text-center text-sm text-gray-500 dark:text-dark-400">
          &copy; {new Date().getFullYear()} CPA Gateway. All rights reserved.
        </div>
      </div>
    </div>
  )
}
