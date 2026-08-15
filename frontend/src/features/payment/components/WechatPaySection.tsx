import { useState, useEffect, useCallback } from "react"
import { QRCodeSVG } from "qrcode.react"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { toast } from "sonner"
import { MessageSquare, Loader2, CheckCircle2, XCircle, RefreshCw } from "lucide-react"
import { useCreateWechatOrder, useWechatOrderStatus } from "@/features/payment/hooks"
import type { WechatCreateResponse } from "@/features/payment/types"

interface WechatPaySectionProps {
  onSuccess?: () => void
}

export default function WechatPaySection({ onSuccess }: WechatPaySectionProps) {
  const [amount, setAmount] = useState("")
  const [order, setOrder] = useState<WechatCreateResponse | null>(null)
  const [polling, setPolling] = useState(false)

  const createOrder = useCreateWechatOrder()

  const statusQuery = useWechatOrderStatus(order?.order_id ?? null, polling)
  const status = statusQuery.data?.status ?? null

  // Handle status transitions
  useEffect(() => {
    if (status === "paid") {
      setPolling(false)
      toast.success(`微信支付成功！已充值 $${statusQuery.data!.amount.toFixed(2)}`)
      onSuccess?.()
    } else if (status === "failed") {
      setPolling(false)
      toast.error("支付失败，请重试")
    }
  }, [status, statusQuery.data, onSuccess])

  const handleCreateOrder = useCallback(async () => {
    const val = parseFloat(amount)
    if (!val || val <= 0 || isNaN(val)) {
      toast.error("请输入有效的充值金额")
      return
    }

    createOrder.mutate(val, {
      onSuccess: (res) => {
        setOrder(res)
        setPolling(true)
      },
    })
  }, [amount, createOrder])

  const handleReset = useCallback(() => {
    setPolling(false)
    setOrder(null)
    setAmount("")
  }, [])

  return (
    <Card className="shadow-sm border-border">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <MessageSquare className="h-5 w-5 text-green-600" />
          微信支付
        </CardTitle>
        <CardDescription>
          使用微信扫码支付，即时到账。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {!order && (
          <div className="space-y-4">
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium text-foreground">充值金额 (USD)</label>
                <span className="text-xs text-muted-foreground">最低 $1</span>
              </div>
              <div className="flex gap-2">
                <Input
                  type="number"
                  min={1}
                  step={1}
                  placeholder="例如：50"
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  className="flex-1"
                  disabled={createOrder.isPending}
                />
                <Button
                  onClick={handleCreateOrder}
                  disabled={createOrder.isPending || !amount}
                  className="shrink-0"
                >
                  {createOrder.isPending ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    "生成二维码"
                  )}
                </Button>
              </div>
            </div>

            <div className="flex gap-2">
              {[10, 50, 100, 500].map((preset) => (
                <Button
                  key={preset}
                  variant="outline"
                  size="sm"
                  onClick={() => setAmount(String(preset))}
                  disabled={createOrder.isPending}
                  className="flex-1"
                >
                  ${preset}
                </Button>
              ))}
            </div>
          </div>
        )}

        {order && (
          <div className="flex flex-col items-center space-y-4 animate-in fade-in zoom-in-95 duration-300">
            {status === "paid" ? (
              <div className="flex flex-col items-center space-y-3 py-6">
                <CheckCircle2 className="h-16 w-16 text-green-500" />
                <div className="text-xl font-bold text-foreground">支付成功</div>
                <div className="text-sm text-muted-foreground">
                  已充值 ${order.amount_usd.toFixed(2)} USD
                </div>
                <Button variant="outline" onClick={handleReset} className="mt-2">
                  <RefreshCw className="h-4 w-4 mr-1" />
                  继续充值
                </Button>
              </div>
            ) : status === "failed" ? (
              <div className="flex flex-col items-center space-y-3 py-6">
                <XCircle className="h-16 w-16 text-destructive" />
                <div className="text-xl font-bold text-foreground">支付失败</div>
                <div className="text-sm text-muted-foreground">
                  订单未成功，请检查支付状态或重试
                </div>
                <Button variant="outline" onClick={handleReset} className="mt-2">
                  <RefreshCw className="h-4 w-4 mr-1" />
                  重新支付
                </Button>
              </div>
            ) : (
              <>
                <div className="rounded-xl border border-border bg-white p-4 shadow-sm">
                  <QRCodeSVG
                    value={order.code_url}
                    size={200}
                    level="M"
                    marginSize={0}
                  />
                </div>
                <div className="text-center space-y-1">
                  <div className="text-sm font-medium text-foreground">
                    请使用微信扫一扫
                  </div>
                  <div className="text-xs text-muted-foreground">
                    订单金额：${order.amount_usd.toFixed(2)} USD
                    {order.currency !== "USD" && (
                      <span>（约 {order.amount_local.toFixed(2)} {order.currency}）</span>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  等待支付…
                </div>
              </>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
