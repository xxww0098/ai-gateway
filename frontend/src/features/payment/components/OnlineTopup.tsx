import { useState, useCallback, useRef, useEffect } from "react"
import { toast } from "sonner"
import { QRCodeSVG } from "qrcode.react"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Badge } from "@/shared/components/ui/badge"
import {
  CreditCard,
  Loader2,
  ExternalLink,
  CheckCircle2,
  XCircle,
  RefreshCw,
  Copy,
  Smartphone,
  ArrowRight,
  ShieldCheck,
} from "lucide-react"
import {
  useCreateAlipayOrder,
  useAlipayOrderStatus,
  useCreateWechatOrder,
  useWechatOrderStatus,
  useStripeConfig,
} from "@/features/payment/hooks"
import type { AlipayCreateResponse, WechatCreateResponse } from "@/features/payment/types"
import StripePayment from "./StripePayment"

const PRESET_AMOUNTS = [10, 20, 50, 100, 200, 500]

type PaymentMethod = "alipay" | "wechat" | "stripe"

interface OnlineTopupProps {
  onSuccess?: () => void
}

function AlipayLogo({ className = "w-5 h-5" }: { className?: string }) {
  return (
    <svg viewBox="0 0 1024 1024" className={className} fill="currentColor">
      <path
        d="M1024 779.6c0 135-109.4 244.4-244.4 244.4H244.4C109.4 1024 0 914.6 0 779.6V244.4C0 109.4 109.4 0 244.4 0h535.2C914.6 0 1024 109.4 1024 244.4v535.2z"
        fill="#1677FF"
      />
      <path
        d="M840.4 756.8c-77.8-38.4-184.8-100-244.2-152 47.6-67.4 83-149.8 98-237.6H548.8V313h175.4v-46.6H548.8V170.8h-74.2v95.6H300.2V313h174.4v54.2H355.8v46.6h259.2c-13.6 68.6-43.2 133.2-82.6 186.2-70-65.8-132.8-144-177.2-229.4l-66.2 31.8c49 94.2 118.4 180.4 195.4 252.8-106.8 54.8-230.8 90.8-316 108.6 12 18.2 30.6 44.8 45.4 63.8 84.4-20.2 211.8-60.6 322-121.2 73.8 63.6 200.4 135 291 180.2l54.6-60.8z"
        fill="#FFFFFF"
      />
    </svg>
  )
}

function WechatPayLogo({ className = "w-5 h-5" }: { className?: string }) {
  return (
    <svg viewBox="0 0 1024 1024" className={className} fill="currentColor">
      <path
        d="M1024 779.6c0 135-109.4 244.4-244.4 244.4H244.4C109.4 1024 0 914.6 0 779.6V244.4C0 109.4 109.4 0 244.4 0h535.2C914.6 0 1024 109.4 1024 244.4v535.2z"
        fill="#07C160"
      />
      <path
        d="M421.2 595.6c-13.2 0-25.6-1.2-37.6-3.2-33.8 24.2-74.6 44.6-118 45.2-1.4 0-2.8 0-4.2-0.2-3-0.4-5.4-2.8-5.8-5.8-0.4-3.4 1.4-6.6 4.4-8 23.4-11.2 46.2-26.4 56.6-43.4-61-38.4-97.4-97.8-97.4-165 0-112.6 100.8-204 225.4-204 121.8 0 221.2 87.6 225.2 196.8-5.6-0.4-11.2-0.8-17-0.8-111 0-202.4 81.2-205.6 184.2-2 1.4-4 2.8-6 4.2zm-97.4-222c-15.6 0-28.2 12.6-28.2 28.2s12.6 28.2 28.2 28.2 28.2-12.6 28.2-28.2-12.6-28.2-28.2-28.2zm149.2 0c-15.6 0-28.2 12.6-28.2 28.2s12.6 28.2 28.2 28.2 28.2-12.6 28.2-28.2-12.6-28.2-28.2-28.2z"
        fill="#FFFFFF"
      />
      <path
        d="M805 607.6c0-93.8-84-170-187.6-170s-187.6 76.2-187.6 170c0 93.8 84 170 187.6 170 20.8 0 40.8-3.2 59.4-9 24.6 17.6 54.4 32.4 86 32.8 1 0 2 0 3-0.2 2.2-0.2 4-2 4.4-4.2 0.2-2.4-1-4.8-3.2-5.8-17-8.2-33.6-19.2-41.2-31.6 49.4-30 79.2-77.8 79.2-132zm-247-38.2c-13 0-23.6-10.6-23.6-23.6s10.6-23.6 23.6-23.6 23.6 10.6 23.6 23.6-10.6 23.6-23.6 23.6zm118.8 0c-13 0-23.6-10.6-23.6-23.6s10.6-23.6 23.6-23.6 23.6 10.6 23.6 23.6-10.6 23.6-23.6 23.6z"
        fill="#FFFFFF"
      />
    </svg>
  )
}

export default function OnlineTopup({ onSuccess }: OnlineTopupProps) {
  const [selectedPreset, setSelectedPreset] = useState<number | null>(50)
  const [amountInput, setAmountInput] = useState<string>("50")
  const [method, setMethod] = useState<PaymentMethod>("alipay")

  // Alipay state
  const [alipayOrder, setAlipayOrder] = useState<AlipayCreateResponse | null>(null)
  const [alipayPolling, setAlipayPolling] = useState(false)
  const createAlipay = useCreateAlipayOrder()
  const alipayStatusQuery = useAlipayOrderStatus(alipayOrder?.order_id ?? null, alipayPolling)
  const alipayStatus = alipayStatusQuery.data?.status ?? null

  // Wechat state
  const [wechatOrder, setWechatOrder] = useState<WechatCreateResponse | null>(null)
  const [wechatPolling, setWechatPolling] = useState(false)
  const createWechat = useCreateWechatOrder()
  const wechatStatusQuery = useWechatOrderStatus(wechatOrder?.order_id ?? null, wechatPolling)
  const wechatStatus = wechatStatusQuery.data?.status ?? null

  // Stripe
  const { data: stripeConfig } = useStripeConfig()

  const onSuccessRef = useRef(onSuccess)
  useEffect(() => {
    onSuccessRef.current = onSuccess
  })

  // Track handled order completions to prevent duplicate toasts
  const handledOrderRef = useRef<string | null>(null)

  // Alipay completion check
  useEffect(() => {
    const orderId = alipayOrder?.order_id
    if (!orderId) return
    if (alipayStatus !== "paid" && alipayStatus !== "failed") return
    if (handledOrderRef.current === orderId) return
    handledOrderRef.current = orderId
    setAlipayPolling(false)
    if (alipayStatus === "paid") {
      const amt = alipayStatusQuery.data?.amount ?? alipayOrder.amount_usd
      toast.success(`支付宝支付成功！账户已充值 $${amt.toFixed(2)}`)
      onSuccessRef.current?.()
    } else {
      toast.error("支付宝支付失败或已取消")
    }
  }, [alipayStatus, alipayOrder, alipayStatusQuery.data?.amount])

  // Wechat completion check
  useEffect(() => {
    const orderId = wechatOrder?.order_id
    if (!orderId) return
    if (wechatStatus !== "paid" && wechatStatus !== "failed") return
    if (handledOrderRef.current === orderId) return
    handledOrderRef.current = orderId
    setWechatPolling(false)
    if (wechatStatus === "paid") {
      const amt = wechatStatusQuery.data?.amount ?? wechatOrder.amount_usd
      toast.success(`微信支付成功！账户已充值 $${amt.toFixed(2)}`)
      onSuccessRef.current?.()
    } else {
      toast.error("微信支付失败或已取消")
    }
  }, [wechatStatus, wechatOrder, wechatStatusQuery.data?.amount])

  const parsedAmount = parseFloat(amountInput)
  const isValidAmount = !isNaN(parsedAmount) && parsedAmount >= 1

  const handleSelectPreset = (val: number) => {
    setSelectedPreset(val)
    setAmountInput(String(val))
  }

  const handleCustomChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const v = e.target.value
    setAmountInput(v)
    const num = parseFloat(v)
    if (PRESET_AMOUNTS.includes(num)) {
      setSelectedPreset(num)
    } else {
      setSelectedPreset(null)
    }
  }

  const handleCreateOrder = useCallback(() => {
    if (!isValidAmount) {
      toast.error("请输入有效的充值金额（最低 $1.00）")
      return
    }

    if (method === "alipay") {
      createAlipay.mutate(parsedAmount, {
        onSuccess: (res) => {
          setAlipayOrder(res)
          setAlipayPolling(true)
          toast.success("支付宝订单已创建，请扫码支付")
        },
      })
    } else if (method === "wechat") {
      createWechat.mutate(parsedAmount, {
        onSuccess: (res) => {
          setWechatOrder(res)
          setWechatPolling(true)
          toast.success("微信支付订单已创建，请扫码支付")
        },
      })
    }
  }, [isValidAmount, method, parsedAmount, createAlipay, createWechat])

  const handleResetOrder = () => {
    setAlipayPolling(false)
    setAlipayOrder(null)
    setWechatPolling(false)
    setWechatOrder(null)
  }

  const copyOrderId = (id: string) => {
    void navigator.clipboard.writeText(id)
    toast.success("订单号已复制到剪贴板")
  }

  // Check if there is an active QR order
  const activeQrOrder =
    method === "alipay" ? alipayOrder : method === "wechat" ? wechatOrder : null
  const activeStatus =
    method === "alipay" ? alipayStatus : method === "wechat" ? wechatStatus : null
  const isPolling =
    method === "alipay" ? alipayPolling : method === "wechat" ? wechatPolling : false
  const isPendingCreation =
    createAlipay.isPending || createWechat.isPending

  return (
    <Card className="rounded-2xl border border-border/80 bg-card p-6 sm:p-7 shadow-xs space-y-6">
      <CardHeader className="p-0 space-y-1">
        <div className="flex items-center justify-between">
          <CardTitle className="text-xl font-bold tracking-tight text-foreground flex items-center gap-2">
            <CreditCard className="w-5 h-5 text-primary" />
            在线快速充值
          </CardTitle>
          <Badge variant="secondary" className="text-xs font-semibold px-2.5 py-0.5 rounded-full">
            实时秒级到账
          </Badge>
        </div>
        <CardDescription className="text-sm text-muted-foreground">
          选择充值金额并使用手机扫码完成支付，资金实时入账并支持全平台模型抵扣。
        </CardDescription>
      </CardHeader>

      <CardContent className="p-0 space-y-6">
        {/* Step 1: Amount Selection (always accessible) */}
        <div className="space-y-3">
          <label className="text-xs font-bold uppercase tracking-wider text-muted-foreground">
            1. 选择或输入充值金额 (USD)
          </label>

          {/* Preset Chips */}
          <div className="grid grid-cols-3 sm:grid-cols-6 gap-2">
            {PRESET_AMOUNTS.map((preset) => {
              const isSelected = selectedPreset === preset
              return (
                <button
                  key={preset}
                  type="button"
                  onClick={() => handleSelectPreset(preset)}
                  className={`h-11 rounded-xl text-sm font-bold transition-all relative flex items-center justify-center ${
                    isSelected
                      ? "bg-primary text-primary-foreground shadow-xs ring-2 ring-primary/20"
                      : "bg-muted/50 hover:bg-muted text-foreground border border-border/80"
                  }`}
                >
                  ${preset}
                  {preset === 50 && (
                    <span
                      className={`absolute -top-2 -right-1 text-[9px] px-1.5 py-0.2 rounded-full font-extrabold uppercase ${
                        isSelected
                          ? "bg-white text-primary"
                          : "bg-primary text-primary-foreground"
                      }`}
                    >
                      常用
                    </span>
                  )}
                </button>
              )
            })}
          </div>

          {/* Custom Input */}
          <div className="relative">
            <span className="absolute left-3.5 top-1/2 -translate-y-1/2 text-muted-foreground text-sm font-semibold">
              $
            </span>
            <Input
              type="number"
              min="1"
              step="0.01"
              placeholder="其他自定义金额（最低 $1.00）"
              value={amountInput}
              onChange={handleCustomChange}
              className="pl-8 h-11 text-sm font-semibold tabular-nums rounded-xl border-border bg-background"
            />
          </div>

          <p className="text-xs text-muted-foreground flex items-center gap-1.5">
            <ShieldCheck className="w-3.5 h-3.5 text-primary shrink-0" />
            实际扣款按支付时实时汇率折算为人民币 (CNY) · 最低 $1.00 起充
          </p>
        </div>

        {/* Step 2: Payment Method Selection */}
        <div className="space-y-3">
          <label className="text-xs font-bold uppercase tracking-wider text-muted-foreground">
            2. 选择支付渠道
          </label>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {/* Alipay Card */}
            <button
              type="button"
              onClick={() => {
                setMethod("alipay")
                handleResetOrder()
              }}
              className={`flex items-center gap-3.5 p-3.5 rounded-xl border text-left transition-all ${
                method === "alipay"
                  ? "border-primary bg-primary/5 ring-2 ring-primary/20 shadow-xs"
                  : "border-border/80 bg-card hover:bg-muted/40"
              }`}
            >
              <AlipayLogo className="w-9 h-9 rounded-xl shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-bold text-foreground">支付宝</span>
                  <span className="text-[10px] bg-primary/10 text-primary font-bold px-1.5 py-0.5 rounded">
                    推荐
                  </span>
                </div>
                <p className="text-xs text-muted-foreground mt-0.5 truncate">
                  支持支付宝扫码 / 花呗 / 余额
                </p>
              </div>
            </button>

            {/* WeChat Pay Card */}
            <button
              type="button"
              onClick={() => {
                setMethod("wechat")
                handleResetOrder()
              }}
              className={`flex items-center gap-3.5 p-3.5 rounded-xl border text-left transition-all ${
                method === "wechat"
                  ? "border-emerald-600 bg-emerald-500/5 ring-2 ring-emerald-500/20 shadow-xs"
                  : "border-border/80 bg-card hover:bg-muted/40"
              }`}
            >
              <WechatPayLogo className="w-9 h-9 rounded-xl shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-bold text-foreground">微信支付</span>
                </div>
                <p className="text-xs text-muted-foreground mt-0.5 truncate">
                  微信 App 扫一扫即时到账
                </p>
              </div>
            </button>

            {/* Stripe Card (if enabled) */}
            {stripeConfig?.enabled && (
              <button
                type="button"
                onClick={() => {
                  setMethod("stripe")
                  handleResetOrder()
                }}
                className={`sm:col-span-2 flex items-center gap-3.5 p-3.5 rounded-xl border text-left transition-all ${
                  (method as PaymentMethod) === "stripe"
                    ? "border-primary bg-primary/5 ring-2 ring-primary/20 shadow-xs"
                    : "border-border/80 bg-card hover:bg-muted/40"
                }`}
              >
                <div className="w-9 h-9 rounded-xl bg-muted flex items-center justify-center text-primary shrink-0">
                  <CreditCard className="w-5 h-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-bold text-foreground">国际信用卡 (Stripe)</span>
                    <span className="text-[10px] bg-muted text-muted-foreground font-bold px-1.5 py-0.5 rounded">
                      USD
                    </span>
                  </div>
                  <p className="text-xs text-muted-foreground mt-0.5 truncate">
                    支持 Visa, MasterCard, American Express
                  </p>
                </div>
              </button>
            )}
          </div>
        </div>

        {/* Step 3: Action or Order Display */}
        {method === "stripe" ? (
          <div className="space-y-4 pt-2">
            <StripePayment onSuccess={onSuccess} />
          </div>
        ) : activeQrOrder ? (
          /* Active QR Order Display */
          <div className="rounded-2xl border border-border/80 bg-muted/20 p-6 space-y-6 animate-in fade-in duration-200">
            {/* Order status banner */}
            <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2 pb-4 border-b border-border/60">
              <div className="flex items-center gap-2">
                {method === "alipay" ? (
                  <AlipayLogo className="w-5 h-5 rounded-md" />
                ) : (
                  <WechatPayLogo className="w-5 h-5 rounded-md" />
                )}
                <span className="text-sm font-semibold text-foreground">
                  {method === "alipay" ? "支付宝扫码支付" : "微信扫码支付"}
                </span>
              </div>
              <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <span>订单号：</span>
                <span className="font-mono text-foreground font-medium">{activeQrOrder.order_id}</span>
                <button
                  type="button"
                  onClick={() => copyOrderId(activeQrOrder.order_id)}
                  className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"
                  title="复制订单号"
                >
                  <Copy className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>

            {/* If paid successfully */}
            {activeStatus === "paid" ? (
              <div className="flex flex-col items-center justify-center py-6 space-y-4 text-center">
                <div className="w-16 h-16 rounded-full bg-emerald-500/10 flex items-center justify-center text-emerald-600 dark:text-emerald-400">
                  <CheckCircle2 className="w-10 h-10" />
                </div>
                <div className="space-y-1">
                  <h3 className="text-xl font-bold text-foreground">充值成功！</h3>
                  <p className="text-sm text-muted-foreground">
                    已成功充值 ${activeQrOrder.amount_usd.toFixed(2)} USD 到账户余额
                  </p>
                </div>
                <Button onClick={handleResetOrder} className="rounded-xl mt-2">
                  继续充值
                </Button>
              </div>
            ) : activeStatus === "failed" ? (
              <div className="flex flex-col items-center justify-center py-6 space-y-4 text-center">
                <div className="w-16 h-16 rounded-full bg-destructive/10 flex items-center justify-center text-destructive">
                  <XCircle className="w-10 h-10" />
                </div>
                <div className="space-y-1">
                  <h3 className="text-xl font-bold text-foreground">支付失败或已关闭</h3>
                  <p className="text-sm text-muted-foreground">
                    订单未成功完成支付，请重新生成或更换支付渠道。
                  </p>
                </div>
                <Button variant="outline" onClick={handleResetOrder} className="rounded-xl mt-2">
                  重新发起支付
                </Button>
              </div>
            ) : (
              /* Waiting for payment QR code presentation */
              <div className="flex flex-col items-center justify-center space-y-4 py-2">
                <div className="rounded-2xl border border-border/80 bg-white p-4 shadow-sm">
                  <QRCodeSVG
                    value={"qr_code" in activeQrOrder ? activeQrOrder.qr_code : activeQrOrder.code_url}
                    size={190}
                    level="M"
                  />
                </div>

                <div className="text-center space-y-1">
                  <p className="text-sm font-semibold text-foreground flex items-center justify-center gap-1.5">
                    <Smartphone className="w-4 h-4 text-primary" />
                    请使用手机【{method === "alipay" ? "支付宝" : "微信"}】扫码完成支付
                  </p>
                  <div className="text-xs text-muted-foreground">
                    应付金额：
                    <span className="font-bold text-foreground tabular-nums">
                      ${activeQrOrder.amount_usd.toFixed(2)} USD
                    </span>
                    {activeQrOrder.amount_local > 0 && (
                      <span className="ml-1">
                        （折合约 ¥{activeQrOrder.amount_local.toFixed(2)} {activeQrOrder.currency}）
                      </span>
                    )}
                  </div>
                </div>

                {/* Alipay direct browser link */}
                {"pay_url" in activeQrOrder && activeQrOrder.pay_url && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="gap-1.5 rounded-xl border-border bg-card"
                    onClick={() => window.open(activeQrOrder.pay_url, "_blank")}
                  >
                    <ExternalLink className="w-3.5 h-3.5 text-primary" />
                    在浏览器新窗口打开支付页
                  </Button>
                )}

                <div className="flex items-center justify-center gap-2 text-xs text-muted-foreground pt-1">
                  <RefreshCw className={`w-3.5 h-3.5 text-primary ${isPolling ? "animate-spin" : ""}`} />
                  <span>{isPolling ? "正在自动轮询到账状态（每 3 秒）..." : "轮询已暂停"}</span>
                </div>

                <div className="pt-3 border-t border-border/60 w-full flex justify-center">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={handleResetOrder}
                    className="text-xs text-muted-foreground hover:text-foreground"
                  >
                    更换充值金额或支付渠道
                  </Button>
                </div>
              </div>
            )}
          </div>
        ) : (
          /* Normal Action Button */
          <div className="pt-2">
            <Button
              onClick={handleCreateOrder}
              disabled={!isValidAmount || isPendingCreation}
              className="w-full h-12 text-sm font-bold rounded-xl gap-2 shadow-xs"
            >
              {isPendingCreation ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  正在创建订单...
                </>
              ) : (
                <>
                  <span>
                    {method === "alipay" ? "生成支付宝二维码" : "生成微信支付二维码"} · $
                    {isValidAmount ? parsedAmount.toFixed(2) : "0.00"} USD
                  </span>
                  <ArrowRight className="w-4 h-4" />
                </>
              )}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  )
}
