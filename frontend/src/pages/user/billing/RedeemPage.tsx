import { useState, useCallback } from "react"
import { Link } from "react-router-dom"
import { useAuthStore } from "@/features/auth/auth_store"
import { fetchApi } from "@/shared/api/client"
import { useProfile } from "@/features/auth/hooks"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { toast } from "sonner"
import {
  Gift,
  Wallet,
  Loader2,
  Sparkles,
  Info,
  CheckCircle2,
  ExternalLink,
  ShieldCheck,
  Zap,
} from "lucide-react"
import { userRoutes } from "@/shared/routes/user"
import OnlineTopup from "@/features/payment/components/OnlineTopup"

type RedeemProps = {
  /** When true, omit page-level chrome (used inside Finance tabs). */
  embedded?: boolean
  onBalanceChange?: () => void
}

export default function Redeem({ embedded = false, onBalanceChange }: RedeemProps) {
  const user = useAuthStore((s) => s.user)
  const [code, setCode] = useState("")
  const [loading, setLoading] = useState(false)
  const { refetch: refetchProfile } = useProfile()

  const refreshBalance = useCallback(async () => {
    try {
      const { data } = await refetchProfile()
      if (data?.user) {
        useAuthStore.getState().updateUser(data.user)
      }
      onBalanceChange?.()
    } catch (err: unknown) {
      console.error("Refresh balance failed:", err)
    }
  }, [refetchProfile, onBalanceChange])

  const handleRedeem = async (e: React.FormEvent) => {
    e.preventDefault()
    const trimmed = code.trim()
    if (!trimmed) return

    setLoading(true)
    try {
      const res = await fetchApi("/user/redeem", {
        method: "POST",
        body: JSON.stringify({ code: trimmed }),
      })
      toast.success(`兑换成功！您的账户增加了 $${res.data.amount.toFixed(4)}`)
      setCode("")
      await refreshBalance()
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "兑换失败，请检查兑换码是否有效")
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="space-y-6">
      {/* If standalone (not embedded in FinancePage), render the balance hero card */}
      {!embedded && (
        <Card className="flex flex-col items-center justify-center rounded-2xl border border-border/80 bg-card p-6 shadow-xs">
          <Wallet className="h-10 w-10 sm:h-12 sm:w-12 text-primary mb-3" />
          <div className="text-sm font-medium text-muted-foreground mb-1">当前账户余额</div>
          <div className="text-3xl sm:text-4xl font-extrabold text-foreground tabular-nums tracking-tight">
            ${user?.balance?.toFixed(4) || "0.0000"}
          </div>
        </Card>
      )}

      {/* Main 12-Column Responsive Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6 items-start">
        {/* Left Column: Unified Online Top-up */}
        <div className="lg:col-span-7 xl:col-span-8">
          <OnlineTopup onSuccess={refreshBalance} />
        </div>

        {/* Right Column: Redeem Code + Billing Guidelines */}
        <div className="lg:col-span-5 xl:col-span-4 space-y-6">
          {/* Redeem Code Card */}
          <Card className="rounded-2xl border border-border/80 bg-card p-6 shadow-xs space-y-5">
            <CardHeader className="p-0 space-y-1.5">
              <div className="flex items-center gap-2.5">
                <div className="w-9 h-9 rounded-xl bg-primary/10 flex items-center justify-center text-primary shrink-0">
                  <Gift className="w-5 h-5" />
                </div>
                <div>
                  <CardTitle className="text-base font-bold text-foreground">
                    兑换码充值
                  </CardTitle>
                  <CardDescription className="text-xs text-muted-foreground">
                    输入活动礼券或卡密充值额度
                  </CardDescription>
                </div>
              </div>
            </CardHeader>

            <CardContent className="p-0 space-y-4">
              <form onSubmit={handleRedeem} className="space-y-3">
                <div className="space-y-1.5">
                  <label className="text-xs font-semibold text-muted-foreground">
                    卡密兑换码
                  </label>
                  <Input
                    placeholder="例如：AGW-a1b2c3d4..."
                    value={code}
                    onChange={(e) => setCode(e.target.value)}
                    className="font-mono text-sm uppercase rounded-xl border-border bg-background h-11"
                    disabled={loading}
                    required
                  />
                </div>

                <Button
                  type="submit"
                  className="w-full h-11 text-sm font-bold rounded-xl gap-2 shadow-xs"
                  disabled={loading || !code.trim()}
                >
                  {loading ? (
                    <>
                      <Loader2 className="w-4 h-4 animate-spin" />
                      正在核销兑换码...
                    </>
                  ) : (
                    <>
                      <Sparkles className="w-4 h-4" />
                      立即兑换充值
                    </>
                  )}
                </Button>
              </form>

              <div className="rounded-xl border border-border/60 bg-muted/30 p-3 text-xs text-muted-foreground space-y-1">
                <div className="font-semibold text-foreground flex items-center gap-1.5">
                  <CheckCircle2 className="w-3.5 h-3.5 text-primary" />
                  即时生效
                </div>
                <p className="leading-relaxed">
                  卡密一旦兑换成功，额度将实时注入您的账户，可在【余额流水】中查看详细入账凭证。
                </p>
              </div>
            </CardContent>
          </Card>

          {/* Billing & Finance Notice Card */}
          <Card className="rounded-2xl border border-border/80 bg-card p-6 shadow-xs space-y-4">
            <CardHeader className="p-0 space-y-1">
              <div className="flex items-center gap-2">
                <Info className="w-4 h-4 text-primary" />
                <CardTitle className="text-sm font-bold text-foreground">
                  充值与计费说明
                </CardTitle>
              </div>
            </CardHeader>

            <CardContent className="p-0 space-y-3 text-xs text-muted-foreground">
              <div className="flex gap-2.5">
                <Zap className="w-4 h-4 text-primary shrink-0 mt-0.5" />
                <div className="space-y-0.5">
                  <span className="font-semibold text-foreground">实时精算扣费</span>
                  <p className="leading-relaxed">
                    API 调用根据请求的真实 Token（输入/输出/推理/缓存）与模型单价实时精确扣除。
                  </p>
                </div>
              </div>

              <div className="flex gap-2.5">
                <ShieldCheck className="w-4 h-4 text-primary shrink-0 mt-0.5" />
                <div className="space-y-0.5">
                  <span className="font-semibold text-foreground">余额永久有效</span>
                  <p className="leading-relaxed">
                    所有充值与兑换余额均无过期时间，永久保存直至消费完毕。
                  </p>
                </div>
              </div>

              <div className="flex gap-2.5">
                <Wallet className="w-4 h-4 text-primary shrink-0 mt-0.5" />
                <div className="space-y-0.5">
                  <span className="font-semibold text-foreground">发票与工单</span>
                  <p className="leading-relaxed">
                    如需对公转账入账、增值税发票开具或账单疑义，可前往{" "}
                    <Link
                      to={userRoutes.tickets}
                      className="text-primary hover:underline font-semibold inline-flex items-center gap-0.5"
                    >
                      工单支持
                      <ExternalLink className="w-3 h-3" />
                    </Link>{" "}
                    提交协助申请。
                  </p>
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  )
}
