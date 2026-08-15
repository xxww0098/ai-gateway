import { memo } from "react"
import { Button } from "@/shared/components/ui/button"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/shared/components/ui/table"
import { Card } from "@/shared/components/ui/card"
import { CalendarPlus, RotateCcw, Trash2, Crown, Clock, Undo2 } from "lucide-react"
import { confirmModal } from "@/shared/confirm-modal"
import { AdminSubscriptionStatusBadge } from "./AdminSubscriptionStatusBadge"
import { AdminSubscriptionUsageBar } from "./AdminSubscriptionUsageBar"
import { daysRemaining, subscriptionFundingSourceLabel } from "../constants"
import type { Subscription } from "../types"

function subscriptionUserLabel(s: Subscription): string {
  const email = s.email?.trim()
  if (email) return email
  const name = s.username?.trim()
  if (name) return name
  return `用户 #${s.user_id}`
}

interface Props {
  subs: Subscription[]
  loading: boolean
  page: number
  totalPages: number
  onPageChange: (p: number) => void
  onExtend: (id: number) => void
  onResetQuota: (id: number) => void
  onRevoke: (id: number) => void
  onReactivate: (id: number) => void
}

export const AdminSubscriptionsTable = memo(function AdminSubscriptionsTable({
  subs,
  loading,
  page,
  totalPages,
  onPageChange,
  onExtend,
  onResetQuota,
  onRevoke,
  onReactivate,
}: Props) {
  return (
    <Card className="shadow-sm border-border overflow-hidden">
      <Table>
        <TableHeader className="bg-gray-50/50 dark:bg-dark-900/50">
          <TableRow>
            <TableHead>用户</TableHead>
            <TableHead>套餐</TableHead>
            <TableHead className="min-w-[120px]">资金来源</TableHead>
            <TableHead>状态</TableHead>
            <TableHead>剩余天数</TableHead>
            <TableHead className="min-w-[200px]">用量</TableHead>
            <TableHead className="text-right">操作</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {loading ? (
            <TableRow>
              <TableCell colSpan={7} className="h-32 text-center text-muted-foreground">加载中...</TableCell>
            </TableRow>
          ) : subs.length === 0 ? (
            <TableRow>
              <TableCell colSpan={7} className="h-32 text-center text-muted-foreground">
                <div className="flex flex-col items-center gap-2">
                  <Crown className="w-8 h-8 text-gray-300 dark:text-gray-700" />
                  <p>暂无订阅记录</p>
                </div>
              </TableCell>
            </TableRow>
          ) : (
            subs.map((s) => (
              <TableRow key={s.id}>
                <TableCell>
                  <div className="flex flex-col">
                    <span className="font-medium text-sm">{subscriptionUserLabel(s)}</span>
                    <span className="text-xs text-gray-400">ID: {s.user_id}</span>
                  </div>
                </TableCell>
                <TableCell>
                  <span className="font-medium text-amber-600 dark:text-amber-400">{s.group_name || `Group #${s.group_id}`}</span>
                </TableCell>
                <TableCell>
                  <div className="flex flex-col gap-0.5 text-left max-w-[200px]">
                    <span className="text-sm text-gray-800 dark:text-gray-200">
                      {subscriptionFundingSourceLabel(s.funding_source)}
                    </span>
                    {s.funding_reference ? (
                      <span className="text-xs text-gray-400 truncate" title={s.funding_reference}>
                        {s.funding_reference}
                      </span>
                    ) : null}
                    {typeof s.price_paid === "number" && s.price_paid > 0 ? (
                      <span className="text-xs text-emerald-600 dark:text-emerald-400">实付 ${s.price_paid.toFixed(2)}</span>
                    ) : null}
                  </div>
                </TableCell>
                <TableCell><AdminSubscriptionStatusBadge status={s.status} /></TableCell>
                <TableCell>
                  <div className="flex items-center gap-1.5">
                    <Clock className="w-3.5 h-3.5 text-gray-400" />
                    <span className={`text-sm font-medium ${daysRemaining(s.expires_at) <= 3 ? 'text-red-500' : 'text-gray-700 dark:text-gray-300'}`}>
                      {daysRemaining(s.expires_at)} 天
                    </span>
                  </div>
                </TableCell>
                <TableCell>
                  <div className="space-y-1.5 min-w-[180px]">
                    <AdminSubscriptionUsageBar usage={s.daily_usage_usd} limit={s.daily_limit_usd} label="5h" />
                    <AdminSubscriptionUsageBar usage={s.monthly_usage_usd} limit={s.monthly_limit_usd} label="月" />
                  </div>
                </TableCell>
                <TableCell className="text-right">
                  <div className="flex justify-end gap-1">
                    <Button size="icon" variant="ghost" title="续期"
                      onClick={() => onExtend(s.id)}
                      className="h-8 w-8 text-gray-400 hover:text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-950/50">
                      <CalendarPlus className="h-4 w-4" />
                    </Button>
                    <Button
                      size="icon"
                      variant="outline"
                      title="重置周期用量（需二次确认，慎点）"
                      className="h-8 w-8 shrink-0 border-amber-500/55 bg-amber-50/70 text-amber-800 shadow-sm hover:border-amber-600 hover:bg-amber-100 hover:text-amber-950 dark:border-amber-600/60 dark:bg-amber-950/50 dark:text-amber-100 dark:hover:bg-amber-900/60 dark:hover:text-amber-50"
                      onClick={async () => {
                        const userLabel = subscriptionUserLabel(s)
                        const pkgLabel = s.group_name || `套餐 #${s.group_id}`
                        const ok = await confirmModal({
                          title: "确认重置配额？",
                          message: `即将清空 ${userLabel} 在「${pkgLabel}」上的 5h/周/月 周期用量统计，界面用量条会归零。此操作不可撤销，请确认不是误触。`,
                          confirmText: "确认重置",
                          cancelText: "取消",
                        })
                        if (ok) onResetQuota(s.id)
                      }}
                    >
                      <RotateCcw className="h-4 w-4" aria-hidden />
                    </Button>
                    {s.status === "suspended" ? (
                      <Button
                        size="icon"
                        variant="outline"
                        title="恢复订阅（将状态改回活跃）"
                        className="h-8 w-8 shrink-0 border-emerald-500/55 bg-emerald-50/70 text-emerald-800 shadow-sm hover:border-emerald-600 hover:bg-emerald-100 hover:text-emerald-950 dark:border-emerald-600/60 dark:bg-emerald-950/50 dark:text-emerald-100 dark:hover:bg-emerald-900/60 dark:hover:text-emerald-50"
                        onClick={async () => {
                          const userLabel = subscriptionUserLabel(s)
                          const pkgLabel = s.group_name || `套餐 #${s.group_id}`
                          const ok = await confirmModal({
                            title: "确认恢复订阅？",
                            message: `将把 ${userLabel} 在「${pkgLabel}」上已撤销的订阅重新设为活跃，用户即可按套餐限额继续使用（需在有效期内；已过期请先用「续期」）。`,
                            confirmText: "确认恢复",
                            cancelText: "取消",
                          })
                          if (ok) onReactivate(s.id)
                        }}
                      >
                        <Undo2 className="h-4 w-4" aria-hidden />
                      </Button>
                    ) : (
                      <Button
                        size="icon"
                        variant="outline"
                        title="撤销订阅（需二次确认，慎点）"
                        className="h-8 w-8 shrink-0 border-red-500/55 bg-red-50/70 text-red-800 shadow-sm hover:border-red-600 hover:bg-red-100 hover:text-red-950 dark:border-red-600/60 dark:bg-red-950/50 dark:text-red-100 dark:hover:bg-red-900/60 dark:hover:text-red-50"
                        onClick={async () => {
                          const userLabel = subscriptionUserLabel(s)
                          const pkgLabel = s.group_name || `套餐 #${s.group_id}`
                          const ok = await confirmModal({
                            title: "确认撤销订阅？",
                            message: `将把 ${userLabel} 在「${pkgLabel}」上的订阅标记为已撤销，用户将不能再使用该套餐额度。撤销后可在本行点击「恢复」重新激活（有效期内），或先「续期」再恢复。`,
                            confirmText: "确认撤销",
                            cancelText: "取消",
                          })
                          if (ok) onRevoke(s.id)
                        }}
                      >
                        <Trash2 className="h-4 w-4" aria-hidden />
                      </Button>
                    )}
                  </div>
                </TableCell>
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
      {totalPages > 1 && (
        <div className="flex justify-center gap-2 p-4 border-t border-border">
          <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => onPageChange(page - 1)}>上一页</Button>
          <span className="text-sm text-gray-500 self-center">{page} / {totalPages}</span>
          <Button size="sm" variant="outline" disabled={page >= totalPages} onClick={() => onPageChange(page + 1)}>下一页</Button>
        </div>
      )}
    </Card>
  )
})
