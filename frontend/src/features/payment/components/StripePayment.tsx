import { useState, useCallback } from "react"
import { Elements, CardElement, useStripe, useElements } from "@stripe/react-stripe-js"
import type { StripeCardElementOptions } from "@stripe/stripe-js"
import { useQueryClient } from "@tanstack/react-query"
import { getStripe } from "@/features/payment/stripe"
import { useAuthStore } from "@/features/auth/auth_store"
import { useStripeConfig, useCreateStripePayment } from "@/features/payment/hooks"
import { apiClient } from "@/shared/api/client"
import { queryKeys } from "@/shared/api/query-keys"
import { toast } from "sonner"

import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Alert, AlertDescription, AlertTitle } from "@/shared/components/ui/alert"
import { Badge } from "@/shared/components/ui/badge"

import {
  CreditCard,
  Loader2,
  CheckCircle2,
  AlertTriangle,
  Zap,
} from "lucide-react"

import type { StripeConfig } from "@/features/payment/types"

const PRESET_AMOUNTS = [10, 50, 100, 500]

const cardElementOptions: StripeCardElementOptions = {
  style: {
    base: {
      fontSize: "16px",
      color: "#32325d",
      "::placeholder": {
        color: "#aab7c4",
      },
    },
    invalid: {
      color: "#fa755a",
    },
  },
  hidePostalCode: true,
}

function CheckoutForm({
  config,
  onSuccess,
}: {
  config: StripeConfig
  onSuccess?: () => void
}) {
  const stripe = useStripe()
  const elements = useElements()
  const queryClient = useQueryClient()
  const user = useAuthStore((s) => s.user)
  const updateUser = useAuthStore((s) => s.updateUser)
  const token = useAuthStore((s) => s.token)

  const [amount, setAmount] = useState<number>(50)
  const [customAmount, setCustomAmount] = useState<string>("")
  const [isCustom, setIsCustom] = useState(false)
  const [loading, setLoading] = useState(false)
  const [success, setSuccess] = useState(false)
  const [successAmount, setSuccessAmount] = useState<number>(0)

  const createPayment = useCreateStripePayment()

  const selectedAmount = isCustom
    ? parseFloat(customAmount) || 0
    : amount

  const handleAmountSelect = (val: number) => {
    setAmount(val)
    setIsCustom(false)
  }

  const handleCustomChange = (val: string) => {
    setCustomAmount(val)
    setIsCustom(true)
  }

  const pollBalance = useCallback(async () => {
    if (!token) return
    try {
      const res = await apiClient.get<{ available_balance?: number; user?: { balance?: number } }>('/user/profile')
      const balance = res?.available_balance ?? res?.user?.balance
      if (typeof balance === "number") {
        updateUser({ balance })
      }
      // Also invalidate the profile query so the Header (driven by useProfile)
      // picks up the new available_balance immediately.
      void queryClient.invalidateQueries({ queryKey: queryKeys.auth.profile() })
    } catch {
      // ignore polling errors
    }
  }, [token, updateUser, queryClient])

  const handleSubmit = async (e: { preventDefault: () => void }) => {
    e.preventDefault()

    if (!stripe || !elements) {
      toast.error("Stripe 尚未加载完成，请稍后重试")
      return
    }

    const cardElement = elements.getElement(CardElement)
    if (!cardElement) {
      toast.error("卡号输入框未找到")
      return
    }

    if (selectedAmount <= 0) {
      toast.error("请输入有效的充值金额")
      return
    }

    setLoading(true)

    try {
      // 1. Create payment intent via hook's mutateAsync
      const paymentData = await createPayment.mutateAsync({
        amount: selectedAmount,
        currency: "USD",
      })

      if (!paymentData?.client_secret) {
        throw new Error("创建支付订单失败：未返回 client_secret")
      }

      // 2. Confirm card payment
      const { error, paymentIntent } = await stripe.confirmCardPayment(
        paymentData.client_secret,
        {
          payment_method: {
            card: cardElement,
            billing_details: {
              name: user?.email || "",
            },
          },
        }
      )

      if (error) {
        throw new Error(error.message || "支付失败")
      }

      if (paymentIntent?.status === "succeeded") {
        setSuccessAmount(paymentData.amount_usd)
        setSuccess(true)
        toast.success(`支付成功！充值 $${paymentData.amount_usd.toFixed(2)}`)
        // Poll balance a few times to reflect webhook update
        for (let i = 1; i <= 6; i++) {
          setTimeout(() => pollBalance(), i * 2000)
        }
        onSuccess?.()
      } else {
        throw new Error(`支付状态异常: ${paymentIntent?.status || "unknown"}`)
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "支付处理失败"
      toast.error(msg)
    } finally {
      setLoading(false)
    }
  }

  if (success) {
    return (
      <div className="space-y-6 animate-in fade-in zoom-in-95 duration-500">
        <div className="flex flex-col items-center justify-center py-10 text-center space-y-4">
          <div className="w-16 h-16 rounded-full bg-green-100 dark:bg-green-900/30 flex items-center justify-center">
            <CheckCircle2 className="w-8 h-8 text-green-600 dark:text-green-400" />
          </div>
          <div>
            <h3 className="text-xl font-bold text-foreground">支付成功</h3>
            <p className="text-muted-foreground mt-1">
              您已成功充值{" "}
              <span className="font-semibold text-foreground">
                ${successAmount.toFixed(2)} USD
              </span>
            </p>
          </div>
          <Button onClick={() => { setSuccess(false); }} variant="outline">
            继续充值
          </Button>
        </div>
      </div>
    )
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-6">
      <div className="space-y-3">
        <label className="text-sm font-medium text-foreground">选择充值金额</label>
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
          {PRESET_AMOUNTS.map((val) => (
            <button
              key={val}
              type="button"
              onClick={() => handleAmountSelect(val)}
              className={`relative rounded-xl border-2 px-4 py-3 text-center transition-all ${
                !isCustom && amount === val
                  ? "border-primary bg-primary/5 text-primary"
                  : "border-border bg-card text-foreground hover:border-primary/50 hover:bg-primary/5"
              }`}
            >
              <span className="text-lg font-bold">${val}</span>
              <span className="block text-xs text-muted-foreground">USD</span>
            </button>
          ))}
        </div>
        <div className="relative">
          <Input
            type="number"
            min={1}
            step="0.01"
            placeholder="自定义金额（USD）"
            value={customAmount}
            onChange={(e) => handleCustomChange(e.target.value)}
            className={`pr-12 ${isCustom ? "border-primary ring-1 ring-primary" : ""}`}
          />
          <span className="absolute right-3 top-1/2 -translate-y-1/2 text-sm text-muted-foreground">
            USD
          </span>
        </div>
      </div>

      <div className="space-y-3">
        <label className="text-sm font-medium text-foreground">银行卡信息</label>
        <div className="rounded-xl border border-border bg-white dark:bg-dark-950 p-4 shadow-sm">
          <CardElement options={cardElementOptions} />
        </div>
        <p className="text-xs text-muted-foreground">
          支持 Visa、MasterCard、American Express 等主流信用卡/借记卡。您的卡号信息不会经过我们的服务器。
        </p>
      </div>

      {config.mode === "sandbox" && (
        <Alert>
          <AlertTriangle className="h-4 w-4" />
          <AlertTitle>测试模式</AlertTitle>
          <AlertDescription>
            当前处于 Stripe 测试模式。可使用测试卡号{" "}
            <code className="rounded bg-muted px-1 py-0.5 text-xs font-mono">4242 4242 4242 4242</code>{" "}
            进行测试。
          </AlertDescription>
        </Alert>
      )}

      <Button
        type="submit"
        className="w-full gap-2"
        size="lg"
        disabled={loading || !stripe || selectedAmount <= 0}
      >
        {loading ? (
          <>
            <Loader2 className="w-4 h-4 animate-spin" />
            处理中...
          </>
        ) : (
          <>
            <Zap className="w-4 h-4" />
            确认支付{" "}
            <span className="font-semibold">
              ${selectedAmount > 0 ? selectedAmount.toFixed(2) : "0.00"}
            </span>
          </>
        )}
      </Button>
    </form>
  )
}

interface StripePaymentProps {
  onSuccess?: () => void
}

export default function StripePayment({ onSuccess }: StripePaymentProps) {
  const { data: config, isLoading: configLoading, error: configError, refetch } = useStripeConfig()
  const [stripeLoaded, setStripeLoaded] = useState(false)
  const [stripeError, setStripeError] = useState<string | null>(null)
  const [refreshKey, setRefreshKey] = useState(0)

  // Load Stripe SDK when config is available
  const stripeReady = config?.enabled && config?.publishable_key
  if (stripeReady && !stripeLoaded && !stripeError) {
    getStripe(config.publishable_key)
      .then(() => setStripeLoaded(true))
      .catch((err) => {
        const msg = err instanceof Error ? err.message : "加载 Stripe 失败"
        toast.error(msg)
        setStripeError(msg)
      })
  }

  const handleSuccess = useCallback(() => {
    setRefreshKey((k) => k + 1)
    onSuccess?.()
  }, [onSuccess])

  const handleRetry = useCallback(() => {
    setStripeError(null)
    setRefreshKey((k) => k + 1)
    refetch()
  }, [refetch])

  return (
    <Card className="shadow-sm border-border">
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="flex items-center gap-2">
            <CreditCard className="h-5 w-5 text-primary" />
            Stripe 支付
          </CardTitle>
          {config?.mode === "sandbox" && (
            <Badge variant="secondary" className="text-xs">测试模式</Badge>
          )}
        </div>
        <CardDescription>
          使用信用卡/借记卡安全快速地充值账户余额。
        </CardDescription>
      </CardHeader>
      <CardContent>
        {configLoading ? (
          <div className="flex items-center justify-center py-10">
            <Loader2 className="w-6 h-6 animate-spin text-muted-foreground" />
          </div>
        ) : configError || stripeError ? (
          <div className="flex flex-col items-center justify-center py-10">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={handleRetry}
            >
              重新加载
            </Button>
          </div>
        ) : !config?.enabled ? (
          <Alert>
            <AlertTriangle className="h-4 w-4" />
            <AlertTitle>支付渠道未启用</AlertTitle>
            <AlertDescription>
              Stripe 支付当前未启用，请联系管理员配置支付参数。
            </AlertDescription>
          </Alert>
        ) : !stripeLoaded ? (
          <div className="flex items-center justify-center py-10">
            <Loader2 className="w-6 h-6 animate-spin text-muted-foreground" />
          </div>
        ) : (
          <Elements
            stripe={getStripe(config.publishable_key)}
            key={refreshKey}
          >
            <CheckoutForm config={config} onSuccess={handleSuccess} />
          </Elements>
        )}
      </CardContent>
    </Card>
  )
}
