import { useState, useEffect, useCallback } from "react"
import { toast } from "sonner"
import { QRCodeSVG } from "qrcode.react"

import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Badge } from "@/shared/components/ui/badge"
import {
  CircleDollarSign,
  Loader2,
  ExternalLink,
  CheckCircle2,
  XCircle,
  RefreshCw,
  Smartphone,
} from "lucide-react"
import { useCreateAlipayOrder, useAlipayOrderStatus } from "@/features/payment/hooks"
import type { AlipayCreateResponse } from "@/features/payment/types"

interface AlipayPaymentProps {
  initialOrderId?: string | null
  onSuccess?: () => void
}

export default function AlipayPayment({ initialOrderId, onSuccess }: AlipayPaymentProps) {
  const [amount, setAmount] = useState("")
  const [order, setOrder] = useState<AlipayCreateResponse | null>(null)
  const [polling, setPolling] = useState(false)

  const createOrder = useCreateAlipayOrder()

  // If initialOrderId is provided, start polling immediately
  const activeOrderId = order?.order_id ?? initialOrderId ?? null
  const statusQuery = useAlipayOrderStatus(activeOrderId, polling)
  const status = statusQuery.data?.status ?? null

  // Start polling when initialOrderId is provided
  useEffect(() => {
    if (initialOrderId) {
      setPolling(true)
    }
  }, [initialOrderId])

  // Handle status transitions
  useEffect(() => {
    if (status === "paid") {
      setPolling(false)
      toast.success(`支付宝支付成功！充值 $${statusQuery.data!.amount.toFixed(2)}`)
      onSuccess?.()
    } else if (status === "failed") {
      setPolling(false)
      toast.error("支付宝支付失败")
    }
  }, [status, statusQuery.data, onSuccess])

  const handleCreateOrder = useCallback(() => {
    const val = parseFloat(amount)
    if (!amount || isNaN(val) || val <= 0) {
      toast.error("请输入有效的充值金额")
      return
    }

    createOrder.mutate(val, {
      onSuccess: (res) => {
        setOrder(res)
        setPolling(true)
        toast.success("订单已创建，请使用支付宝扫码支付")
      },
    })
  }, [amount, createOrder])

  const handleReset = useCallback(() => {
    setPolling(false)
    setOrder(null)
    setAmount("")
  }, [])

  const statusBadge = () => {
    if (!status && !polling) return null
    switch (status) {
      case "paid":
        return (
          <Badge variant="default" className="bg-green-500 hover:bg-green-600 gap-1">
            <CheckCircle2 className="w-3.5 h-3.5" />
            支付成功
          </Badge>
        )
      case "failed":
        return (
          <Badge variant="destructive" className="gap-1">
            <XCircle className="w-3.5 h-3.5" />
            支付失败
          </Badge>
        )
      default:
        return (
          <Badge variant="secondary" className="gap-1">
            <RefreshCw className="w-3.5 h-3.5 animate-spin" />
            等待支付
          </Badge>
        )
    }
  }

  return (
    <Card className="shadow-sm border-border">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <CircleDollarSign className="h-5 w-5 text-primary" />
          支付宝充值
        </CardTitle>
        <CardDescription>
          输入充值金额，使用支付宝扫码完成支付。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        {!order && !initialOrderId && (
          <div className="space-y-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">充值金额 (USD)</label>
              <div className="relative">
                <span className="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground text-sm">$</span>
                <Input
                  type="number"
                  min="1"
                  step="0.01"
                  placeholder="例如：10.00"
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  className="pl-7"
                  disabled={createOrder.isPending}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                实际扣款将以人民币结算，汇率以支付时为准。
              </p>
            </div>

            <Button
              onClick={handleCreateOrder}
              disabled={createOrder.isPending || !amount || parseFloat(amount) <= 0}
              className="w-full gap-2"
            >
              {createOrder.isPending ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  创建订单中...
                </>
              ) : (
                <>
                  <Smartphone className="w-4 h-4" />
                  生成支付二维码
                </>
              )}
            </Button>
          </div>
        )}

        {(order || initialOrderId) && (
          <div className="space-y-5 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <div className="flex items-center justify-between">
              <div className="text-sm text-muted-foreground">
                订单号：<span className="font-mono text-foreground">{activeOrderId}</span>
              </div>
              {statusBadge()}
            </div>

            {(!status || status === "pending") && (
              <>
                <div className="flex flex-col items-center justify-center space-y-3 py-2">
                  {order?.qr_code ? (
                    <div className="p-4 bg-white rounded-xl border border-border">
                      <QRCodeSVG value={order.qr_code} size={180} level="M" />
                    </div>
                  ) : (
                    <div className="p-4 bg-muted rounded-xl">
                      <Loader2 className="w-12 h-12 animate-spin text-muted-foreground" />
                    </div>
                  )}
                  <p className="text-sm text-muted-foreground">
                    请使用支付宝扫描上方二维码完成支付
                  </p>
                  {order?.pay_url && (
                    <Button
                      variant="outline"
                      size="sm"
                      className="gap-1"
                      onClick={() => window.open(order.pay_url, "_blank")}
                    >
                      <ExternalLink className="w-3.5 h-3.5" />
                      在浏览器中打开支付页
                    </Button>
                  )}
                </div>

                <div className="flex items-center justify-center gap-2 text-xs text-muted-foreground">
                  <RefreshCw className={`w-3.5 h-3.5 ${polling ? "animate-spin" : ""}`} />
                  {polling ? "正在轮询支付状态（每 3 秒）..." : "轮询已停止"}
                </div>
              </>
            )}

            {status === "paid" && (
              <div className="flex flex-col items-center justify-center space-y-3 py-4">
                <div className="w-16 h-16 rounded-full bg-green-100 dark:bg-green-900/30 flex items-center justify-center">
                  <CheckCircle2 className="w-8 h-8 text-green-600 dark:text-green-400" />
                </div>
                <div className="text-center">
                  <p className="text-lg font-semibold text-green-600 dark:text-green-400">
                    支付成功！
                  </p>
                  <p className="text-sm text-muted-foreground mt-1">
                    已充值 ${order?.amount_usd?.toFixed(2) ?? statusQuery.data?.amount?.toFixed(2)} USD
                    {order && order.amount_local > 0 && (
                      <span>（约 ¥{order.amount_local.toFixed(2)} {order.currency}）</span>
                    )}
                  </p>
                  {statusQuery.data?.paid_at && (
                    <p className="text-xs text-muted-foreground mt-1">
                      支付时间：{new Date(statusQuery.data.paid_at).toLocaleString()}
                    </p>
                  )}
                </div>
                <Button variant="outline" onClick={handleReset}>
                  继续充值
                </Button>
              </div>
            )}

            {status === "failed" && (
              <div className="flex flex-col items-center justify-center space-y-3 py-4">
                <div className="w-16 h-16 rounded-full bg-red-100 dark:bg-red-900/30 flex items-center justify-center">
                  <XCircle className="w-8 h-8 text-red-600 dark:text-red-400" />
                </div>
                <div className="text-center">
                  <p className="text-lg font-semibold text-red-600 dark:text-red-400">
                    支付失败
                  </p>
                  <p className="text-sm text-muted-foreground mt-1">
                    订单未成功完成，请重试或联系客服。
                  </p>
                </div>
                <Button variant="outline" onClick={handleReset}>
                  重新尝试
                </Button>
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
