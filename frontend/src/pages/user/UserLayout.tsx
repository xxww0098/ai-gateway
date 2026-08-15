import { Outlet, Navigate, useLocation } from 'react-router-dom'
import { useEffect } from 'react'
import { Sidebar } from '@/shared/components/layout/Sidebar'
import { Header } from '@/shared/components/layout/Header'
import { MobileBottomNav } from '@/shared/components/layout/MobileBottomNav'
import { useAuthStore } from '@/features/auth/auth_store'
import { useProfile } from '@/features/auth/hooks'
import { useAppStore } from '@/shared/store/app_store'
import { isAdminPanelPath } from '@/shared/routes/admin'

export default function UserLayout() {
  const token = useAuthStore(s => s.token)
  const user = useAuthStore(s => s.user)
  const updateUser = useAuthStore(s => s.updateUser)
  const sidebarCollapsed = useAppStore(s => s.sidebarCollapsed)
  const location = useLocation()

  // useProfile leverages react-query caching (staleTime: 5min) and auto-refetch
  const { data: profileData } = useProfile()

  // Sync profile data back to auth store when it changes
  useEffect(() => {
    if (profileData?.user) {
      updateUser(profileData.user)
    }
  }, [profileData, updateUser])

  // Close mobile drawer on route change (back/forward / deep links)
  const setMobileOpen = useAppStore(s => s.setMobileOpen)
  useEffect(() => {
    setMobileOpen(false)
  }, [location.pathname, setMobileOpen])

  if (!token || !user) {
    return <Navigate to="/login" replace />
  }

  if (isAdminPanelPath(location.pathname) && user.role !== 'admin') {
    return <AdminRouteForbidden />
  }

  return (
    <div className="flex min-h-screen bg-gray-50 dark:bg-gray-950 font-sans">
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
          <div className="max-w-7xl mx-auto w-full animate-in fade-in slide-in-from-bottom-4 duration-500">
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
    <div className="flex min-h-screen items-center justify-center bg-gray-50 p-6 dark:bg-gray-950">
      <section className="w-full max-w-md rounded-2xl border border-border bg-card p-8 text-center shadow-sm dark:border-dark-800 dark:bg-dark-900">
        <div className="mx-auto mb-5 flex h-14 w-14 items-center justify-center rounded-full bg-red-50 text-2xl font-semibold text-red-600 dark:bg-red-950/40 dark:text-red-300">
          403
        </div>
        <h1 className="text-xl font-semibold text-gray-900 dark:text-gray-100">无权访问管理页面</h1>
        <p className="mt-3 text-sm leading-6 text-gray-500 dark:text-gray-400">
          当前账号没有管理员权限，请切换到管理员账号后再访问该页面。
        </p>
      </section>
    </div>
  )
}
