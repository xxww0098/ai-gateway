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
        <TabsList className="relative flex w-full max-w-[600px] h-auto p-1.5 bg-gray-100/80 dark:bg-dark-800/80 rounded-full mb-4">
          <div
            className="absolute top-1.5 bottom-1.5 w-[calc((100%-12px)/3)] bg-white dark:bg-dark-700 rounded-full shadow-sm transition-transform duration-300 ease-out"
            style={{
              transform: `translateX(${tabs.findIndex((t) => t.id === activeTab) * 100}%)`,
              left: '6px',
            }}
          />
          {tabs.map((tab) => (
            <TabsTrigger
              key={tab.id}
              value={tab.id}
              className="relative z-10 flex flex-1 items-center justify-center gap-2 py-3 px-4 text-sm font-medium text-gray-600 hover:text-gray-900 dark:text-dark-300 dark:hover:text-dark-50 data-[state=active]:text-gray-900 dark:data-[state=active]:text-white data-[state=active]:bg-transparent dark:data-[state=active]:bg-transparent data-[state=active]:shadow-none rounded-full transition-colors"
            >
              <tab.icon className="h-4 w-4" />
              {tab.label}
            </TabsTrigger>
          ))}
        </TabsList>

        <div className="bg-white dark:bg-dark-900 border border-gray-200 dark:border-dark-800 rounded-2xl p-6 shadow-sm min-h-[500px]">
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
