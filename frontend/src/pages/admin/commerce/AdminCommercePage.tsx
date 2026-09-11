import { lazy, Suspense } from 'react'
import { ClipboardList, RotateCcw } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/components/ui/tabs'

const AdminOrders = lazy(() => import('../orders/AdminOrdersPage'))
const AdminRefunds = lazy(() => import('../refunds/AdminRefundsPage'))

const tabs = [
  { id: 'orders', label: '支付订单', icon: ClipboardList },
  { id: 'refunds', label: '退款审核', icon: RotateCcw },
] as const

type CommerceTab = (typeof tabs)[number]['id']

function resolveTab(raw: string | null): CommerceTab {
  if (raw === 'refunds' || raw === 'refund') return 'refunds'
  return 'orders'
}

export default function AdminCommercePage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const activeTab = resolveTab(searchParams.get('tab'))

  const handleTabChange = (value: string) => {
    setSearchParams({ tab: resolveTab(value) }, { replace: true })
  }

  return (
    <div className="mx-auto max-w-7xl space-y-6">
      <Tabs value={activeTab} onValueChange={handleTabChange} className="w-full">
        <TabsList className="grid h-auto w-full max-w-md grid-cols-2 gap-1 p-1 bg-muted rounded-xl">
          {tabs.map((tab) => (
            <TabsTrigger
              key={tab.id}
              value={tab.id}
              className="flex items-center justify-center gap-2 py-2.5 px-3 text-sm font-medium data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow-xs rounded-lg"
            >
              <tab.icon className="h-4 w-4" />
              {tab.label}
            </TabsTrigger>
          ))}
        </TabsList>

        <TabsContent value={activeTab} className="mt-6 focus-visible:outline-none">
          <Suspense fallback={<TabFallback />}>
            {activeTab === 'orders' && <AdminOrders />}
            {activeTab === 'refunds' && <AdminRefunds />}
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
