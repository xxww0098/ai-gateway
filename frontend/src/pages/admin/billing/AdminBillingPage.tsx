import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/components/ui/tabs'
import { Layers, CreditCard, Crown } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'

import Pricing from '../pricing/AdminPricingPage'
import RedeemCodes from '../redeem-codes/AdminRedeemCodesPage'
import Subscriptions from '../subscriptions/AdminSubscriptionsPage'

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
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500 max-w-6xl mx-auto">
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">计费</h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1 text-sm">
          分组倍率、兑换卡密与订阅套餐。模型基础价在「模型」页编辑。
        </p>
      </div>

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
          <TabsContent value="pricing" className="mt-0 focus-visible:outline-none">
            <Pricing />
          </TabsContent>
          <TabsContent value="redeem" className="mt-0 focus-visible:outline-none">
            <RedeemCodes />
          </TabsContent>
          <TabsContent value="subscriptions" className="mt-0 focus-visible:outline-none">
            <Subscriptions />
          </TabsContent>
        </div>
      </Tabs>
    </div>
  )
}
