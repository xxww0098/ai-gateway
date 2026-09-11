import { useState, useEffect } from "react"
import { Link } from "react-router-dom"
import {
  X,
  Copy,
  Check,
  Receipt,
  CreditCard,
  ArrowRight,
  ShieldCheck,
  Calendar,
  Hash,
  Coins,
} from "lucide-react"
import { toast } from "sonner"
import { getStatusInfo, getProviderInfo, fmtAmount, fmtLocalAmount, fmtDateTime } from "../constants"
import { OrderStatusBadge } from "./OrderStatusBadge"
import { userRoutes } from "@/shared/routes/user"
import type { PaymentOrder } from "../types"

interface Props {
  order: PaymentOrder
  onClose: () => void
}

export function OrderDetailDrawer({ order, onClose }: Props) {
  const [copiedField, setCopiedField] = useState<string | null>(null)
  const sInfo = getStatusInfo(order.status)
  const pInfo = getProviderInfo(order.provider)
  const isPending = order.status === "pending"
  const isPaid = order.status === "paid"

  // Close on Escape key
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose()
    }
    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [onClose])

  const copyToClipboard = (text: string, label: string) => {
    void navigator.clipboard.writeText(text)
    setCopiedField(label)
    toast.success(`已复制 ${label}`)
    setTimeout(() => setCopiedField(null), 2000)
  }

  const handleCopySummary = () => {
    const summary = [
      `【AI-GateWay 充值订单凭证】`,
      `订单编号: #${order.id}`,
      `支付渠道: ${pInfo.label}`,
      `充值金额: ${fmtAmount(order.amount_usd)} (${fmtLocalAmount(order.amount_local, order.currency)})`,
      `当前状态: ${sInfo.label}`,
      `交易单号: ${order.transaction_id || "无"}`,
      `创建时间: ${fmtDateTime(order.created_at)}`,
      order.paid_at ? `支付时间: ${fmtDateTime(order.paid_at)}` : "",
    ]
      .filter(Boolean)
      .join("\n")

    void navigator.clipboard.writeText(summary)
    toast.success("已复制完整凭证信息")
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end animate-in fade-in duration-200">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/40 backdrop-blur-xs transition-opacity"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Drawer Panel */}
      <div className="relative w-full max-w-md bg-card h-full shadow-2xl border-l border-border flex flex-col animate-in slide-in-from-right duration-300 z-10">
        {/* Drawer Header */}
        <div className="px-6 py-4.5 border-b border-border flex items-center justify-between bg-card">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-xl bg-primary/10 text-primary flex items-center justify-center">
              <Receipt className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-base font-bold text-foreground tracking-tight">充值订单凭证</h2>
              <p className="text-xs text-muted-foreground font-mono tabular-nums">
                单号 #{order.id}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭详情"
            className="h-8 w-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Scrollable Receipt Body */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6">
          {/* Digital Voucher Card */}
          <div className="rounded-2xl border border-border bg-muted/30 p-5 space-y-5 relative">
            {/* Amount display */}
            <div className="text-center space-y-2 pb-4 border-b border-dashed border-border">
              <div className="flex items-center justify-center">
                <OrderStatusBadge order={order} size="sm" />
              </div>
              <div>
                <div className="text-3xl font-extrabold tabular-nums tracking-tight text-foreground">
                  {fmtAmount(order.amount_usd)}
                </div>
                <div className="inline-flex items-center gap-1.5 mt-1 text-xs font-semibold px-2.5 py-0.5 rounded-full bg-background border border-border text-muted-foreground tabular-nums shadow-2xs">
                  <Coins className="w-3 h-3 text-primary shrink-0" />
                  <span>{fmtLocalAmount(order.amount_local, order.currency)}</span>
                </div>
              </div>
            </div>

            {/* Status Context Banner */}
            <div className={`rounded-xl border p-3 text-xs space-y-1 ${sInfo.bg} ${sInfo.border} ${sInfo.color}`}>
              <div className="font-semibold flex items-center gap-1.5">
                <span className={`w-1.5 h-1.5 rounded-full ${sInfo.dot}`} />
                <span>{sInfo.label}</span>
              </div>
              <p className="opacity-90 leading-relaxed text-[11px]">{sInfo.desc}</p>
              {isPaid && order.paid_at && (
                <p className="text-[11px] font-medium opacity-80 pt-0.5">
                  到账时间：{fmtDateTime(order.paid_at)}
                </p>
              )}
            </div>

            {/* Itemized Grid */}
            <div className="space-y-3 text-xs">
              {/* Provider */}
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground flex items-center gap-1.5">
                  <CreditCard className="w-3.5 h-3.5 text-muted-foreground/80" />
                  支付渠道
                </span>
                <span
                  className={`inline-flex items-center px-2.5 py-0.5 rounded-lg border text-xs font-semibold ${pInfo.bg} ${pInfo.border} ${pInfo.color}`}
                >
                  {pInfo.label}
                </span>
              </div>

              {/* Transaction ID */}
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground flex items-center gap-1.5">
                  <Hash className="w-3.5 h-3.5 text-muted-foreground/80" />
                  交易单号
                </span>
                {order.transaction_id ? (
                  <button
                    type="button"
                    onClick={() => copyToClipboard(order.transaction_id!, "交易单号")}
                    className="inline-flex items-center gap-1.5 font-mono text-xs font-semibold text-foreground hover:text-primary transition-colors max-w-[200px] truncate group"
                    title={order.transaction_id}
                  >
                    <span className="truncate">{order.transaction_id}</span>
                    {copiedField === "交易单号" ? (
                      <Check className="w-3.5 h-3.5 text-emerald-500 shrink-0" />
                    ) : (
                      <Copy className="w-3.5 h-3.5 text-muted-foreground group-hover:text-primary shrink-0" />
                    )}
                  </button>
                ) : (
                  <span className="text-muted-foreground font-mono">-</span>
                )}
              </div>

              {/* Order ID */}
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground flex items-center gap-1.5">
                  <Receipt className="w-3.5 h-3.5 text-muted-foreground/80" />
                  系统单号
                </span>
                <button
                  type="button"
                  onClick={() => copyToClipboard(String(order.id), "系统单号")}
                  className="inline-flex items-center gap-1 font-mono text-xs font-semibold text-foreground hover:text-primary transition-colors group"
                >
                  <span>#{order.id}</span>
                  {copiedField === "系统单号" ? (
                    <Check className="w-3 h-3 text-emerald-500" />
                  ) : (
                    <Copy className="w-3 h-3 text-muted-foreground group-hover:text-primary" />
                  )}
                </button>
              </div>

              {/* Created At */}
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground flex items-center gap-1.5">
                  <Calendar className="w-3.5 h-3.5 text-muted-foreground/80" />
                  创建时间
                </span>
                <span className="font-mono text-foreground tabular-nums">
                  {fmtDateTime(order.created_at)}
                </span>
              </div>

              {/* Paid At */}
              {order.paid_at && (
                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground flex items-center gap-1.5">
                    <ShieldCheck className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                    支付时间
                  </span>
                  <span className="font-mono text-foreground tabular-nums">
                    {fmtDateTime(order.paid_at)}
                  </span>
                </div>
              )}
            </div>
          </div>

          {/* Pending Guidance */}
          {isPending && (
            <div className="rounded-xl border border-border bg-card p-4 space-y-2.5">
              <div className="flex items-center justify-between">
                <span className="text-xs font-semibold text-foreground">尚未完成支付？</span>
                <Link
                  to={userRoutes.financeTopup}
                  className="inline-flex items-center gap-1 text-xs font-semibold text-primary hover:underline"
                >
                  去收银台
                  <ArrowRight className="w-3 h-3" />
                </Link>
              </div>
              <p className="text-[11px] text-muted-foreground leading-relaxed">
                若您已通过扫码或跳转完成扣款，网关通常在数秒内收到回调并自动核销入账。若超过5分钟未入账，请联系客服并提供上方交易单号。
              </p>
            </div>
          )}

          {/* Metadata if present */}
          {order.metadata && (
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <p className="text-xs font-bold text-muted-foreground uppercase tracking-wider">
                  元数据 (Metadata)
                </p>
                <button
                  type="button"
                  onClick={() => copyToClipboard(order.metadata!, "元数据")}
                  className="text-xs text-muted-foreground hover:text-foreground inline-flex items-center gap-1 font-medium transition-colors"
                >
                  <Copy className="w-3 h-3" />
                  复制 JSON
                </button>
              </div>
              <pre className="rounded-xl border border-border bg-muted/60 p-3.5 font-mono text-[11px] text-foreground overflow-x-auto leading-relaxed max-h-48">
                {order.metadata}
              </pre>
            </div>
          )}
        </div>

        {/* Drawer Footer Actions */}
        <div className="px-6 py-4 border-t border-border bg-card flex items-center gap-2.5">
          <button
            type="button"
            onClick={handleCopySummary}
            className="flex-1 btn btn-secondary h-9 text-xs font-semibold gap-1.5 shadow-2xs"
          >
            <Copy className="w-3.5 h-3.5" />
            复制凭证
          </button>
          <button
            type="button"
            onClick={onClose}
            className="btn btn-primary h-9 px-6 text-xs font-semibold shadow-xs"
          >
            关闭
          </button>
        </div>
      </div>
    </div>
  )
}

