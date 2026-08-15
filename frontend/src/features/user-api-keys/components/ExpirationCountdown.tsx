import { Clock, AlertTriangle, Infinity as InfinityIcon } from "lucide-react"

interface Props {
  expiresAt: string | null | undefined
}

export function ExpirationCountdown({ expiresAt }: Props) {
  if (!expiresAt) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs text-gray-400 dark:text-gray-500">
        <InfinityIcon className="h-3.5 w-3.5" />
        永久
      </span>
    )
  }

  const now = new Date()
  const exp = new Date(expiresAt)
  const diffMs = exp.getTime() - now.getTime()
  const diffDays = Math.ceil(diffMs / (1000 * 60 * 60 * 24))

  if (diffDays <= 0) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs font-semibold text-red-600 dark:text-red-400">
        <AlertTriangle className="h-3.5 w-3.5" />
        已过期
      </span>
    )
  }

  if (diffDays < 3) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs font-semibold text-red-600 dark:text-red-400">
        <Clock className="h-3.5 w-3.5" />
        {diffDays} 天后到期
      </span>
    )
  }

  if (diffDays < 7) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs font-semibold text-amber-600 dark:text-amber-400">
        <Clock className="h-3.5 w-3.5" />
        {diffDays} 天后到期
      </span>
    )
  }

  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400">
      <Clock className="h-3.5 w-3.5" />
      {diffDays} 天后到期
    </span>
  )
}
