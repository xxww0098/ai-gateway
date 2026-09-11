import { lazy, Suspense } from 'react'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/components/ui/tabs'
import { Layers, CreditCard, Crown } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'

const Pricing = lazy(() => import('../pricing/AdminPricingPage'))
const RedeemCodes = lazy(() => import('../redeem-codes/AdminRedeemCodesPage'))
const Subscriptions = lazy(() => import('../subscriptions/AdminSubscriptionsPage'))

const tabs = [
  { id: 'pricing', label: '分组倍率', icon: Layers },
  { id: 'redeem', label: '充值卡密', icon: CreditCard },
  { id: 'subscriptions', label: '订阅管理', icon: Crown },
]

const validTabIds = new Set(tabs.map((t) => t.id))

export default function Billing() {
  const [searchParams, setSearchParams] = useSearchParams()
  const rawTab = searchParams.get('tab') || 'pricing'
  const activeTab = validTabIds.has(rawTab) ? rawTab : 'pricing'

  const handleTabChange = (value: string) => {
    setSearchParams({ tab: validTabIds.has(value) ? value : 'pricing' }, { replace: true })
  }

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <Tabs value={activeTab} onValueChange={handleTabChange} className="w-full">
        <TabsList className="grid h-auto w-full max-w-md grid-cols-3 gap-1 p-1 bg-muted rounded-xl mb-6">
          {tabs.map((tab) => (
            <TabsTrigger
              key={tab.id}
              value={tab.id}
              className="py-2.5 px-3 text-sm font-medium data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow-xs rounded-lg flex items-center justify-center gap-1.5"
            >
              <tab.icon className="h-4 w-4" />
              {tab.label}
            </TabsTrigger>
          ))}
        </TabsList>

        <div className="w-full">
          <TabsContent value={activeTab} className="mt-0 focus-visible:outline-none">
            <Suspense fallback={<TabFallback />}>
              {activeTab === 'pricing' && <Pricing />}
              {activeTab === 'redeem' && <RedeemCodes />}
              {activeTab === 'subscriptions' && <Subscriptions />}
            </Suspense>
          </TabsContent>
        </div>
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
