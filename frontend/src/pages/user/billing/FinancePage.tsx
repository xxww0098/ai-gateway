import { Wallet } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/components/ui/tabs'
import Redeem from './RedeemPage'
import BalanceHistory from './BalanceHistoryPage'

const TABS = [
  { id: 'topup', label: '充值兑换' },
  { id: 'history', label: '余额流水' },
] as const

type FinanceTab = (typeof TABS)[number]['id']

function resolveTab(raw: string | null): FinanceTab {
  if (raw === 'history' || raw === 'balance') return 'history'
  if (raw === 'topup' || raw === 'redeem' || raw === 'recharge') return 'topup'
  return 'topup'
}

export default function FinancePage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const activeTab = resolveTab(searchParams.get('tab'))

  const handleTabChange = (value: string) => {
    const next = resolveTab(value)
    setSearchParams({ tab: next }, { replace: true })
  }

  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500">
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white flex items-center gap-2">
          <Wallet className="w-6 h-6 text-emerald-600" />
          财务
        </h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1">
          充值、兑换码与余额变动记录。
        </p>
      </div>

      <Tabs value={activeTab} onValueChange={handleTabChange} className="w-full">
        <TabsList className="grid h-auto w-full max-w-md grid-cols-2 gap-1 p-1 bg-gray-100/80 dark:bg-dark-800/80 rounded-xl">
          {TABS.map((tab) => (
            <TabsTrigger
              key={tab.id}
              value={tab.id}
              className="py-2.5 px-3 text-sm font-medium data-[state=active]:bg-white dark:data-[state=active]:bg-dark-700 data-[state=active]:shadow-sm rounded-lg"
            >
              {tab.label}
            </TabsTrigger>
          ))}
        </TabsList>

        <TabsContent value="topup" className="mt-6 focus-visible:outline-none">
          <Redeem embedded />
        </TabsContent>
        <TabsContent value="history" className="mt-6 focus-visible:outline-none">
          <BalanceHistory embedded />
        </TabsContent>
      </Tabs>
    </div>
  )
}
