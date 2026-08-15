// Constants for user orders

import { Clock, CheckCircle2, XCircle, AlertCircle } from "lucide-react"
import type { Subscription } from "./types"

const PROVIDER_MAP: Record<string, { label: string; color: string }> = {
  stripe:  { label: 'Stripe', color: 'text-blue-500' },
  alipay:  { label: '支付宝', color: 'text-sky-500' },
  wechat:  { label: '微信支付', color: 'text-emerald-500' },
}

const STATUS_MAP: Record<string, { label: string; color: string; bg: string; icon: React.ElementType }> = {
  pending:  { label: '待支付', color: 'text-amber-600 dark:text-amber-400', bg: 'bg-amber-50 dark:bg-amber-900/20', icon: Clock },
  paid:     { label: '已支付', color: 'text-emerald-600 dark:text-emerald-400', bg: 'bg-emerald-50 dark:bg-emerald-900/20', icon: CheckCircle2 },
  failed:   { label: '失败',   color: 'text-red-600 dark:text-red-400',      bg: 'bg-red-50 dark:bg-red-900/20',      icon: XCircle },
  refunded: { label: '已退款', color: 'text-gray-600 dark:text-gray-400',    bg: 'bg-gray-100 dark:bg-dark-700',      icon: AlertCircle },
}

export function getProviderInfo(p: string) {
  return PROVIDER_MAP[p] || { label: p, color: 'text-gray-500' }
}

export function getStatusInfo(s: string) {
  return STATUS_MAP[s] || { label: s, color: 'text-gray-500', bg: 'bg-gray-100 dark:bg-dark-700', icon: AlertCircle }
}

function daysBetween(a: string | Date, b: string | Date): number {
  const d1 = new Date(a).getTime()
  const d2 = new Date(b).getTime()
  const diff = d2 - d1
  return Math.max(0, Math.ceil(diff / (1000 * 60 * 60 * 24)))
}

export function calculateRefund(sub: Subscription): number {
  if (sub.status !== 'active') return 0
  const now = new Date()
  const expiresAt = new Date(sub.expires_at)
  if (now >= expiresAt) return 0
  const totalDays = daysBetween(sub.starts_at, sub.expires_at) || 1
  const dailyRate = sub.price_paid / totalDays
  const remainingDays = daysBetween(now, expiresAt)
  const amount = remainingDays * dailyRate
  if (amount <= 0) return 0
  return Math.min(amount, sub.price_paid)
}

export function fmtAmount(n: number): string {
  return `$${n.toFixed(2)}`
}

export function fmtLocalAmount(n: number, currency: string): string {
  return `${n.toFixed(2)} ${currency}`
}

export function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}
