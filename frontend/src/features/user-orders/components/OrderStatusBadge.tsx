import { getStatusInfo } from "../constants"
import type { PaymentOrder } from "../types"

interface Props {
  order: PaymentOrder
}

export function OrderStatusBadge({ order }: Props) {
  const sInfo = getStatusInfo(order.status)
  const StatusIcon = sInfo.icon
  return (
    <span className={`inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-[11px] font-medium ${sInfo.bg} ${sInfo.color}`}>
      <StatusIcon className="w-3 h-3" />
      {sInfo.label}
    </span>
  )
}
