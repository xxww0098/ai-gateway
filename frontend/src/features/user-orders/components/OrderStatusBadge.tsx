import { getStatusInfo } from "../constants"
import type { PaymentOrder } from "../types"

interface Props {
  order?: PaymentOrder
  status?: string
  size?: "sm" | "default"
  className?: string
}

export function OrderStatusBadge({ order, status, size = "default", className = "" }: Props) {
  const statusKey = status || order?.status || ""
  const sInfo = getStatusInfo(statusKey)
  const StatusIcon = sInfo.icon
  const isPending = statusKey === "pending"

  const sizeClasses =
    size === "sm"
      ? "px-2 py-0.5 text-[11px] gap-1"
      : "px-2.5 py-1 text-xs gap-1.5"

  return (
    <span
      className={`inline-flex items-center font-semibold rounded-lg border tabular-nums transition-colors shadow-2xs ${sizeClasses} ${sInfo.bg} ${sInfo.border} ${sInfo.color} ${className}`}
    >
      <span className="relative flex h-1.5 w-1.5">
        {isPending && (
          <span
            className={`animate-ping absolute inline-flex h-full w-full rounded-full opacity-75 ${sInfo.dot}`}
          />
        )}
        <span className={`relative inline-flex rounded-full h-1.5 w-1.5 ${sInfo.dot}`} />
      </span>
      <StatusIcon className="w-3 h-3 shrink-0" />
      <span>{sInfo.label}</span>
    </span>
  )
}

