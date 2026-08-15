// Constants for admin subscriptions

/** 管理员分配订阅时的资金来源（与后端 subscriptionFundingSourceSet 一致） */
export const SUBSCRIPTION_FUNDING_SOURCE_OPTIONS = [
  { value: "wechat_transfer", label: "微信转账/个人收款" },
  { value: "alipay_transfer", label: "支付宝转账" },
  { value: "bank_transfer", label: "银行对公转账" },
  { value: "platform_balance", label: "用户余额内扣（已入账）" },
  { value: "stripe", label: "Stripe 卡付" },
  { value: "coupon_comp", label: "活动/补偿/免费开通" },
  { value: "other", label: "其他（须填单号或备注）" },
] as const

export function subscriptionFundingSourceLabel(code: string | undefined | null): string {
  if (code == null || code === "") return "—"
  const row = SUBSCRIPTION_FUNDING_SOURCE_OPTIONS.find((o) => o.value === code)
  if (row) return row.label
  if (code === "legacy_unspecified") return "历史/未标注"
  return code
}

export function formatLimit(v?: number | null): string {
  return v != null ? `$${v.toFixed(2)}` : "∞"
}

export function daysRemaining(expiresAt: string): number {
  const diff = new Date(expiresAt).getTime() - Date.now()
  return Math.max(0, Math.ceil(diff / (1000 * 60 * 60 * 24)))
}

export function usagePercent(usage: number, limit?: number | null): number | null {
  if (!limit || limit <= 0) return null
  return Math.min(100, (usage / limit) * 100)
}
