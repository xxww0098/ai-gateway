import type { ReactNode } from "react"
import { Copy, Check, Trash2, MoreHorizontal } from "lucide-react"
import { Button } from "@/shared/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/components/ui/dropdown-menu"
import { ApiKeyStatusBadge } from "./ApiKeyStatusBadge"
import { ApiKeyUsageDialog } from "./ApiKeyUsageDialog"
import { ModelListDialog } from "./ModelListDialog"
import { QuotaProgressBar } from "./QuotaProgressBar"
import { ExpirationCountdown } from "./ExpirationCountdown"
import { GroupRebindDropdown } from "./GroupRebindDropdown"
import type { ApiKey, AvailableGroup } from "../types"

export function maskApiKeyDisplay(key: string): string {
  if (key.startsWith("sk-agw-")) return "sk-agw-****"
  if (key.startsWith("agw-")) return "agw-****"
  if (key.startsWith("sk-")) return "sk-****"
  return "****"
}

interface Props {
  keys: ApiKey[]
  loading: boolean
  copiedId: number | null
  onCopy: (id: number, key: string) => void
  onDelete: (id: number) => void
  groups: AvailableGroup[]
  groupsLoading: boolean
  rebindingId: number | null
  onRebindGroup: (keyId: number, groupId: number | null) => void
  /** 空状态下的下一步动作（通常是「新建密钥」对话框），无则只显示文案。 */
  emptyAction?: ReactNode
}

export function ApiKeysTable({
  keys,
  loading,
  copiedId,
  onCopy,
  onDelete,
  groups,
  groupsLoading,
  rebindingId,
  onRebindGroup,
  emptyAction,
}: Props) {
  if (loading) {
    return (
      <div className="glass-card flex h-32 items-center justify-center gap-2 text-gray-500">
        <div className="w-4 h-4 rounded-full bg-primary-500/50 animate-pulse" />
        数据加载中...
      </div>
    )
  }

  if (keys.length === 0) {
    return (
      <div className="glass-card flex h-32 flex-col items-center justify-center gap-3 text-gray-500 text-sm">
        <span>您还没有创建任何 API Key，请点击下方按钮新建。</span>
        {emptyAction}
      </div>
    )
  }

  return (
    <>
      {/* Mobile card list */}
      <div className="md:hidden space-y-3">
        {keys.map((k) => (
          <div
            key={k.id}
            className="rounded-xl border border-border bg-white dark:bg-dark-900 p-3 shadow-sm space-y-3"
          >
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0 space-y-1">
                <div className="font-medium text-gray-900 dark:text-white truncate">{k.name}</div>
                <ApiKeyStatusBadge status={k.display_status || k.status} />
              </div>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button type="button" variant="ghost" size="icon" className="h-11 w-11 shrink-0" aria-label="更多操作">
                    <MoreHorizontal className="h-5 w-5" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-48">
                  <DropdownMenuItem className="cursor-pointer" onSelect={() => onCopy(k.id, k.key)}>
                    复制 API Key
                  </DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <div className="px-2 py-1.5">
                    <p className="text-[11px] text-muted-foreground mb-1">分组</p>
                    <GroupRebindDropdown
                      currentGroupId={k.group_id}
                      currentGroupName={k.group_name}
                      groups={groups}
                      loading={groupsLoading}
                      onRebind={(groupId) => onRebindGroup(k.id, groupId)}
                      rebinding={rebindingId === k.id}
                    />
                  </div>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem
                    className="cursor-pointer text-red-600 focus:text-red-600"
                    onSelect={() => onDelete(k.id)}
                  >
                    <Trash2 className="h-4 w-4 mr-2" />
                    删除凭证
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>

            <button
              type="button"
              onClick={() => onCopy(k.id, k.key)}
              className="flex w-full min-h-11 items-center justify-between gap-2 rounded-lg border border-border bg-gray-50 dark:bg-dark-800 px-3 py-2 font-mono text-xs text-left active:bg-gray-100 dark:active:bg-dark-700"
            >
              <span>{maskApiKeyDisplay(k.key)}</span>
              {copiedId === k.id ? (
                <Check className="h-4 w-4 text-green-500 shrink-0" />
              ) : (
                <Copy className="h-4 w-4 opacity-60 shrink-0" />
              )}
            </button>

            {(k.quota > 0 || k.rate_limit_30d > 0) && (
              <div className="space-y-2">
                {k.quota > 0 && (
                  <QuotaProgressBar used={k.quota_used} total={k.quota} label="总额度" />
                )}
                {k.rate_limit_30d > 0 && (
                  <QuotaProgressBar used={k.usage_30d} total={k.rate_limit_30d} label="月限额" />
                )}
              </div>
            )}

            <div className="flex flex-wrap gap-2">
              <ApiKeyUsageDialog key_={k} />
              <ModelListDialog key_={k} />
            </div>
          </div>
        ))}
      </div>

      {/* Desktop table */}
      <div className="glass-card overflow-hidden hidden md:block">
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th className="w-[180px]">调用凭证名称</th>
                <th>API Key (点击复制)</th>
                <th>状态</th>
                <th className="hidden md:table-cell">分组 / 额度</th>
                <th className="hidden lg:table-cell">有效期</th>
                <th className="hidden lg:table-cell">最近使用</th>
                <th>操作</th>
                <th className="w-[50px]"></th>
              </tr>
            </thead>
            <tbody>
              {keys.map((k) => (
                <tr key={k.id}>
                  <td className="font-medium text-gray-900 dark:text-white">
                    <div className="flex flex-col gap-1">
                      <span>{k.name}</span>
                      {k.group_name && (
                        <span className="md:hidden inline-flex w-fit items-center rounded-md border border-primary-500/20 bg-primary-50 dark:bg-primary-900/20 px-2 py-0.5 text-xs font-semibold text-primary-600 dark:text-primary-400">
                          {k.group_name}
                        </span>
                      )}
                    </div>
                  </td>
                  <td>
                    <div
                      className="font-mono text-xs bg-gray-100 dark:bg-dark-900 hover:bg-gray-200 dark:hover:bg-dark-700 text-gray-700 dark:text-gray-300 py-1.5 px-2.5 rounded-lg border border-border cursor-pointer inline-flex items-center gap-2 group transition-colors"
                      onClick={() => onCopy(k.id, k.key)}
                    >
                      {maskApiKeyDisplay(k.key)}
                      {copiedId === k.id ? (
                        <Check className="h-3.5 w-3.5 text-green-500" />
                      ) : (
                        <Copy className="h-3.5 w-3.5 opacity-50 group-hover:opacity-100 transition-opacity" />
                      )}
                    </div>
                  </td>
                  <td>
                    <ApiKeyStatusBadge status={k.display_status || k.status} />
                  </td>
                  <td className="hidden md:table-cell">
                    <div className="flex flex-col gap-2 min-w-[180px]">
                      <GroupRebindDropdown
                        currentGroupId={k.group_id}
                        currentGroupName={k.group_name}
                        groups={groups}
                        loading={groupsLoading}
                        onRebind={(groupId) => onRebindGroup(k.id, groupId)}
                        rebinding={rebindingId === k.id}
                      />
                      {k.quota > 0 && (
                        <QuotaProgressBar used={k.quota_used} total={k.quota} label="总额度" />
                      )}
                      {k.rate_limit_30d > 0 && (
                        <QuotaProgressBar used={k.usage_30d} total={k.rate_limit_30d} label="月限额" />
                      )}
                    </div>
                  </td>
                  <td className="hidden lg:table-cell">
                    <ExpirationCountdown expiresAt={k.expires_at} />
                  </td>
                  <td className="hidden lg:table-cell text-sm text-gray-500 dark:text-gray-400 font-mono">
                    {k.last_used_at ? new Date(k.last_used_at).toLocaleDateString() : "—"}
                  </td>
                  <td>
                    <div className="flex items-center gap-1.5">
                      <ApiKeyUsageDialog key_={k} />
                      <ModelListDialog key_={k} />
                    </div>
                  </td>
                  <td className="text-right">
                    <Button
                      type="button"
                      variant="dangerIcon"
                      onClick={() => onDelete(k.id)}
                      title="删除凭证"
                      aria-label="删除凭证"
                    >
                      <Trash2 />
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </>
  )
}
