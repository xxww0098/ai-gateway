// Constants for user orders

import { Clock, CheckCircle2, XCircle, AlertCircle } from "lucide-react"
import type { Subscription } from "./types"

export interface ProviderInfo {
  label: string
  color: string
  bg: string
  border: string
}

export interface StatusInfo {
  label: string
  color: string
  bg: string
  border: string
  dot: string
  desc: string
  icon: React.ElementType
}

const PROVIDER_MAP: Record<string, ProviderInfo> = {
  stripe: {
    label: 'Stripe',
    color: 'text-indigo-600 dark:text-indigo-400',
    bg: 'bg-indigo-50 dark:bg-indigo-950/40',
    border: 'border-indigo-200/80 dark:border-indigo-800/50',
  },
  alipay: {
    label: '支付宝',
    color: 'text-sky-600 dark:text-sky-400',
    bg: 'bg-sky-50 dark:bg-sky-950/40',
    border: 'border-sky-200/80 dark:border-sky-800/50',
  },
  wechat: {
    label: '微信支付',
    color: 'text-emerald-600 dark:text-emerald-400',
    bg: 'bg-emerald-50 dark:bg-emerald-950/40',
    border: 'border-emerald-200/80 dark:border-emerald-800/50',
  },
}

const STATUS_MAP: Record<string, StatusInfo> = {
  pending: {
    label: '待支付',
    color: 'text-amber-700 dark:text-amber-300',
    bg: 'bg-amber-50/80 dark:bg-amber-950/30',
    border: 'border-amber-200/80 dark:border-amber-800/50',
    dot: 'bg-amber-500',
    desc: '订单等待支付确认。支付完成后额度将自动入账',
    icon: Clock,
  },
  paid: {
    label: '已支付',
    color: 'text-emerald-700 dark:text-emerald-300',
    bg: 'bg-emerald-50/80 dark:bg-emerald-950/30',
    border: 'border-emerald-200/80 dark:border-emerald-800/50',
    dot: 'bg-emerald-500',
    desc: '支付成功，额度已实时计入账户可用余额',
    icon: CheckCircle2,
  },
  failed: {
    label: '失败',
    color: 'text-rose-700 dark:text-rose-300',
    bg: 'bg-rose-50/80 dark:bg-rose-950/30',
    border: 'border-rose-200/80 dark:border-rose-800/50',
    dot: 'bg-rose-500',
    desc: '订单支付未完成或已取消',
    icon: XCircle,
  },
  refunded: {
    label: '已退款',
    color: 'text-slate-700 dark:text-slate-300',
    bg: 'bg-slate-100/80 dark:bg-slate-800/40',
    border: 'border-slate-200/80 dark:border-slate-700/60',
    dot: 'bg-slate-400',
    desc: '款项已按退款策略原路退回',
    icon: AlertCircle,
  },
}

export function getProviderInfo(p: string): ProviderInfo {
  return PROVIDER_MAP[p] || {
    label: p || '未知渠道',
    color: 'text-muted-foreground',
    bg: 'bg-muted/60',
    border: 'border-border',
  }
}

export function getStatusInfo(s: string): StatusInfo {
  return (
    STATUS_MAP[s] || {
      label: s || '未知状态',
      color: 'text-muted-foreground',
      bg: 'bg-muted/60',
      border: 'border-border',
      dot: 'bg-muted-foreground',
      desc: '状态信息未知',
      icon: AlertCircle,
    }
  )
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
