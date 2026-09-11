import { Outlet, Navigate, useLocation } from 'react-router-dom'
import { useEffect } from 'react'
import { Sidebar } from '@/shared/components/layout/Sidebar'
import { Header } from '@/shared/components/layout/Header'
import { MobileBottomNav } from '@/shared/components/layout/MobileBottomNav'
import { useAuthStore } from '@/features/auth/auth_store'
import { useProfile } from '@/features/auth/hooks'
import { useAppStore } from '@/shared/store/app_store'
import { isAdminPanelPath } from '@/shared/routes/admin'
import { isStaff } from '@/shared/role_core'

export default function UserLayout() {
  const token = useAuthStore(s => s.token)
  const hasUser = useAuthStore(s => !!s.user)
  const userId = useAuthStore(s => s.user?.id)
  const userEmail = useAuthStore(s => s.user?.email)
  const userRole = useAuthStore(s => s.user?.role)
  const updateUser = useAuthStore(s => s.updateUser)
  const sidebarCollapsed = useAppStore(s => s.sidebarCollapsed)
  const location = useLocation()

  const { data: profileData } = useProfile()

  useEffect(() => {
    const next = profileData?.user
    if (!next) return
    if (next.id === userId && next.email === userEmail && next.role === userRole) return
    updateUser({ id: next.id, email: next.email, role: next.role })
  }, [profileData?.user, userId, userEmail, userRole, updateUser])

  // Close mobile drawer on route change (back/forward / deep links)
  const setMobileOpen = useAppStore(s => s.setMobileOpen)
  useEffect(() => {
    setMobileOpen(false)
  }, [location.pathname, setMobileOpen])

  if (!token || !hasUser) {
    return <Navigate to="/login" replace />
  }

  if (isAdminPanelPath(location.pathname) && !isStaff(userRole)) {
    return <AdminRouteForbidden />
  }

  return (
    <div className="flex min-h-screen bg-muted/40 font-sans">
      <Sidebar />

      <div
        className={`flex-1 flex flex-col min-w-0 transition-all duration-300 ${
          sidebarCollapsed ? 'lg:pl-[72px]' : 'lg:pl-48'
        }`}
      >
        <Header />

        <main
          className="flex-1 overflow-x-hidden p-3 sm:p-4 md:p-6 lg:p-8 pb-[calc(3.5rem+env(safe-area-inset-bottom,0px)+0.75rem)] lg:pb-8"
        >
          <div className="mx-auto w-full max-w-7xl">
            <Outlet />
          </div>
        </main>
      </div>

      <MobileBottomNav />
    </div>
  )
}

function AdminRouteForbidden() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-muted/40 p-6">
      <section className="w-full max-w-md rounded-xl border border-border bg-card p-8 text-center">
        <h1 className="text-xl font-semibold text-foreground">无权访问管理页面</h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          当前账号没有管理员权限，请切换到管理员账号后再访问该页面。
        </p>
      </section>
    </div>
  )
}
