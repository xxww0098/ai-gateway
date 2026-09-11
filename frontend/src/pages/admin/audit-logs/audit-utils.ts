import type { LucideIcon } from "lucide-react"
import {
  Shield,
  Zap,
  Coins,
  LogIn,
  UserPlus,
  Settings,
  Lock,
  Unlock,
  KeyRound,
  DollarSign,
  Database,
  Activity,
  Layers,
} from "lucide-react"

export type LogSource = "all" | "panel" | "sdk" | "balance"

export interface AuditEntry {
  id: string
  source: LogSource
  actor_id: number
  actor_email?: string
  action: string
  target?: string
  method?: string
  path?: string
  status_code?: number
  ip_address?: string
  request_id?: string
  metadata?: Record<string, unknown>
  created_at: string
}

export interface AuditResponse {
  items: AuditEntry[]
  total: number
  page: number
  page_size: number
  source: LogSource
}

export interface SourceMeta {
  label: string
  description: string
  icon: LucideIcon
  badgeClass: string
  activeCardClass: string
}

export const SOURCE_CONFIG: Record<LogSource, SourceMeta> = {
  all: {
    label: "全部",
    description: "所有层级审计流",
    icon: Layers,
    badgeClass: "bg-muted text-muted-foreground border-border",
    activeCardClass: "border-primary/40 bg-primary/5 ring-1 ring-primary/20",
  },
  panel: {
    label: "面板操作",
    description: "管理员配置与系统鉴权",
    icon: Shield,
    badgeClass: "bg-blue-500/10 text-blue-700 dark:text-blue-300 border-blue-200 dark:border-blue-900/50",
    activeCardClass: "border-blue-500/40 bg-blue-500/5 ring-1 ring-blue-500/20",
  },
  sdk: {
    label: "SDK 调用",
    description: "/v1/* 代理请求与模型调用",
    icon: Zap,
    badgeClass: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 border-emerald-200 dark:border-emerald-900/50",
    activeCardClass: "border-emerald-500/40 bg-emerald-500/5 ring-1 ring-emerald-500/20",
  },
  balance: {
    label: "余额变动",
    description: "Hold 冻结、结算与充值入账",
    icon: Coins,
    badgeClass: "bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-200 dark:border-amber-900/50",
    activeCardClass: "border-amber-500/40 bg-amber-500/5 ring-1 ring-amber-500/20",
  },
}

export interface ActionMeta {
  label: string
  icon: LucideIcon
  category: "auth" | "balance" | "sdk" | "admin" | "system"
}

export function getActionMeta(action: string): ActionMeta {
  const lower = action.toLowerCase()

  if (lower.startsWith("auth.login")) {
    return { label: "登录认证", icon: LogIn, category: "auth" }
  }
  if (lower.startsWith("auth.register")) {
    return { label: "用户注册", icon: UserPlus, category: "auth" }
  }
  if (lower.startsWith("auth.logout")) {
    return { label: "退出登录", icon: LogIn, category: "auth" }
  }
  if (lower.startsWith("auth.password")) {
    return { label: "修改密码", icon: KeyRound, category: "auth" }
  }

  if (lower.startsWith("balance:credit")) {
    return { label: "充值加款", icon: DollarSign, category: "balance" }
  }
  if (lower.startsWith("balance:debit")) {
    return { label: "资金扣减", icon: DollarSign, category: "balance" }
  }
  if (lower.startsWith("balance:hold")) {
    return { label: "资金冻结", icon: Lock, category: "balance" }
  }
  if (lower.startsWith("balance:settle")) {
    return { label: "扣费结算", icon: Coins, category: "balance" }
  }
  if (lower.startsWith("balance:release")) {
    return { label: "解冻返还", icon: Unlock, category: "balance" }
  }

  if (lower.startsWith("sdk:")) {
    return { label: "模型转发", icon: Zap, category: "sdk" }
  }

  if (lower.startsWith("admin.user")) {
    return { label: "用户管理", icon: UserPlus, category: "admin" }
  }
  if (lower.startsWith("admin.pricing") || lower.startsWith("admin.group")) {
    return { label: "定价策略", icon: Settings, category: "admin" }
  }
  if (lower.startsWith("admin.channel") || lower.startsWith("admin.provider")) {
    return { label: "渠道配置", icon: Database, category: "admin" }
  }
  if (lower.startsWith("admin.order")) {
    return { label: "订单处理", icon: DollarSign, category: "admin" }
  }

  return { label: action, icon: Activity, category: "system" }
}

export function formatRelativeTime(isoString: string): string {
  try {
    const d = new Date(isoString)
    const now = Date.now()
    const diff = Math.floor((now - d.getTime()) / 1000)

    if (diff < 15) return "刚刚"
    if (diff < 60) return `${diff} 秒前`
    if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`
    if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`
    if (diff < 86400 * 7) return `${Math.floor(diff / 86400)} 天前`

    const pad = (n: number) => String(n).padStart(2, "0")
    return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
  } catch {
    return "-"
  }
}

export function formatExactTime(isoString: string): string {
  try {
    const d = new Date(isoString)
    const pad = (n: number) => String(n).padStart(2, "0")
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  } catch {
    return "-"
  }
}

export function getMethodBadgeClass(method?: string): string {
  switch (method?.toUpperCase()) {
    case "POST":
      return "border-sky-200 bg-sky-50 text-sky-700 dark:border-sky-900/60 dark:bg-sky-950/40 dark:text-sky-300"
    case "GET":
      return "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-300"
    case "PUT":
    case "PATCH":
      return "border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-300"
    case "DELETE":
      return "border-rose-200 bg-rose-50 text-rose-700 dark:border-rose-900/60 dark:bg-rose-950/40 dark:text-rose-300"
    default:
      return "border-border bg-muted/60 text-muted-foreground"
  }
}

export function getStatusBadgeClass(code?: number): { badgeClass: string; dotClass: string; text: string } {
  if (!code) {
    return {
      badgeClass: "border-border bg-muted/50 text-muted-foreground",
      dotClass: "bg-muted-foreground",
      text: "-",
    }
  }

  if (code >= 200 && code < 300) {
    return {
      badgeClass: "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900/60 dark:bg-emerald-950/30 dark:text-emerald-300",
      dotClass: "bg-emerald-500",
      text: `${code}`,
    }
  }

  if (code >= 400 && code < 500) {
    return {
      badgeClass: "border-rose-200 bg-rose-50 text-rose-700 dark:border-rose-900/60 dark:bg-rose-950/30 dark:text-rose-300",
      dotClass: "bg-rose-500",
      text: `${code}`,
    }
  }

  if (code >= 500) {
    return {
      badgeClass: "border-red-300 bg-red-100 text-red-800 dark:border-red-800 dark:bg-red-950/50 dark:text-red-200",
      dotClass: "bg-red-600",
      text: `${code}`,
    }
  }

  return {
    badgeClass: "border-border bg-muted text-muted-foreground",
    dotClass: "bg-muted-foreground",
    text: `${code}`,
  }
}
