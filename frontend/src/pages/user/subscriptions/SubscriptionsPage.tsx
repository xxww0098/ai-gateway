import { useCallback, useEffect, useMemo, useState } from "react"
import { Link, useNavigate } from "react-router-dom"
import { useQueryClient } from "@tanstack/react-query"
import { errorMessage, fetchApi } from "@/shared/api/client"
import { queryKeys } from "@/shared/api/query-keys"
import { useAuthStore } from "@/features/auth/auth_store"
import { Card, CardContent } from "@/shared/components/ui/card"
import { Badge } from "@/shared/components/ui/badge"
import { Button } from "@/shared/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/components/ui/dialog"
import { Crown, Clock, CalendarDays, Zap, Wallet, ArrowLeftRight, AlertCircle } from "lucide-react"
import { toast } from "sonner"
import { useSubscriptionOrders, calculateRefund } from "@/features/user-orders"
import { userRoutes, userRefundApplyPath } from "@/shared/routes/user"

interface Subscription {
  id: number
  group_id?: number
  group_name?: string
  status: string
  starts_at: string
  expires_at: string
  price_paid?: number
  daily_usage_usd: number
  weekly_usage_usd: number
  monthly_usage_usd: number
  daily_limit_usd?: number | null
  weekly_limit_usd?: number | null
  monthly_limit_usd?: number | null
}

interface SubscriptionPackage {
  id: number
  name?: string
  description?: string | null
  rate_multiplier: number
  default_validity_days: number
  daily_limit_usd?: number | null
  weekly_limit_usd?: number | null
  monthly_limit_usd?: number | null
  subscription_price_usd?: number
}

function selfServicePrice(pkg: SubscriptionPackage): number {
  const v = pkg.subscription_price_usd
  return typeof v === "number" && v > 0 ? v : 0
}

export default function Subscriptions() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const authBalance = useAuthStore((s) => s.user?.balance)
  const updateUser = useAuthStore((s) => s.updateUser)
  const [subs, setSubs] = useState<Subscription[]>([])
  const [packages, setPackages] = useState<SubscriptionPackage[]>([])
  const [balance, setBalance] = useState<number | null>(
    typeof authBalance === "number" ? authBalance : null
  )
  const [loading, setLoading] = useState(true)
  const [checkoutPkg, setCheckoutPkg] = useState<SubscriptionPackage | null>(null)
  const [purchasing, setPurchasing] = useState(false)

  const {
    refundedSubIds,
    pendingRefundSubIds,
    completedRefundSubIds,
  } = useSubscriptionOrders()

  const loadData = useCallback(async () => {
    setLoading(true)
    try {
      const [subsRes, pkgsRes, profileRes] = await Promise.all([
        fetchApi("/user/subscriptions"),
        fetchApi("/user/subscription-packages").catch(() => ({ data: [] })),
        fetchApi("/user/profile").catch(() => ({ data: null })),
      ])
      setSubs(subsRes?.data || [])
      setPackages(pkgsRes?.data || [])
      const b = profileRes?.data?.available_balance ?? profileRes?.data?.user?.balance
      if (typeof b === "number") {
        setBalance(b)
        updateUser({ balance: b, role: profileRes.data.user?.role ?? profileRes.data.role })
      } else {
        setBalance(null)
      }
    } catch (err: unknown) {
      console.error(errorMessage(err, "加载订阅失败"))
    } finally {
      setLoading(false)
    }
  }, [updateUser])

  useEffect(() => {
    void loadData()
  }, [loadData])

  useEffect(() => {
    if (typeof authBalance === "number") {
      setBalance(authBalance)
    }
  }, [authBalance])

  const activeGroupIds = useMemo(() => {
    const set = new Set<number>()
    subs.forEach((s) => {
      if (s.status === "active" && s.group_id != null) {
        set.add(s.group_id)
      }
    })
    return set
  }, [subs])

  const confirmPurchase = async () => {
    if (!checkoutPkg) return
    const price = selfServicePrice(checkoutPkg)
    if (price <= 0) return

    setPurchasing(true)
    try {
      const res = await fetchApi("/user/subscriptions/purchase", {
        method: "POST",
        body: JSON.stringify({ group_id: checkoutPkg.id }),
      })
      const data = res?.data
      toast.success("订阅开通成功")
      if (typeof data?.balance === "number") {
        setBalance(data.balance)
        updateUser({ balance: data.balance })
      }
      // Invalidate the profile query so the Header (which displays
      // available_balance from useProfile) refetches the latest balance.
      void queryClient.invalidateQueries({ queryKey: queryKeys.auth.profile() })
      setCheckoutPkg(null)
      await loadData()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "开通失败"))
    } finally {
      setPurchasing(false)
    }
  }

  const daysRemaining = (expiresAt: string) => {
    const diff = new Date(expiresAt).getTime() - Date.now()
    return Math.max(0, Math.ceil(diff / (1000 * 60 * 60 * 24)))
  }

  const usagePercent = (usage: number, limit?: number | null) => {
    if (!limit || limit <= 0) return null
    return Math.min(100, (usage / limit) * 100)
  }

  const formatLimit = (v?: number | null) => (v != null ? `$${v.toFixed(2)}` : "∞")

  const UsageBar = ({ usage, limit, label }: { usage: number; limit?: number | null; label: string }) => {
    const pct = usagePercent(usage, limit)
    if (pct === null)
      return (
        <div className="space-y-1">
          <div className="flex justify-between text-xs text-gray-500 font-medium">
            <span>{label}</span>
            <span>${usage.toFixed(4)} / ∞</span>
          </div>
          <div className="h-2 bg-gray-100 dark:bg-dark-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full bg-gray-300 dark:bg-dark-600 w-full" />
          </div>
        </div>
      )
    const color = pct >= 90 ? "bg-red-500" : pct >= 70 ? "bg-amber-500" : "bg-emerald-500"
    return (
      <div className="space-y-1">
        <div className="flex justify-between text-xs text-gray-500 font-medium">
          <span>{label}</span>
          <span>
            ${usage.toFixed(4)} / ${limit!.toFixed(2)}
          </span>
        </div>
        <div className="h-2 bg-gray-100 dark:bg-dark-800 rounded-full overflow-hidden">
          <div className={`h-full rounded-full transition-all ${color}`} style={{ width: `${pct}%` }} />
        </div>
      </div>
    )
  }

  const statusBadge = (status: string) => {
    switch (status) {
      case "active":
        return (
          <Badge className="bg-emerald-500 hover:bg-emerald-600 text-white border-transparent shadow-sm">有效</Badge>
        )
      case "expired":
        return <Badge variant="secondary">已过期</Badge>
      case "suspended":
        return <Badge variant="destructive">已撤销</Badge>
      default:
        return <Badge variant="outline">{status}</Badge>
    }
  }

  if (loading) {
    return (
      <div className="space-y-6">
        <div className="h-8 w-48 bg-gray-200 dark:bg-dark-800 rounded animate-pulse" />
        <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
          {[1, 2, 3].map((i) => (
            <div key={i} className="h-48 bg-gray-100 dark:bg-dark-800 rounded-xl animate-pulse" />
          ))}
        </div>
      </div>
    )
  }

  const priceLabel = (p: number) => `$${p.toFixed(2)}`

  return (
    <div className="space-y-8 animate-in fade-in duration-500" style={{ willChange: "transform, opacity" }}>
      <div className="flex flex-col sm:flex-row sm:items-end sm:justify-between gap-3">
        <div className="space-y-2">
          <h3 className="text-2xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <Crown className="w-6 h-6 text-amber-500" />
            我的订阅
          </h3>
          <p className="text-gray-500">
            查看权益与额度；可退订的订阅可在此申请。订阅有效期内优先扣额度，不消耗账户余额。
          </p>
        </div>
        <Link
          to={userRoutes.refunds}
          className="text-sm text-primary-600 hover:underline shrink-0"
        >
          退款记录
        </Link>
      </div>

      {subs.length === 0 ? (
        <Card className="border-dashed border-2 border-amber-200 dark:border-amber-800/40 bg-amber-50/30 dark:bg-amber-950/10">
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <div className="w-16 h-16 bg-amber-100 dark:bg-amber-900/50 rounded-full flex items-center justify-center mb-4">
              <Crown className="w-8 h-8 text-amber-500" />
            </div>
            <h4 className="text-lg font-semibold text-gray-700 dark:text-gray-300 mb-2">暂无可用订阅</h4>
            <p className="text-sm text-gray-500 max-w-md">
              您当前没有任何有效的订阅套餐。若下方有开放中的套餐，可直接使用账户余额自行开通。
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-6 md:grid-cols-2 xl:grid-cols-3">
          {subs.map((s) => {
            const refundAmount = calculateRefund({
              id: s.id,
              group_name: s.group_name,
              status: s.status,
              starts_at: s.starts_at,
              expires_at: s.expires_at,
              price_paid: s.price_paid ?? 0,
              daily_usage_usd: s.daily_usage_usd,
              weekly_usage_usd: s.weekly_usage_usd,
              monthly_usage_usd: s.monthly_usage_usd,
            })
            const isRefundable =
              refundAmount > 0 && !refundedSubIds.has(s.id) && s.status === "active"
            const hasPendingRefund = pendingRefundSubIds.has(s.id)
            const hasCompletedRefund = completedRefundSubIds.has(s.id)

            return (
              <Card
                key={s.id}
                className="relative overflow-hidden group/card transition-all hover:shadow-lg border-amber-100 dark:border-amber-900/30 bg-amber-50/30 dark:bg-dark-900 flex flex-col"
              >
                <CardContent className="p-6 flex-1 flex flex-col">
                  <div className="flex justify-between items-start mb-6">
                    <div className="space-y-1">
                      <h4 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
                        {s.group_name || `默认订阅分组`}
                      </h4>
                      {s.status === "active" && (
                        <div className="flex items-center gap-1.5 text-sm font-medium text-amber-600 dark:text-amber-500">
                          <Clock className="w-4 h-4" />
                          剩余 {daysRemaining(s.expires_at)} 天
                        </div>
                      )}
                    </div>
                    {statusBadge(s.status)}
                  </div>

                  <div className="space-y-4 mb-4 flex-1">
                    <div className="bg-gray-50 dark:bg-dark-800/50 rounded-xl p-4 space-y-4">
                      <UsageBar usage={s.daily_usage_usd} limit={s.daily_limit_usd} label="今日额度" />
                      <UsageBar usage={s.weekly_usage_usd} limit={s.weekly_limit_usd} label="本周额度" />
                      <UsageBar usage={s.monthly_usage_usd} limit={s.monthly_limit_usd} label="本月额度" />
                    </div>

                    {isRefundable && (
                      <div className="bg-emerald-50 dark:bg-emerald-950/20 border border-emerald-100 dark:border-emerald-900/30 rounded-lg p-3">
                        <div className="flex items-center gap-2 text-emerald-700 dark:text-emerald-400 text-sm font-medium">
                          <ArrowLeftRight className="w-4 h-4" />
                          可退金额
                        </div>
                        <div className="text-xl font-bold text-emerald-600 dark:text-emerald-400 mt-1">
                          ${refundAmount.toFixed(2)}
                        </div>
                      </div>
                    )}
                    {hasPendingRefund && (
                      <div className="bg-blue-50 dark:bg-blue-950/20 border border-blue-100 dark:border-blue-900/30 rounded-lg p-3 flex items-center gap-2">
                        <AlertCircle className="w-4 h-4 text-blue-500 shrink-0" />
                        <span className="text-sm text-blue-700 dark:text-blue-400">退款审核中</span>
                      </div>
                    )}
                    {hasCompletedRefund && (
                      <div className="bg-blue-50 dark:bg-blue-950/20 border border-blue-100 dark:border-blue-900/30 rounded-lg p-3 flex items-center gap-2">
                        <AlertCircle className="w-4 h-4 text-blue-500 shrink-0" />
                        <span className="text-sm text-blue-700 dark:text-blue-400">退款已通过</span>
                      </div>
                    )}
                  </div>

                  <div className="pt-4 border-t border-gray-100 dark:border-dark-800 space-y-3">
                    <div className="text-xs text-gray-400 flex items-center justify-between">
                      <span className="flex items-center gap-1">
                        <CalendarDays className="w-3.5 h-3.5" />
                        生效: {new Date(s.starts_at).toLocaleDateString()}
                      </span>
                      <span>过期: {new Date(s.expires_at).toLocaleDateString()}</span>
                    </div>
                    {isRefundable ? (
                      <Button
                        size="sm"
                        variant="outline"
                        className="w-full gap-1 border-emerald-200 text-emerald-700 hover:bg-emerald-50 hover:text-emerald-800 dark:border-emerald-800 dark:text-emerald-400 dark:hover:bg-emerald-950/30"
                        onClick={() => navigate(userRefundApplyPath(s.id))}
                      >
                        <ArrowLeftRight className="w-3.5 h-3.5" />
                        申请退订
                      </Button>
                    ) : hasPendingRefund ? (
                      <Button
                        size="sm"
                        variant="outline"
                        className="w-full"
                        onClick={() => navigate(userRoutes.refunds)}
                      >
                        查看退款进度
                      </Button>
                    ) : null}
                  </div>
                </CardContent>
              </Card>
            )
          })}
        </div>
      )}

      {packages.length > 0 && (
        <div className="mt-12 pt-8 border-t border-gray-100 dark:border-dark-800">
          <div className="flex flex-col sm:flex-row sm:items-end sm:justify-between gap-4 mb-6">
            <div className="space-y-2">
              <h3 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
                <Zap className="w-5 h-5 text-amber-500" />
                可用订阅套餐
              </h3>
              <p className="text-sm text-gray-500 max-w-2xl">
                以下为平台当前开放的订阅套餐。已标价套餐可使用账户余额立即开通；未标价套餐由管理员分配。
              </p>
            </div>
            {balance != null && (
              <div className="flex items-center gap-2 rounded-xl border border-amber-200/70 dark:border-amber-900/40 bg-amber-50/50 dark:bg-amber-950/20 px-4 py-2 text-sm shrink-0">
                <Wallet className="h-4 w-4 text-amber-600" />
                <span className="text-gray-600 dark:text-dark-300">账户余额</span>
                <span className="font-semibold tabular-nums text-gray-900 dark:text-white">{priceLabel(balance)}</span>
                <Link to={userRoutes.financeTopup} className="text-xs text-primary-600 hover:underline ml-1">
                  充值
                </Link>
              </div>
            )}
          </div>

          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {packages.map((pkg) => {
              const price = selfServicePrice(pkg)
              const hasSelf = price > 0
              const already = activeGroupIds.has(pkg.id)
              const canBuy = hasSelf && !already && balance != null && balance + 1e-9 >= price

              return (
                <Card
                  key={pkg.id}
                  className="bg-gradient-to-br from-amber-50/50 to-orange-50/30 dark:from-amber-950/20 dark:to-orange-950/10 border-amber-100/50 dark:border-amber-900/20 shadow-sm transition-all hover:shadow-md flex flex-col"
                >
                  <CardContent className="p-5 flex flex-col flex-1">
                    <div className="flex items-start justify-between mb-4 gap-2">
                      <div className="space-y-1 min-w-0">
                        <h4 className="font-bold text-gray-900 dark:text-gray-100 text-lg flex flex-wrap items-center gap-2">
                          {pkg.name}
                          {pkg.rate_multiplier !== 1 && (
                            <Badge
                              variant="outline"
                              className="text-[10px] bg-white/50 dark:bg-black/20 text-amber-600 border-amber-200"
                            >
                              {pkg.rate_multiplier}x 倍率
                            </Badge>
                          )}
                        </h4>
                        <p className="text-xs text-gray-400">默认有效期: {pkg.default_validity_days} 天</p>
                        {pkg.description && (
                          <p className="text-xs text-gray-500 dark:text-dark-400 line-clamp-2">{pkg.description}</p>
                        )}
                      </div>
                    </div>

                    <div className="grid grid-cols-3 gap-2 mb-4">
                      <div className="bg-white/60 dark:bg-dark-800/60 rounded-lg p-2 text-center border border-gray-100 dark:border-dark-700">
                        <div className="text-[10px] text-gray-400 mb-0.5">今日额度</div>
                        <div className="text-xs font-bold text-gray-700 dark:text-gray-300">{formatLimit(pkg.daily_limit_usd)}</div>
                      </div>
                      <div className="bg-white/60 dark:bg-dark-800/60 rounded-lg p-2 text-center border border-gray-100 dark:border-dark-700">
                        <div className="text-[10px] text-gray-400 mb-0.5">本周额度</div>
                        <div className="text-xs font-bold text-gray-700 dark:text-gray-300">{formatLimit(pkg.weekly_limit_usd)}</div>
                      </div>
                      <div className="bg-white/60 dark:bg-dark-800/60 rounded-lg p-2 text-center border border-gray-100 dark:border-dark-700">
                        <div className="text-[10px] text-gray-400 mb-0.5">本月额度</div>
                        <div className="text-xs font-bold text-gray-700 dark:text-gray-300">{formatLimit(pkg.monthly_limit_usd)}</div>
                      </div>
                    </div>

                    <div className="mt-auto pt-2 border-t border-amber-100/60 dark:border-amber-900/20 flex flex-col gap-2">
                      {hasSelf ? (
                        <>
                          <div className="flex items-baseline justify-between text-sm">
                            <span className="text-gray-500">开通价</span>
                            <span className="text-lg font-bold text-amber-700 dark:text-amber-400 tabular-nums">
                              {priceLabel(price)}
                            </span>
                          </div>
                          {already ? (
                            <Button type="button" variant="secondary" className="w-full" disabled>
                              您已有该套餐的活跃订阅
                            </Button>
                          ) : (
                            <Button
                              type="button"
                              className="w-full bg-amber-600 hover:bg-amber-700 text-white"
                              disabled={!canBuy}
                              onClick={() => setCheckoutPkg(pkg)}
                            >
                              {balance != null && balance + 1e-9 < price ? "余额不足" : "用余额开通"}
                            </Button>
                          )}
                        </>
                      ) : (
                        <p className="text-xs text-center text-gray-500 dark:text-dark-400 py-2">
                          该套餐未开放自助购买，请联系管理员分配
                        </p>
                      )}
                    </div>
                  </CardContent>
                </Card>
              )
            })}
          </div>
        </div>
      )}

      <Dialog open={!!checkoutPkg} onOpenChange={(open) => !open && setCheckoutPkg(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>确认开通订阅</DialogTitle>
            <DialogDescription asChild>
              <div className="space-y-2 text-sm text-muted-foreground">
                {checkoutPkg && (
                  <>
                    <p>
                      套餐「<span className="font-medium text-foreground">{checkoutPkg.name}</span>」，有效期{" "}
                      <span className="font-medium text-foreground">{checkoutPkg.default_validity_days}</span> 天。
                    </p>
                    <p>
                      将从账户余额扣除{" "}
                      <span className="font-semibold text-amber-700 dark:text-amber-400">
                        {priceLabel(selfServicePrice(checkoutPkg))}
                      </span>
                      。开通后在有效期内使用本套餐额度，一般调用不再扣减账户余额（以平台规则为准）。
                    </p>
                  </>
                )}
              </div>
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-2 sm:gap-0">
            <Button type="button" variant="outline" onClick={() => setCheckoutPkg(null)} disabled={purchasing}>
              取消
            </Button>
            <Button
              type="button"
              className="bg-amber-600 hover:bg-amber-700 text-white"
              disabled={purchasing}
              onClick={() => void confirmPurchase()}
            >
              {purchasing ? "处理中..." : "确认支付并开通"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
