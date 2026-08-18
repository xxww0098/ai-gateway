import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/components/ui/tabs'
import { Settings as SettingsIcon, ScrollText, Bell, MessageSquareText, Wallet } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'

import AdminProxyConfigPage from '../proxy/AdminProxyConfigPage'
import AdminProxyLogsPage from '../proxy/AdminProxyLogsPage'
import Announcements from '../announcements/AdminAnnouncementsPage'
import AdminTicketQuickRepliesPage from './AdminTicketQuickRepliesPage'
import AdminPaymentConfig from '../payment-config/AdminPaymentConfigPage'

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
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500 max-w-7xl mx-auto" style={{ willChange: 'transform, opacity' }}>
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">系统设置</h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1">
          网关运行参数、日志、公告、工单快捷回复与支付渠道配置。
        </p>
      </div>

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

        <TabsContent value="config" className="mt-6 focus-visible:outline-none">
          <AdminProxyConfigPage />
        </TabsContent>
        <TabsContent value="logs" className="mt-6 focus-visible:outline-none">
          <AdminProxyLogsPage />
        </TabsContent>
        <TabsContent value="announcements" className="mt-6 focus-visible:outline-none">
          <Announcements />
        </TabsContent>
        <TabsContent value="ticket-replies" className="mt-6 focus-visible:outline-none">
          <AdminTicketQuickRepliesPage />
        </TabsContent>
        <TabsContent value="payment" className="mt-6 focus-visible:outline-none">
          <AdminPaymentConfig />
        </TabsContent>
      </Tabs>
    </div>
  )
}
