import { Badge } from "@/shared/components/ui/badge"

interface Props {
  status: string
}

export function SubscriptionStatusBadge({ status }: Props) {
  switch (status) {
    case 'active':
      return <Badge className="bg-emerald-500 hover:bg-emerald-600 text-white border-transparent">有效</Badge>
    case 'expired':
      return <Badge variant="secondary">已过期</Badge>
    case 'suspended':
      return <Badge variant="destructive">已撤销</Badge>
    case 'refunded':
      return <Badge className="bg-blue-500 hover:bg-blue-600 text-white border-transparent">已退款</Badge>
    default:
      return <Badge variant="outline">{status}</Badge>
  }
}
