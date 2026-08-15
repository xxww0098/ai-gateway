import { lazy, Suspense, type ReactNode } from 'react'
import { Navigate, Route, Routes, useLocation } from 'react-router-dom'
import { Toaster } from 'sonner'
import { ConfirmModalProvider } from '@/shared/confirm-modal'
import { ErrorBoundary } from '@/shared/components/ErrorBoundary'
import { adminRoutes, adminBillingTab, adminCommerceTab } from '@/shared/routes/admin'
import { userRoutes } from '@/shared/routes/user'
import { AuthLayout } from './pages/public/AuthLayout'
import UserLayout from './pages/user/UserLayout'

/** Client redirect that keeps ?query (e.g. tab= on billing/settings/channels). */
function RedirectWithSearch({ to }: { to: string }) {
  const { search } = useLocation()
  return <Navigate to={`${to}${search}`} replace />
}

const Home = lazy(() => import('./pages/public/HomePage'))
const Login = lazy(() => import('./pages/public/LoginPage'))
const Register = lazy(() => import('./pages/public/RegisterPage'))
const Dashboard = lazy(() => import('./pages/user/dashboard/DashboardPage'))
const Keys = lazy(() => import('./pages/user/api-keys/ApiKeysPage'))
const Models = lazy(() => import('./pages/user/models/ModelsPage'))
const Usage = lazy(() => import('./pages/user/usage/UsagePage'))
const Finance = lazy(() => import('./pages/user/billing/FinancePage'))
const Subscriptions = lazy(() => import('./pages/user/subscriptions/SubscriptionsPage'))
const Orders = lazy(() => import('./pages/user/orders/OrdersPage'))
const Refunds = lazy(() => import('./pages/user/refunds/RefundsPage'))
const RefundApply = lazy(() => import('./pages/user/refunds/RefundApplyPage'))
const Tickets = lazy(() => import('./pages/user/tickets/TicketsPage'))

const AdminUsers = lazy(() => import('./pages/admin/users/AdminUsersPage'))
const AdminChannels = lazy(() => import('./pages/admin/proxy/AdminProxyChannelsPage'))
const AdminBilling = lazy(() => import('./pages/admin/billing/AdminBillingPage'))
const AdminUsageLogs = lazy(() => import('./pages/admin/usage-logs/AdminUsageLogsPage'))
const AdminSettings = lazy(() => import('./pages/admin/settings/AdminSettingsPage'))
const AdminCommerce = lazy(() => import('./pages/admin/commerce/AdminCommercePage'))
const AdminTickets = lazy(() => import('./pages/admin/tickets/AdminTicketsPage'))
const AdminAuditLogs = lazy(() => import('./pages/admin/audit-logs/AdminAuditLogsPage'))

function PageFallback() {
  return (
    <div className="flex min-h-[240px] items-center justify-center text-sm text-muted-foreground">
      页面加载中...
    </div>
  )
}

function eb(children: ReactNode) {
  return <ErrorBoundary>{children}</ErrorBoundary>
}

function App() {
  return (
    <ConfirmModalProvider>
      <Suspense fallback={<PageFallback />}>
        <Routes>
          <Route path="/" element={eb(<Home />)} />

          <Route element={<AuthLayout />}>
            <Route path="/login" element={eb(<Login />)} />
            <Route path="/register" element={eb(<Register />)} />
          </Route>

          <Route element={<UserLayout />}>
            {/* ── User routes ── */}
            <Route path="/dashboard" element={eb(<Dashboard />)} />
            <Route path="/keys" element={eb(<Keys />)} />
            <Route path="/models" element={eb(<Models />)} />
            <Route path="/usage" element={eb(<Usage />)} />
            <Route path="/finance" element={eb(<Finance />)} />
            <Route path="/subscriptions" element={eb(<Subscriptions />)} />
            <Route path="/orders" element={eb(<Orders />)} />
            <Route path="/refunds" element={eb(<Refunds />)} />
            <Route path="/refunds/apply" element={eb(<RefundApply />)} />
            <Route path="/tickets" element={eb(<Tickets />)} />
            <Route path="/tickets/:id" element={eb(<Tickets />)} />

            {/* Legacy user finance paths */}
            <Route path="/balance" element={<Navigate to={userRoutes.financeHistory} replace />} />
            <Route path="/redeem" element={<Navigate to={userRoutes.financeTopup} replace />} />
            <Route path="/recharge" element={<Navigate to={userRoutes.financeTopup} replace />} />
            <Route path="/refund/apply" element={<RedirectWithSearch to={userRoutes.refundApply} />} />

            {/* ── Admin routes (canonical /admin/*) ── */}
            <Route path={adminRoutes.users} element={eb(<AdminUsers />)} />
            <Route path={adminRoutes.channels} element={eb(<AdminChannels />)} />
            <Route path={adminRoutes.billing} element={eb(<AdminBilling />)} />
            <Route path={adminRoutes.usageLogs} element={eb(<AdminUsageLogs />)} />
            <Route path={adminRoutes.commerce} element={eb(<AdminCommerce />)} />
            <Route path={adminRoutes.tickets} element={eb(<AdminTickets />)} />
            <Route path={`${adminRoutes.tickets}/:id`} element={eb(<AdminTickets />)} />
            <Route path={adminRoutes.settings} element={eb(<AdminSettings />)} />
            <Route path={adminRoutes.auditLogs} element={eb(<AdminAuditLogs />)} />

            {/* Legacy admin paths → canonical (bookmarks / deep links) */}
            <Route path="/users" element={<Navigate to={adminRoutes.users} replace />} />
            <Route path="/channels" element={<RedirectWithSearch to={adminRoutes.channels} />} />
            <Route path="/billing" element={<RedirectWithSearch to={adminRoutes.billing} />} />
            <Route path="/usage-logs" element={<Navigate to={adminRoutes.usageLogs} replace />} />
            <Route path="/settings" element={<RedirectWithSearch to={adminRoutes.settings} />} />
            <Route path="/order-management" element={<Navigate to={adminCommerceTab('orders')} replace />} />
            <Route path={adminRoutes.orders} element={<Navigate to={adminCommerceTab('orders')} replace />} />
            <Route path={adminRoutes.refunds} element={<Navigate to={adminCommerceTab('refunds')} replace />} />
            <Route path="/payment-config" element={<Navigate to={`${adminRoutes.settings}?tab=payment`} replace />} />
            <Route path="/admin/pricing" element={<Navigate to={adminBillingTab('pricing')} replace />} />
            <Route path="/admin/redeem-codes" element={<Navigate to={adminBillingTab('redeem')} replace />} />
            <Route path="/admin/subscriptions" element={<Navigate to={adminBillingTab('subscriptions')} replace />} />
          </Route>

          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </Suspense>
      <Toaster position="top-center" richColors theme="system" />
    </ConfirmModalProvider>
  )
}

export default App
