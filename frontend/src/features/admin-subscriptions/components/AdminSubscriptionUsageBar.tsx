import { memo } from "react"
import { usagePercent } from "../constants"

interface Props {
  usage: number
  limit?: number | null
  label: string
}

export const AdminSubscriptionUsageBar = memo(function AdminSubscriptionUsageBar({ usage, limit, label }: Props) {
  const pct = usagePercent(usage, limit)
  if (pct === null) return <span className="text-xs text-gray-400">无限制</span>
  const color = pct >= 90 ? "bg-red-500" : pct >= 70 ? "bg-amber-500" : "bg-emerald-500"
  return (
    <div className="space-y-0.5">
      <div className="flex justify-between text-[10px] text-gray-500">
        <span>{label}</span>
        <span>${usage.toFixed(4)} / ${limit!.toFixed(2)}</span>
      </div>
      <div className="h-1.5 bg-gray-100 dark:bg-dark-800 rounded-full overflow-hidden">
        <div className={`h-full rounded-full transition-all ${color}`} style={{ width: `${pct}%` }} />
      </div>
    </div>
  )
})
