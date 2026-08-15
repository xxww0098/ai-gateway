import { memo } from "react"
import { Badge } from "@/shared/components/ui/badge"

interface Props {
  status: string
}

export const AdminSubscriptionStatusBadge = memo(function AdminSubscriptionStatusBadge({ status }: Props) {
  switch (status) {
    case "active":
      return <Badge className="bg-emerald-500 hover:bg-emerald-600 text-white border-transparent">活跃</Badge>
    case "expired":
      return <Badge variant="secondary">已过期</Badge>
    case "suspended":
      return <Badge variant="destructive">已撤销</Badge>
    default:
      return <Badge variant="outline">{status}</Badge>
  }
})
