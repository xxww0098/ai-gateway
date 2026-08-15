import { memo } from "react"
import { Card, CardContent } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Badge } from "@/shared/components/ui/badge"
import { PencilLine, Trash2, Wallet } from "lucide-react"
import { cn } from "@/shared/utils/utils"
import { formatLimit } from "../constants"
import type { Group } from "../types"

interface Props {
  group: Group
  onEdit: (g: Group) => void
  onDelete: (g: Group) => void
}

export const SubscriptionPackageCard = memo(function SubscriptionPackageCard({ group, onEdit, onDelete }: Props) {
  const selfServiceOn =
    group.subscription_price_usd != null && Number(group.subscription_price_usd) > 0
  const priceLabel =
    selfServiceOn && group.subscription_price_usd != null
      ? Number(group.subscription_price_usd).toLocaleString("en-US", {
          minimumFractionDigits: 0,
          maximumFractionDigits: 2,
        })
      : null

  return (
    <Card
      className={cn(
        "relative overflow-hidden group/card transition-all hover:shadow-md",
        "border bg-gradient-to-br",
        selfServiceOn
          ? "border-amber-400/90 dark:border-amber-500/80 ring-2 ring-amber-500/45 dark:ring-amber-400/35 shadow-md shadow-amber-200/35 dark:shadow-amber-950/50 from-amber-50/95 via-orange-50/60 to-amber-100/40 dark:from-amber-950/35 dark:via-orange-950/20 dark:to-amber-950/25"
          : "border-amber-100 dark:border-amber-900/30 from-amber-50/50 to-orange-50/30 dark:from-amber-950/10 dark:to-orange-950/10"
      )}
    >
      {selfServiceOn && (
        <>
          <div
            className="pointer-events-none absolute inset-x-0 top-0 h-1 bg-gradient-to-r from-amber-500 via-orange-500 to-amber-400"
            aria-hidden
          />
          <div
            className="pointer-events-none absolute -right-8 -top-8 h-24 w-24 rounded-full bg-amber-400/15 blur-2xl dark:bg-amber-400/10"
            aria-hidden
          />
        </>
      )}
      <CardContent className={cn("p-4", selfServiceOn && "pt-5")}>
        <div className="flex items-start justify-between mb-3">
          <div className="flex min-w-0 flex-1 items-center gap-2.5">
            <div
              className={cn(
                "flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br font-bold text-sm text-white shadow-sm",
                selfServiceOn
                  ? "from-amber-500 to-orange-600 ring-2 ring-white/50 dark:ring-amber-950/30"
                  : "from-amber-400 to-orange-500"
              )}
            >
              {group.name.substring(0, 2).toUpperCase()}
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-1.5 gap-y-1">
                <h4 className="font-semibold text-gray-900 dark:text-white text-sm leading-tight">{group.name}</h4>
                {selfServiceOn && (
                  <Badge
                    variant="secondary"
                    className="h-5 gap-0.5 border border-amber-400/70 bg-amber-100 px-1.5 text-[10px] font-semibold text-amber-950 shadow-sm dark:border-amber-600 dark:bg-amber-900/50 dark:text-amber-50"
                  >
                    <Wallet className="h-3 w-3 shrink-0 opacity-90" aria-hidden />
                    余额自助
                    {priceLabel != null ? ` $${priceLabel}` : ""}
                  </Badge>
                )}
              </div>
              <span className="text-[10px] text-gray-400">默认 {group.default_validity_days} 天 · 倍率 {group.rate_multiplier}x</span>
            </div>
          </div>
          <div className="flex gap-1 opacity-60 group-hover/card:opacity-100 transition-opacity">
            <Button size="icon" variant="ghost" className="h-7 w-7 text-gray-400 hover:text-blue-600"
              title="编辑" onClick={() => onEdit(group)}>
              <PencilLine className="h-3.5 w-3.5" />
            </Button>
            <Button size="icon" variant="ghost" className="h-7 w-7 text-gray-400 hover:text-red-600"
              title="删除" onClick={() => onDelete(group)}>
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
        <div className="grid grid-cols-3 gap-2 text-center">
          <div className="bg-white/60 dark:bg-dark-800/40 rounded-lg p-2">
            <div className="text-[10px] text-gray-400 mb-0.5">5h限额</div>
            <div className="text-xs font-semibold text-gray-700 dark:text-gray-300">{formatLimit(group.daily_limit_usd)}</div>
          </div>
          <div className="bg-white/60 dark:bg-dark-800/40 rounded-lg p-2">
            <div className="text-[10px] text-gray-400 mb-0.5">周限额</div>
            <div className="text-xs font-semibold text-gray-700 dark:text-gray-300">{formatLimit(group.weekly_limit_usd)}</div>
          </div>
          <div className="bg-white/60 dark:bg-dark-800/40 rounded-lg p-2">
            <div className="text-[10px] text-gray-400 mb-0.5">月限额</div>
            <div className="text-xs font-semibold text-gray-700 dark:text-gray-300">{formatLimit(group.monthly_limit_usd)}</div>
          </div>
        </div>
      </CardContent>
    </Card>
  )
})
