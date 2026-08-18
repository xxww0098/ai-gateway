import { ClipboardList, RotateCcw } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/components/ui/tabs'
import AdminOrders from '../orders/AdminOrdersPage'
import AdminRefunds from '../refunds/AdminRefundsPage'

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
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500 max-w-7xl mx-auto">
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">交易管理</h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1 text-sm">
          集中处理全站用户的在线充值订单与订阅退款申请。
        </p>
      </div>

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

        <TabsContent value="orders" className="mt-6 focus-visible:outline-none">
          <AdminOrders embedded />
        </TabsContent>
        <TabsContent value="refunds" className="mt-6 focus-visible:outline-none">
          <AdminRefunds embedded />
        </TabsContent>
      </Tabs>
    </div>
  )
}
