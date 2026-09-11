import { useState, useMemo, type ReactNode } from "react"
import {
  Copy,
  Check,
  Trash2,
  MoreHorizontal,
  Search,
  X,
  Filter,
} from "lucide-react"
import { Button } from "@/shared/components/ui/button"
import { EmptyState } from "@/shared/components/EmptyState"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/components/ui/dropdown-menu"
import { maskApiKeyDisplay } from "../mask"
import { ApiKeyStatusBadge } from "./ApiKeyStatusBadge"
import { ApiKeyUsageDialog } from "./ApiKeyUsageDialog"
import { ModelListDialog } from "./ModelListDialog"
import { QuotaProgressBar } from "./QuotaProgressBar"
import { ExpirationCountdown } from "./ExpirationCountdown"
import { GroupRebindDropdown } from "./GroupRebindDropdown"
import { ApiKeysEmptyState } from "./ApiKeysEmptyState"
import type { ApiKey, AvailableGroup } from "../types"

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
  const [searchTerm, setSearchTerm] = useState("")
  const [selectedGroupId, setSelectedGroupId] = useState<string>("all")
  const [statusFilter, setStatusFilter] = useState<string>("all")

  // Filtered keys
  const filteredKeys = useMemo(() => {
    return keys.filter((k) => {
      // Search term
      if (searchTerm.trim()) {
        const query = searchTerm.toLowerCase().trim()
        const nameMatch = k.name.toLowerCase().includes(query)
        const keyMatch = k.key.toLowerCase().includes(query)
        if (!nameMatch && !keyMatch) return false
      }

      // Group filter
      if (selectedGroupId !== "all") {
        if (selectedGroupId === "unbound") {
          if (k.group_id != null) return false
        } else {
          if (String(k.group_id) !== selectedGroupId) return false
        }
      }

      // Status filter
      if (statusFilter !== "all") {
        const effStatus = k.display_status || k.status
        if (effStatus !== statusFilter) return false
      }

      return true
    })
  }, [keys, searchTerm, selectedGroupId, statusFilter])

  const hasActiveFilters =
    searchTerm.trim().length > 0 || selectedGroupId !== "all" || statusFilter !== "all"

  const clearFilters = () => {
    setSearchTerm("")
    setSelectedGroupId("all")
    setStatusFilter("all")
  }

  // Skeleton loading state
  if (loading) {
    return (
      <div className="space-y-4">
        <div className="flex items-center justify-between gap-4">
          <div className="h-10 w-64 rounded-xl bg-muted animate-pulse" />
          <div className="h-10 w-32 rounded-xl bg-muted animate-pulse" />
        </div>
        <div className="rounded-xl border border-border bg-card overflow-hidden shadow-2xs">
          <div className="divide-y divide-border">
            <div className="h-11 bg-muted/40" />
            {[1, 2, 3, 4].map((i) => (
              <div key={i} className="flex items-center justify-between p-4 gap-4">
                <div className="space-y-2 flex-1">
                  <div className="h-4 w-1/3 rounded bg-muted animate-pulse" />
                  <div className="h-3 w-1/4 rounded bg-muted/60 animate-pulse" />
                </div>
                <div className="h-8 w-28 rounded-lg bg-muted animate-pulse" />
                <div className="h-6 w-16 rounded-full bg-muted animate-pulse" />
                <div className="h-8 w-20 rounded bg-muted animate-pulse hidden md:block" />
              </div>
            ))}
          </div>
        </div>
      </div>
    )
  }

  // Full onboarding empty state when no keys exist at all
  if (keys.length === 0) {
    return <ApiKeysEmptyState actionNode={emptyAction} />
  }

  return (
    <div className="space-y-4">
      {/* Search & Filter Bar */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-2.5">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground pointer-events-none" />
          <input
            type="text"
            placeholder="搜索 Key 名称或前缀..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="w-full pl-9 pr-8 py-2 rounded-xl border border-border bg-card text-xs sm:text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-primary focus:border-primary transition-all"
          />
          {searchTerm && (
            <button
              type="button"
              onClick={() => setSearchTerm("")}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 p-0.5 rounded text-muted-foreground hover:text-foreground"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>

        <div className="flex items-center gap-2 overflow-x-auto pb-1 sm:pb-0">
          {/* Group Filter */}
          <div className="relative shrink-0">
            <select
              value={selectedGroupId}
              onChange={(e) => setSelectedGroupId(e.target.value)}
              className="appearance-none pl-3 pr-8 py-2 rounded-xl border border-border bg-card text-xs font-medium text-foreground cursor-pointer focus:outline-none focus:ring-1 focus:ring-primary transition-all"
            >
              <option value="all">全部分组</option>
              <option value="unbound">未绑定分组</option>
              {groups.map((g) => (
                <option key={g.id} value={String(g.id)}>
                  {g.name}
                </option>
              ))}
            </select>
            <Filter className="absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground pointer-events-none" />
          </div>

          {/* Status Filter */}
          <div className="relative shrink-0">
            <select
              value={statusFilter}
              onChange={(e) => setStatusFilter(e.target.value)}
              className="appearance-none pl-3 pr-8 py-2 rounded-xl border border-border bg-card text-xs font-medium text-foreground cursor-pointer focus:outline-none focus:ring-1 focus:ring-primary transition-all"
            >
              <option value="all">全部状态</option>
              <option value="active">正常</option>
              <option value="inactive">已禁用</option>
              <option value="expired">已过期</option>
              <option value="exhausted">额度已尽</option>
            </select>
            <Filter className="absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground pointer-events-none" />
          </div>

          {hasActiveFilters && (
            <button
              type="button"
              onClick={clearFilters}
              className="inline-flex items-center gap-1 px-2.5 py-1.5 rounded-xl border border-dashed border-border text-xs text-muted-foreground hover:text-foreground transition-colors shrink-0"
            >
              <X className="h-3 w-3" />
              <span>清除</span>
            </button>
          )}
        </div>
      </div>

      {/* When filtering returns zero results */}
      {filteredKeys.length === 0 ? (
        <div className="rounded-xl border border-border bg-card p-8">
          <EmptyState
            size="compact"
            tone="no-results"
            title="未找到匹配的 API Key"
            description="请尝试更换搜索关键词，或清除当前分组与状态筛选条件。"
            action={{
              label: "清除所有筛选",
              onClick: clearFilters,
            }}
          />
        </div>
      ) : (
        <>
          {/* Mobile Card List */}
          <div className="md:hidden space-y-3">
            {filteredKeys.map((k) => (
              <div
                key={k.id}
                className="rounded-xl border border-border bg-card p-4 shadow-2xs space-y-3"
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0 space-y-1">
                    <div className="font-semibold text-foreground truncate text-sm">
                      {k.name}
                    </div>
                    <div className="flex flex-wrap items-center gap-1.5">
                      <ApiKeyStatusBadge status={k.display_status || k.status} />
                      {k.group_name && (
                        <span className="inline-flex items-center rounded-md border border-primary/20 bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
                          {k.group_name}
                        </span>
                      )}
                    </div>
                  </div>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-9 w-9 shrink-0"
                        aria-label="更多操作"
                      >
                        <MoreHorizontal className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-48">
                      <DropdownMenuItem
                        className="cursor-pointer"
                        onSelect={() => onCopy(k.id, k.key)}
                      >
                        <Copy className="h-4 w-4 mr-2" />
                        复制 API Key
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <div className="px-2 py-1.5">
                        <p className="text-[11px] text-muted-foreground mb-1 font-medium">分组设置</p>
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
                        className="cursor-pointer text-red-600 focus:text-red-600 dark:text-red-400"
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
                  className="flex w-full items-center justify-between gap-2 rounded-lg border border-border bg-muted/40 px-3 py-2 font-mono text-xs text-left active:bg-muted transition-colors group"
                >
                  <span className="text-foreground select-all">{maskApiKeyDisplay(k.key)}</span>
                  {copiedId === k.id ? (
                    <Check className="h-4 w-4 text-emerald-600 dark:text-emerald-400 shrink-0" />
                  ) : (
                    <Copy className="h-4 w-4 text-muted-foreground group-hover:text-foreground transition-colors shrink-0" />
                  )}
                </button>

                {(k.quota > 0 || k.rate_limit_30d > 0) && (
                  <div className="space-y-2 pt-1 border-t border-border">
                    {k.quota > 0 && (
                      <QuotaProgressBar used={k.quota_used} total={k.quota} label="总额度" />
                    )}
                    {k.rate_limit_30d > 0 && (
                      <QuotaProgressBar
                        used={k.usage_30d}
                        total={k.rate_limit_30d}
                        label="月限额"
                      />
                    )}
                  </div>
                )}

                <div className="flex items-center justify-between pt-1 border-t border-border text-xs text-muted-foreground">
                  <span className="flex items-center gap-1">
                    <span>有效期:</span>
                    <ExpirationCountdown expiresAt={k.expires_at} />
                  </span>
                  <div className="flex items-center gap-1.5">
                    <ApiKeyUsageDialog key_={k} />
                    <ModelListDialog key_={k} />
                  </div>
                </div>
              </div>
            ))}
          </div>

          {/* Desktop Table */}
          <div className="hidden md:block rounded-xl border border-border bg-card overflow-hidden shadow-2xs">
            <div className="overflow-x-auto">
              <table className="w-full text-left border-collapse">
                <thead>
                  <tr className="border-b border-border bg-muted/40 text-xs font-semibold text-muted-foreground">
                    <th className="py-3 px-4 w-[200px]">凭证名称</th>
                    <th className="py-3 px-4">API Key</th>
                    <th className="py-3 px-4">状态</th>
                    <th className="py-3 px-4">分组 / 额度上限</th>
                    <th className="py-3 px-4 hidden lg:table-cell">有效期</th>
                    <th className="py-3 px-4 hidden xl:table-cell">最近使用</th>
                    <th className="py-3 px-4 text-right">操作</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border text-xs sm:text-sm">
                  {filteredKeys.map((k) => (
                    <tr
                      key={k.id}
                      className="hover:bg-muted/30 transition-colors group"
                    >
                      {/* Name & Group badge */}
                      <td className="py-3.5 px-4 font-medium text-foreground">
                        <div className="flex flex-col gap-1 min-w-[140px]">
                          <span className="font-semibold text-foreground leading-snug">
                            {k.name}
                          </span>
                          {k.group_name ? (
                            <span className="inline-flex w-fit items-center rounded-md border border-primary/20 bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
                              {k.group_name}
                            </span>
                          ) : (
                            <span className="text-[11px] text-muted-foreground">默认通用分组</span>
                          )}
                        </div>
                      </td>

                      {/* API Key */}
                      <td className="py-3.5 px-4">
                        <button
                          type="button"
                          onClick={() => onCopy(k.id, k.key)}
                          className="inline-flex items-center gap-2 rounded-lg border border-border bg-muted/40 hover:bg-muted px-2.5 py-1.5 font-mono text-xs text-foreground transition-colors group/key cursor-pointer"
                          title="点击复制完整 API Key"
                        >
                          <span className="select-all">{maskApiKeyDisplay(k.key)}</span>
                          {copiedId === k.id ? (
                            <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400 shrink-0" />
                          ) : (
                            <Copy className="h-3.5 w-3.5 text-muted-foreground group-hover/key:text-foreground transition-colors shrink-0" />
                          )}
                        </button>
                      </td>

                      {/* Status */}
                      <td className="py-3.5 px-4">
                        <ApiKeyStatusBadge status={k.display_status || k.status} />
                      </td>

                      {/* Group & Quotas */}
                      <td className="py-3.5 px-4">
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
                            <QuotaProgressBar
                              used={k.quota_used}
                              total={k.quota}
                              label="总额度"
                            />
                          )}
                          {k.rate_limit_30d > 0 && (
                            <QuotaProgressBar
                              used={k.usage_30d}
                              total={k.rate_limit_30d}
                              label="30天限额"
                            />
                          )}
                          {k.quota <= 0 && k.rate_limit_30d <= 0 && (
                            <span className="text-[11px] text-muted-foreground">无消费限额</span>
                          )}
                        </div>
                      </td>

                      {/* Expiration */}
                      <td className="py-3.5 px-4 hidden lg:table-cell">
                        <ExpirationCountdown expiresAt={k.expires_at} />
                      </td>

                      {/* Last Used */}
                      <td className="py-3.5 px-4 hidden xl:table-cell text-xs text-muted-foreground font-mono">
                        {k.last_used_at ? new Date(k.last_used_at).toLocaleDateString() : "—"}
                      </td>

                      {/* Actions */}
                      <td className="py-3.5 px-4 text-right">
                        <div className="inline-flex items-center justify-end gap-1.5">
                          <ApiKeyUsageDialog key_={k} />
                          <ModelListDialog key_={k} />
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8 text-muted-foreground hover:text-red-600 dark:hover:text-red-400"
                            onClick={() => onDelete(k.id)}
                            title="删除凭证"
                            aria-label="删除凭证"
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </>
      )}
    </div>
  )
}
