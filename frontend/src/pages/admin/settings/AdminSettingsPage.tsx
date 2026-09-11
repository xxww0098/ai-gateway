import { lazy, Suspense } from 'react'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/components/ui/tabs'
import { Settings as SettingsIcon, ScrollText, Bell, MessageSquareText, Wallet } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'

const AdminProxyConfigPage = lazy(() => import('../proxy/AdminProxyConfigPage'))
const AdminProxyLogsPage = lazy(() => import('../proxy/AdminProxyLogsPage'))
const Announcements = lazy(() => import('../announcements/AdminAnnouncementsPage'))
const AdminTicketQuickRepliesPage = lazy(() => import('./AdminTicketQuickRepliesPage'))
const AdminPaymentConfig = lazy(() => import('../payment-config/AdminPaymentConfigPage'))

const tabs = [
  { id: 'config', label: '网关配置', icon: SettingsIcon },
  { id: 'logs', label: '运行日志', icon: ScrollText },
  { id: 'announcements', label: '系统公告', icon: Bell },
  { id: 'ticket-replies', label: '工单快捷回复', icon: MessageSquareText },
  { id: 'payment', label: '支付渠道', icon: Wallet },
] as const

type SettingsTab = (typeof tabs)[number]['id']

function resolveTab(raw: string | null): SettingsTab {
  if (raw && tabs.some((t) => t.id === raw)) return raw as SettingsTab
  return 'config'
}

export default function AdminSettings() {
  const [searchParams, setSearchParams] = useSearchParams()
  // URL is the single source of truth (supports deep links / redirects while mounted)
  const activeTab = resolveTab(searchParams.get('tab'))

  const handleTabChange = (value: string) => {
    setSearchParams({ tab: resolveTab(value) }, { replace: true })
  }

  return (
    <div className="mx-auto max-w-7xl space-y-6">
      <Tabs value={activeTab} onValueChange={handleTabChange} className="w-full">
        <TabsList className="grid h-auto w-full max-w-4xl grid-cols-2 gap-1 p-1 sm:grid-cols-3 lg:grid-cols-5 bg-muted rounded-xl">
          {tabs.map(tab => (
            <TabsTrigger
              key={tab.id}
              value={tab.id}
              className="flex items-center gap-2 py-2.5 px-3 text-sm font-medium data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow-xs rounded-lg transition-all"
            >
              <tab.icon className="h-4 w-4" />
              {tab.label}
            </TabsTrigger>
          ))}
        </TabsList>

        <TabsContent value={activeTab} className="mt-6 focus-visible:outline-none">
          <Suspense fallback={<TabFallback />}>
            {activeTab === 'config' && <AdminProxyConfigPage />}
            {activeTab === 'logs' && <AdminProxyLogsPage />}
            {activeTab === 'announcements' && <Announcements />}
            {activeTab === 'ticket-replies' && <AdminTicketQuickRepliesPage />}
            {activeTab === 'payment' && <AdminPaymentConfig />}
          </Suspense>
        </TabsContent>
      </Tabs>
    </div>
  )
}

function TabFallback() {
  return (
    <div className="space-y-3 py-6" role="status" aria-label="加载中">
      <div className="h-10 rounded-xl bg-muted animate-pulse" />
      <div className="h-10 rounded-xl bg-muted/70 animate-pulse" />
      <div className="h-32 rounded-xl bg-muted/50 animate-pulse" />
    </div>
  )
}
