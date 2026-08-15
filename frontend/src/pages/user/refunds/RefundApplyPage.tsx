import { useState, useEffect, useCallback } from "react"
import { useNavigate, useSearchParams } from "react-router-dom"
import { errorMessage, fetchApi } from "@/shared/api/client"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Textarea } from "@/shared/components/ui/textarea"
import { Alert, AlertDescription } from "@/shared/components/ui/alert"
import { ArrowLeft, ArrowLeftRight, Calculator, CalendarDays, AlertTriangle, CheckCircle2 } from "lucide-react"
import { userRoutes } from "@/shared/routes/user"

interface Subscription {
  id: number
  group_name?: string
  status: string
  starts_at: string
  expires_at: string
  price_paid: number
}

interface RefundRecord {
  id: number
  subscription_id: number
  status: string
}

function daysBetween(a: string | Date, b: string | Date): number {
  const d1 = new Date(a).getTime()
  const d2 = new Date(b).getTime()
  const diff = d2 - d1
  return Math.max(0, Math.ceil(diff / (1000 * 60 * 60 * 24)))
}

function calculateRefund(sub: Subscription) {
  const now = new Date()
  const expiresAt = new Date(sub.expires_at)
  const startsAt = new Date(sub.starts_at)

  if (now >= expiresAt) {
    return { amount: 0, dailyRate: 0, daysUsed: 0, totalDays: 0, remainingDays: 0 }
  }

  const totalDays = daysBetween(startsAt, expiresAt) || 1
  const dailyRate = sub.price_paid / totalDays
  const daysUsed = daysBetween(startsAt, now)
  const remainingDays = daysBetween(now, expiresAt)
  const amount = remainingDays * dailyRate

  return {
    amount: Math.max(0, Math.min(amount, sub.price_paid)),
    dailyRate,
    daysUsed,
    totalDays,
    remainingDays,
  }
}

export default function RefundApply() {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const preselectedId = parseInt(searchParams.get("subscription_id") || "0", 10)

  const [subs, setSubs] = useState<Subscription[]>([])
  const [refunds, setRefunds] = useState<RefundRecord[]>([])
  const [selectedId, setSelectedId] = useState<number>(preselectedId)
  const [reason, setReason] = useState("")
  const [loading, setLoading] = useState(true)
  const [submitting, setSubmitting] = useState(false)
  const [submitted, setSubmitted] = useState(false)

  const isRefundable = useCallback((sub: Subscription, refundList: RefundRecord[]) => {
    if (sub.status !== "active") return false
    const now = new Date()
    if (now >= new Date(sub.expires_at)) return false
    if ((sub.price_paid || 0) <= 0) return false
    const hasRefund = refundList.some(
      (r) => r.subscription_id === sub.id && (r.status === "pending" || r.status === "approved")
    )
    return !hasRefund
  }, [])

  const loadData = useCallback(async () => {
    setLoading(true)
    try {
      const [subsRes, refundsRes] = await Promise.all([
        fetchApi("/user/subscriptions"),
        fetchApi("/refund/list").catch(() => ({ data: { items: [] } })),
      ])
      const subscriptions = subsRes?.data || []
      setSubs(subscriptions)
      setRefunds(refundsRes?.data?.items || [])

      const preselected = subscriptions.find((s: Subscription) => s.id === preselectedId)
      if (!preselected || !isRefundable(preselected, refundsRes?.data?.items || [])) {
        const firstRefundable = subscriptions.find((s: Subscription) =>
          isRefundable(s, refundsRes?.data?.items || [])
        )
        if (firstRefundable) {
          setSelectedId(firstRefundable.id)
        } else {
          setSelectedId(0)
        }
      }
    } catch (err: unknown) {
      toast.error(errorMessage(err, "加载失败"))
    } finally {
      setLoading(false)
    }
  }, [isRefundable, preselectedId])

  useEffect(() => {
    loadData()
  }, [loadData])

  const selectedSub = subs.find((s) => s.id === selectedId)
  const calc = selectedSub ? calculateRefund(selectedSub) : null

  const refundableSubs = subs.filter((s) => isRefundable(s, refunds))

  const handleSubmit = async () => {
    if (!selectedId) {
      toast.error("请选择要退订的订单")
      return
    }
    if (!calc || calc.amount <= 0) {
      toast.error("该订单无可退金额")
      return
    }

    setSubmitting(true)
    try {
      await fetchApi("/refund/apply", {
        method: "POST",
        body: JSON.stringify({
          subscription_id: selectedId,
          reason: reason.trim(),
        }),
      })
      setSubmitted(true)
      toast.success("退订申请已提交")
    } catch (err: unknown) {
      toast.error(errorMessage(err, "申请失败"))
    } finally {
      setSubmitting(false)
    }
  }

  if (loading) {
    return (
      <div className="max-w-2xl mx-auto space-y-6">
        <div className="h-8 w-48 bg-gray-200 dark:bg-dark-800 rounded animate-pulse" />
        <div className="h-96 bg-gray-100 dark:bg-dark-800 rounded-xl animate-pulse" />
      </div>
    )
  }

  if (submitted) {
    return (
      <div className="max-w-2xl mx-auto space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500">
        <Button variant="ghost" className="gap-1 -ml-3" onClick={() => navigate(userRoutes.subscriptions)}>
          <ArrowLeft className="w-4 h-4" />
          返回订阅
        </Button>

        <Card className="border-emerald-100 dark:border-emerald-900/30 bg-emerald-50/30 dark:bg-emerald-950/10">
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <div className="w-16 h-16 bg-emerald-100 dark:bg-emerald-900/50 rounded-full flex items-center justify-center mb-4">
              <CheckCircle2 className="w-8 h-8 text-emerald-500" />
            </div>
            <h4 className="text-lg font-semibold text-gray-900 dark:text-white mb-2">退订申请已提交</h4>
            <p className="text-sm text-gray-500 max-w-md mb-6">
              您的退订申请已收到。金额小于 $100 的退订将自动通过，其余需等待管理员审核。
            </p>
            <div className="flex gap-3">
              <Button variant="outline" onClick={() => navigate(userRoutes.subscriptions)}>
                返回订阅
              </Button>
              <Button onClick={() => navigate(userRoutes.refunds)}>查看退款进度</Button>
            </div>
          </CardContent>
        </Card>
      </div>
    )
  }

  if (refundableSubs.length === 0) {
    return (
      <div className="max-w-2xl mx-auto space-y-6">
        <Button variant="ghost" className="gap-1 -ml-3" onClick={() => navigate(userRoutes.subscriptions)}>
          <ArrowLeft className="w-4 h-4" />
          返回订阅
        </Button>

        <Alert className="bg-amber-50 dark:bg-amber-950/20 border-amber-200 dark:border-amber-900/30">
          <AlertTriangle className="h-4 w-4 text-amber-500" />
          <AlertDescription className="text-amber-700 dark:text-amber-400">
            当前没有可退订的订阅。只有状态为有效、未过期且未申请过退款的订阅才能退订。
          </AlertDescription>
        </Alert>
      </div>
    )
  }

  return (
    <div className="max-w-2xl mx-auto space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500">
      <Button variant="ghost" className="gap-1 -ml-3" onClick={() => navigate(userRoutes.subscriptions)}>
        <ArrowLeft className="w-4 h-4" />
        返回订阅
      </Button>

      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white flex items-center gap-2">
          <ArrowLeftRight className="w-6 h-6 text-primary" />
          申请退订
        </h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1">
            选择要退订的订单，系统将自动计算可退金额。审核通过后将取消对应订阅权益，并按规则调整账户余额。
        </p>
      </div>

      <Card className="shadow-sm border-border">
        <CardHeader>
          <CardTitle className="text-lg flex items-center gap-2">
            <Calculator className="w-5 h-5 text-primary" />
            退订计算
          </CardTitle>
          <CardDescription>选择订单后，系统将根据剩余天数自动计算可退金额。</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="space-y-2">
            <label className="text-sm font-medium">选择订单</label>
            <div className="grid gap-2">
              {refundableSubs.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => setSelectedId(s.id)}
                  className={`flex items-center justify-between p-3 rounded-lg border text-left transition-all ${
                    selectedId === s.id
                      ? "border-primary bg-primary/5 dark:bg-primary/10"
                      : "border-gray-200 dark:border-dark-700 hover:border-gray-300 dark:hover:border-dark-600"
                  }`}
                >
                  <div className="space-y-0.5">
                    <div className="font-medium text-sm text-gray-900 dark:text-white">
                      {s.group_name || `订阅 #${s.id}`}
                    </div>
                    <div className="text-xs text-gray-500 flex items-center gap-2">
                      <span className="flex items-center gap-1">
                        <CalendarDays className="w-3 h-3" />
                        {new Date(s.starts_at).toLocaleDateString()} - {new Date(s.expires_at).toLocaleDateString()}
                      </span>
                    </div>
                  </div>
                  <div className="text-sm font-semibold text-gray-900 dark:text-white">
                    ${s.price_paid?.toFixed(2) || "0.00"}
                  </div>
                </button>
              ))}
            </div>
          </div>

          {calc && selectedSub && calc.amount > 0 && (
            <div className="bg-gray-50 dark:bg-dark-800/50 rounded-xl p-4 space-y-3">
              <h4 className="text-sm font-semibold text-gray-700 dark:text-gray-300">计算详情</h4>
              <div className="grid grid-cols-2 gap-3 text-sm">
                <div className="flex justify-between">
                  <span className="text-gray-500">订单金额</span>
                  <span className="font-medium">${selectedSub.price_paid.toFixed(2)}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-gray-500">服务总天数</span>
                  <span className="font-medium">{calc.totalDays} 天</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-gray-500">日单价</span>
                  <span className="font-medium">${calc.dailyRate.toFixed(4)}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-gray-500">已使用天数</span>
                  <span className="font-medium">{calc.daysUsed} 天</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-gray-500">剩余天数</span>
                  <span className="font-medium text-emerald-600 dark:text-emerald-400">{calc.remainingDays} 天</span>
                </div>
              </div>
              <div className="pt-3 border-t border-gray-200 dark:border-dark-700">
                <div className="flex items-center justify-between">
                  <span className="text-sm font-semibold text-gray-700 dark:text-gray-300">可退金额</span>
                  <span className="text-2xl font-bold text-emerald-600 dark:text-emerald-400">
                    ${calc.amount.toFixed(2)}
                  </span>
                </div>
                {calc.amount < 100 && (
                  <p className="text-xs text-emerald-600/70 dark:text-emerald-400/70 mt-1">
                    金额小于 $100，提交后将自动通过
                  </p>
                )}
              </div>
            </div>
          )}

          {calc && calc.amount <= 0 && (
            <Alert className="bg-red-50 dark:bg-red-950/20 border-red-200 dark:border-red-900/30">
              <AlertTriangle className="h-4 w-4 text-red-500" />
              <AlertDescription className="text-red-700 dark:text-red-400">
                该订单当前无可退金额，可能已过期或剩余天数为 0。
              </AlertDescription>
            </Alert>
          )}

          <div className="space-y-2">
            <label className="text-sm font-medium">退订原因</label>
            <Textarea
              placeholder="请简要说明退订原因（选填，最多 500 字）"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              rows={4}
              maxLength={500}
            />
            <div className="flex justify-between text-xs text-gray-400">
              <span>这将帮助管理员了解您的需求</span>
              <span>{reason.length}/500</span>
            </div>
          </div>

          <Button
            className="w-full"
            size="lg"
            disabled={!selectedId || !calc || (calc.amount ?? 0) <= 0 || submitting}
            onClick={handleSubmit}
          >
            {submitting ? "提交中..." : `确认申请退订${calc && (calc.amount ?? 0) > 0 ? ` ($${calc.amount.toFixed(2)})` : ""}`}
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
