import { useState } from "react"
import { useQuery } from "@tanstack/react-query"
import { apiClient } from "@/shared/api/client"
import { queryKeys } from "@/shared/api/query-keys"
import { errorMessage } from "@/shared/api/errors"
import { EmptyState, EmptyStateRow } from "@/shared/components/EmptyState"
import { userRoutes } from "@/shared/routes/user"
import { Button } from "@/shared/components/ui/button"
import { toast } from "sonner"
import {
  Wallet,
  ArrowUpRight,
  RefreshCw,
  ChevronLeft,
  ChevronRight,
  Gift,
  Settings2,
  Coins,
  FileText,
  Clock,
} from "lucide-react"

interface BalanceEntry {
  id: number
  kind?: string
  type?: string
  amount: number
  balance_before?: number
  balance_after?: number
  operator_email?: string | null
  note?: string | null
  reference?: string | null
  created_at: string
}

const KIND_MAP: Record<
  string,
  { label: string; color: string; bg: string; icon: React.ElementType }
> = {
  deposit: {
    label: "充值入账",
    color: "text-emerald-600 dark:text-emerald-400",
    bg: "bg-emerald-500/10",
    icon: ArrowUpRight,
  },
  redeem: {
    label: "卡密兑换",
    color: "text-teal-600 dark:text-teal-400",
    bg: "bg-teal-500/10",
    icon: Gift,
  },
  initial: {
    label: "初始余额",
    color: "text-blue-600 dark:text-blue-400",
    bg: "bg-blue-500/10",
    icon: Coins,
  },
  usage: {
    label: "API 扣费",
    color: "text-orange-600 dark:text-orange-400",
    bg: "bg-orange-500/10",
    icon: Coins,
  },
  adjustment: {
    label: "管理员调账",
    color: "text-purple-600 dark:text-purple-400",
    bg: "bg-purple-500/10",
    icon: Settings2,
  },
}

function getKindInfo(kind: string) {
  return (
    KIND_MAP[kind] || {
      label: kind,
      color: "text-muted-foreground",
      bg: "bg-muted",
      icon: FileText,
    }
  )
}

function fmtAmount(n: number): string {
  const prefix = n >= 0 ? "+" : ""
  return `${prefix}$${n.toFixed(4)}`
}

function fmtBalance(n: number | undefined | null): string {
  if (n === undefined || n === null) return "-"
  return `$${n.toFixed(4)}`
}

function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

type HistoryPage = {
  items: BalanceEntry[]
  total: number
  page: number
}

export default function BalanceHistory() {
  const [page, setPage] = useState(1)
  const [pageSize] = useState(20)
  const [filterKind, setFilterKind] = useState("")

  const historyQuery = useQuery({
    queryKey: queryKeys.billing.history({
      page,
      pageSize,
      kind: filterKind || undefined,
    }),
    queryFn: async () => {
      const params = new URLSearchParams()
      params.set("page", String(page))
      params.set("page_size", String(pageSize))
      if (filterKind) params.set("kind", filterKind)
      try {
        return await apiClient.get<HistoryPage>(`/user/balance-history?${params}`)
      } catch (err) {
        toast.error(errorMessage(err, "加载余额记录失败"))
        throw err
      }
    },
    retry: 0,
  })

  const entries = historyQuery.data?.items ?? []
  const total = historyQuery.data?.total ?? 0
  const loading = historyQuery.isLoading || historyQuery.isFetching

  const handleFilter = (kind: string) => {
    setFilterKind(kind)
    setPage(1)
  }

  const totalPages = Math.ceil(total / pageSize)
  const handlePage = (p: number) => {
    setPage(p)
  }

  return (
    <div className="space-y-4">
      {/* Filter toolbar */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-1.5">
          {[
            { key: "", label: "全部变动" },
            { key: "deposit", label: "在线充值" },
            { key: "redeem", label: "卡密兑换" },
            { key: "usage", label: "API 扣费" },
            { key: "adjustment", label: "管理调账" },
          ].map((f) => (
            <button
              key={f.key}
              type="button"
              onClick={() => handleFilter(f.key)}
              className={`px-3.5 py-1.5 rounded-xl text-xs font-semibold transition-all ${
                filterKind === f.key
                  ? "bg-primary text-primary-foreground shadow-xs"
                  : "bg-card border border-border/80 text-muted-foreground hover:text-foreground hover:bg-muted/80"
              }`}
            >
              {f.label}
            </button>
          ))}
        </div>

        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            void historyQuery.refetch()
          }}
          disabled={loading}
          className="h-8 px-3 text-xs rounded-xl gap-1.5 border-border"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
          刷新流水
        </Button>
      </div>

      {/* Mobile Card View */}
      <div className="md:hidden space-y-3">
        {loading ? (
          <div className="rounded-2xl border border-border/80 bg-card flex h-36 items-center justify-center gap-2 text-muted-foreground text-sm">
            <RefreshCw className="w-4 h-4 animate-spin text-primary" />
            加载流水记录中...
          </div>
        ) : entries.length === 0 ? (
          <EmptyState
            bordered
            size="compact"
            tone="first-use"
            icon={Wallet}
            title="暂无余额变动记录"
            description="充值入账、卡密兑换、API 调用扣费与退款等资金明细均将在此实时呈现。"
            action={{ label: "前往充值", to: userRoutes.financeTopup }}
          />
        ) : (
          entries.map((entry) => {
            const kind = entry.kind ?? entry.type ?? "unknown"
            const info = getKindInfo(kind)
            const Icon = info.icon
            const isPositive = entry.amount >= 0
            return (
              <div
                key={entry.id}
                className="rounded-2xl border border-border/80 bg-card p-4 shadow-xs space-y-2.5"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2.5 min-w-0">
                    <div
                      className={`w-8 h-8 rounded-xl flex items-center justify-center shrink-0 ${info.bg}`}
                    >
                      <Icon className={`w-4 h-4 ${info.color}`} />
                    </div>
                    <span className="text-sm font-bold text-foreground truncate">
                      {info.label}
                    </span>
                  </div>
                  <span
                    className={`text-sm font-bold tabular-nums shrink-0 ${
                      isPositive
                        ? "text-emerald-600 dark:text-emerald-400"
                        : "text-foreground"
                    }`}
                  >
                    {fmtAmount(entry.amount)}
                  </span>
                </div>

                <div className="flex items-center justify-between text-xs text-muted-foreground tabular-nums pt-1 border-t border-border/50">
                  <span className="flex items-center gap-1">
                    <Clock className="w-3 h-3" />
                    {fmtDateTime(entry.created_at)}
                  </span>
                  <span>
                    {fmtBalance(entry.balance_before)} →{" "}
                    <strong className="text-foreground font-semibold">
                      {fmtBalance(entry.balance_after)}
                    </strong>
                  </span>
                </div>

                {(entry.note || entry.reference) && (
                  <p className="text-xs text-muted-foreground line-clamp-2 bg-muted/30 p-2 rounded-lg">
                    {entry.note || entry.reference}
                  </p>
                )}
              </div>
            )
          })
        )}

        {total > 0 && (
          <div className="flex items-center justify-between text-xs text-muted-foreground pt-2">
            <span className="tabular-nums font-medium">
              共 {total} 条记录 · {page}/{totalPages} 页
            </span>
            <div className="flex gap-1.5">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => handlePage(page - 1)}
                className="h-8 w-8 p-0 rounded-xl"
              >
                <ChevronLeft className="w-4 h-4" />
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages}
                onClick={() => handlePage(page + 1)}
                className="h-8 w-8 p-0 rounded-xl"
              >
                <ChevronRight className="w-4 h-4" />
              </Button>
            </div>
          </div>
        )}
      </div>

      {/* Desktop Data Table */}
      <div className="rounded-2xl border border-border/80 bg-card overflow-hidden shadow-xs hidden md:block">
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead className="border-b border-border bg-muted/40 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              <tr>
                <th className="py-3.5 px-4 w-[170px]">发生时间</th>
                <th className="py-3.5 px-4 w-[140px]">变动类型</th>
                <th className="py-3.5 px-4 w-[130px]">变动金额</th>
                <th className="py-3.5 px-4 w-[130px]">变动前</th>
                <th className="py-3.5 px-4 w-[130px]">变动后</th>
                <th className="py-3.5 px-4">备注说明 / 关联单号</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border/60">
              {loading ? (
                <tr>
                  <td colSpan={6} className="h-44 text-center">
                    <div className="flex items-center justify-center gap-2 text-muted-foreground text-sm">
                      <RefreshCw className="w-4 h-4 animate-spin text-primary" />
                      正在加载流水记录...
                    </div>
                  </td>
                </tr>
              ) : entries.length === 0 ? (
                <EmptyStateRow
                  colSpan={6}
                  tone="first-use"
                  icon={Wallet}
                  title="暂无余额变动记录"
                  description="充值入账、卡密兑换、API 调用扣费与退款等资金明细均将在此实时呈现。"
                  action={{ label: "前往充值", to: userRoutes.financeTopup }}
                />
              ) : (
                entries.map((entry) => {
                  const kind = entry.kind ?? entry.type ?? "unknown"
                  const info = getKindInfo(kind)
                  const Icon = info.icon
                  const isPositive = entry.amount >= 0
                  return (
                    <tr
                      key={entry.id}
                      className="hover:bg-muted/30 transition-colors"
                    >
                      <td className="py-3 px-4">
                        <span className="text-xs text-muted-foreground tabular-nums">
                          {fmtDateTime(entry.created_at)}
                        </span>
                      </td>
                      <td className="py-3 px-4">
                        <div className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-semibold bg-muted/50 border border-border/50">
                          <Icon className={`w-3.5 h-3.5 ${info.color}`} />
                          <span className="text-foreground">{info.label}</span>
                        </div>
                      </td>
                      <td className="py-3 px-4">
                        <span
                          className={`text-sm font-bold tabular-nums ${
                            isPositive
                              ? "text-emerald-600 dark:text-emerald-400"
                              : "text-foreground"
                          }`}
                        >
                          {fmtAmount(entry.amount)}
                        </span>
                      </td>
                      <td className="py-3 px-4">
                        <span className="text-xs text-muted-foreground tabular-nums">
                          {fmtBalance(entry.balance_before)}
                        </span>
                      </td>
                      <td className="py-3 px-4">
                        <span className="text-xs font-semibold text-foreground tabular-nums">
                          {fmtBalance(entry.balance_after)}
                        </span>
                      </td>
                      <td className="py-3 px-4">
                        <span
                          className="text-xs text-muted-foreground truncate block max-w-[340px]"
                          title={entry.note || entry.reference || ""}
                        >
                          {entry.note || entry.reference || "-"}
                          {entry.operator_email && (
                            <span className="text-[11px] text-muted-foreground/80 ml-1">
                              ({entry.operator_email})
                            </span>
                          )}
                        </span>
                      </td>
                    </tr>
                  )
                })
              )}
            </tbody>
          </table>
        </div>

        {total > 0 && (
          <div className="px-5 py-3.5 border-t border-border flex items-center justify-between bg-muted/20">
            <div className="text-xs text-muted-foreground tabular-nums">
              共 <strong className="text-foreground">{total}</strong> 条变动记录 · 第 {page} / {totalPages} 页
            </div>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => handlePage(page - 1)}
                className="h-8 px-2.5 rounded-xl border-border text-xs gap-1"
              >
                <ChevronLeft className="w-3.5 h-3.5" />
                上一页
              </Button>
              <span className="px-2 text-xs text-muted-foreground tabular-nums font-medium">
                {page} / {totalPages}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages}
                onClick={() => handlePage(page + 1)}
                className="h-8 px-2.5 rounded-xl border-border text-xs gap-1"
              >
                下一页
                <ChevronRight className="w-3.5 h-3.5" />
              </Button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
