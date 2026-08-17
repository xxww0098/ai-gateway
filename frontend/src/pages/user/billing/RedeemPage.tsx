import { useState, useCallback } from "react"
import { useAuthStore } from "@/features/auth/auth_store"
import { fetchApi } from "@/shared/api/client"
import { useProfile } from "@/features/auth/hooks"
import { Card, CardDescription } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { toast } from "sonner"
import { ChevronDown, Gift, Wallet } from "lucide-react"
import StripePayment from "@/features/payment/components/StripePayment"
import WechatPaySection from "@/features/payment/components/WechatPaySection"
import AlipayPayment from "@/features/payment/components/AlipayPayment"

type RedeemProps = {
  /** When true, omit page-level chrome (used inside Finance tabs). */
  embedded?: boolean
}

export default function Redeem({ embedded = false }: RedeemProps) {
  const user = useAuthStore(s => s.user)
  const [code, setCode] = useState("")
  const [loading, setLoading] = useState(false)
  const [redeemOpen, setRedeemOpen] = useState(false)
  const { refetch: refetchProfile } = useProfile()

  const refreshBalance = useCallback(async () => {
    try {
      const { data } = await refetchProfile()
      if (data?.user) {
        useAuthStore.getState().updateUser(data.user)
      }
    } catch (err: unknown) {
      console.error("Refresh balance failed:", err)
    }
  }, [refetchProfile])

  const handleRedeem = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!code.trim()) return

    setLoading(true)
    try {
      const res = await fetchApi("/user/redeem", {
        method: "POST",
        body: JSON.stringify({ code: code.trim() }),
      })
      toast.success(`充值成功！您的账户增加了 $${res.data.amount.toFixed(4)}`)
      setCode("")
      await refreshBalance()
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "兑换失败，请检查兑换码是否有效")
    } finally {
      setLoading(false)
    }
  }

  return (
    <div
      className={`space-y-5 max-w-2xl mx-auto ${embedded ? '' : 'animate-in fade-in slide-in-from-bottom-4 duration-500'}`}
      style={embedded ? undefined : { willChange: 'transform, opacity' }}
    >
      {!embedded && (
        <div className="text-center md:text-left">
          <h2 className="text-2xl sm:text-3xl font-bold tracking-tight text-foreground">充值与兑换</h2>
          <p className="text-muted-foreground mt-1 text-sm sm:text-base">
            使用支付宝、微信或兑换码增加可用余额。
          </p>
        </div>
      )}

      {/* Balance always first on mobile */}
      <Card className="shadow-sm border-border flex flex-col justify-center items-center p-5 sm:p-6 bg-gradient-to-br from-primary/5 to-secondary/30">
        <Wallet className="h-10 w-10 sm:h-12 sm:w-12 text-primary mb-3" />
        <div className="text-sm text-muted-foreground mb-1">当前账户余额</div>
        <div className="text-3xl sm:text-4xl font-bold text-foreground tabular-nums">
          ${user?.balance?.toFixed(4) || "0.00"}
        </div>
      </Card>

      {/* Online payment first — primary path */}
      <div className="space-y-4">
        <p className="text-sm font-medium text-gray-700 dark:text-gray-300">在线充值</p>
        <AlipayPayment onSuccess={refreshBalance} />
        <WechatPaySection onSuccess={refreshBalance} />
        <StripePayment onSuccess={refreshBalance} />
      </div>

      {/* Redeem code collapsed by default on mobile intent */}
      <div className="rounded-xl border border-border overflow-hidden">
        <button
          type="button"
          onClick={() => setRedeemOpen((v) => !v)}
          className="flex w-full min-h-12 items-center justify-between gap-2 px-4 py-3 text-left bg-gray-50/80 dark:bg-dark-800/50"
          aria-expanded={redeemOpen}
        >
          <span className="flex items-center gap-2 text-sm font-medium">
            <Gift className="h-4 w-4 text-primary" />
            我有兑换码
          </span>
          <ChevronDown
            className={`h-4 w-4 text-muted-foreground transition-transform ${redeemOpen ? 'rotate-180' : ''}`}
          />
        </button>
        {redeemOpen && (
          <div className="p-4 border-t border-border">
            <CardDescription className="mb-3">
              请输入 16 位或 32 位的充值卡密。
            </CardDescription>
            <form onSubmit={handleRedeem} className="space-y-3">
              <Input
                placeholder="例如：AGW-a1b2c3d4..."
                value={code}
                onChange={(e) => setCode(e.target.value)}
                className="font-mono text-base sm:text-sm min-h-11"
                required
              />
              <Button type="submit" className="w-full min-h-11" disabled={loading || !code.trim()}>
                {loading ? "处理中..." : "立即兑换"}
              </Button>
            </form>
          </div>
        )}
      </div>
    </div>
  )
}
