import { Outlet } from 'react-router-dom'
import { ThemeToggleButton } from '@/shared/components/ThemeToggleButton'

export function AuthLayout() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-muted/40 p-4">
      <div className="w-full max-w-md">
        <div className="mb-4 flex justify-end">
          <ThemeToggleButton />
        </div>
        <div className="mb-8 flex shrink-0 justify-center">
          <img src="/icon.svg" alt="AI-GateWay" className="h-14 w-14 rounded-2xl" />
        </div>

        <div className="rounded-2xl border border-border bg-card p-6 sm:p-8 shadow-sm">
          <Outlet />
        </div>

        <div className="mt-8 text-center text-sm text-muted-foreground">
          &copy; {new Date().getFullYear()} AI-GateWay
        </div>
      </div>
    </div>
  )
}
