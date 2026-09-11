import { useState, useCallback } from "react"
import { useSearchParams, Link } from "react-router-dom"
import { useAuthStore } from "@/features/auth/auth_store"
import { useProfile } from "@/features/auth/hooks"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/components/ui/tabs"
import { Button } from "@/shared/components/ui/button"
import { toast } from "sonner"
import {
  CreditCard,
  ReceiptText,
  RefreshCw,
  Coins,
  ShieldCheck,
} from "lucide-react"
import { userRoutes } from "@/shared/routes/user"
import Redeem from "./RedeemPage"
import BalanceHistory from "./BalanceHistoryPage"

const TABS = [
  { id: "topup", label: "充值与兑换", icon: CreditCard },
  { id: "history", label: "余额流水明细", icon: ReceiptText },
] as const

type FinanceTab = (typeof TABS)[number]["id"]

function resolveTab(raw: string | null): FinanceTab {
  if (raw === "history" || raw === "balance") return "history"
  if (raw === "topup" || raw === "redeem" || raw === "recharge") return "topup"
  return "topup"
}

export default function FinancePage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const activeTab = resolveTab(searchParams.get("tab"))
  const user = useAuthStore((s) => s.user)
  const { refetch: refetchProfile } = useProfile()
  const [refreshing, setRefreshing] = useState(false)

  const handleTabChange = (value: string) => {
    const next = resolveTab(value)
    setSearchParams({ tab: next }, { replace: true })
  }

  const handleRefreshBalance = useCallback(async () => {
    setRefreshing(true)
    try {
      const { data } = await refetchProfile()
      if (data?.user) {
        useAuthStore.getState().updateUser(data.user)
      }
      toast.success("账户余额已刷新")
    } catch {
      toast.error("刷新余额失败，请稍后重试")
    } finally {
      setRefreshing(false)
    }
  }, [refetchProfile])

  const balance = user?.balance !== undefined ? user.balance : 0

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl sm:text-3xl font-extrabold tracking-tight text-foreground">
            财务中心
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            账户资产管理、在线快速充值与实时消费流水明细
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefreshBalance}
            disabled={refreshing}
            className="h-9 gap-1.5 rounded-xl border-border bg-card"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? "animate-spin text-primary" : ""}`} />
            刷新余额
          </Button>
          <Button asChild variant="outline" size="sm" className="h-9 gap-1.5 rounded-xl border-border bg-card">
            <Link to={userRoutes.orders}>
              <ReceiptText className="w-3.5 h-3.5 text-muted-foreground" />
              充值订单
            </Link>
          </Button>
        </div>
      </div>

      {/* Financial Status Hero Banner */}
      <div className="rounded-2xl border border-border/80 bg-card p-6 sm:p-7 shadow-xs relative overflow-hidden">
        {/* Subtle decorative glow */}
        <div className="pointer-events-none absolute -right-12 -top-12 h-44 w-44 rounded-full bg-primary/5 blur-3xl" />

        <div className="flex flex-col lg:flex-row lg:items-center lg:justify-between gap-6 relative z-10">
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <div className="flex h-6 items-center gap-1.5 rounded-full bg-primary/10 px-2.5 text-xs font-semibold text-primary">
                <span className="h-1.5 w-1.5 rounded-full bg-primary animate-pulse" />
                实时精算可用
              </div>
              <span className="text-xs text-muted-foreground">
                · 按 Token 调用实时扣费
              </span>
            </div>
            <div>
              <span className="text-xs uppercase tracking-wider text-muted-foreground font-semibold">
                当前账户可用余额
              </span>
              <div className="mt-1 flex items-baseline gap-2">
                <span className="text-3xl sm:text-4xl lg:text-5xl font-extrabold tracking-tight tabular-nums text-foreground">
                  ${balance.toFixed(4)}
                </span>
                <span className="text-sm font-semibold text-muted-foreground">USD</span>
              </div>
            </div>
          </div>

          {/* Right-side quick info cards */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 lg:w-auto">
            <div className="rounded-xl border border-border/60 bg-muted/30 p-3.5 space-y-1">
              <div className="flex items-center gap-2 text-xs font-semibold text-foreground">
                <ShieldCheck className="h-4 w-4 text-primary shrink-0" />
                资金与扣费规则
              </div>
              <p className="text-xs text-muted-foreground leading-relaxed">
                充值资金永久有效，全平台可用模型通用，按实际调用 Token 精算。
              </p>
            </div>
            <div className="rounded-xl border border-border/60 bg-muted/30 p-3.5 space-y-1">
              <div className="flex items-center gap-2 text-xs font-semibold text-foreground">
                <Coins className="h-4 w-4 text-primary shrink-0" />
                充值与结算汇率
              </div>
              <p className="text-xs text-muted-foreground leading-relaxed">
                最低 $1.00 起充，扫码支付时自动按官方实时汇率折算人民币 (CNY)。
              </p>
            </div>
          </div>
        </div>
      </div>

      {/* Tabs Switcher */}
      <Tabs value={activeTab} onValueChange={handleTabChange} className="w-full space-y-6">
        <TabsList className="grid h-auto w-full max-w-md grid-cols-2 gap-1.5 rounded-2xl border border-border/80 bg-muted/60 p-1.5 shadow-2xs">
          {TABS.map((tab) => {
            const Icon = tab.icon
            return (
              <TabsTrigger
                key={tab.id}
                value={tab.id}
                className="flex items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-bold text-muted-foreground transition-all data-[state=active]:bg-card data-[state=active]:text-foreground data-[state=active]:shadow-xs"
              >
                <Icon className="w-4 h-4" />
                {tab.label}
              </TabsTrigger>
            )
          })}
        </TabsList>

        <TabsContent value="topup" className="mt-0 focus-visible:outline-none">
          <Redeem embedded onBalanceChange={handleRefreshBalance} />
        </TabsContent>

        <TabsContent value="history" className="mt-0 focus-visible:outline-none">
          <BalanceHistory />
        </TabsContent>
      </Tabs>
    </div>
  )
}
