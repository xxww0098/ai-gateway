import { useState, useEffect, useCallback, useMemo } from "react"
import { fetchApi } from "@/shared/api/client"
import { Card } from "@/shared/components/ui/card"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/shared/components/ui/table"
import { Badge } from "@/shared/components/ui/badge"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Tabs, TabsList, TabsTrigger } from "@/shared/components/ui/tabs"
import { toast } from "sonner"
import {
  RefreshCw,
  Search,
  ShieldAlert,
  ShieldCheck,
  Filter,
  X,
  ChevronLeft,
  ChevronRight,
  Clock,
  Globe,
  ChevronDown,
} from "lucide-react"
import { EmptyState } from "@/shared/components/EmptyState"
import {
  type LogSource,
  type AuditEntry,
  type AuditResponse,
  SOURCE_CONFIG,
  getActionMeta,
  formatRelativeTime,
  formatExactTime,
  getMethodBadgeClass,
  getStatusBadgeClass,
} from "./audit-utils"
import { AuditLogDetailDrawer } from "./AuditLogDetailDrawer"

export default function AdminAuditLogsPage() {
  const [logs, setLogs] = useState<AuditEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [source, setSource] = useState<LogSource>("all")
  const [keyword, setKeyword] = useState("")
  const [activeSearch, setActiveSearch] = useState("")
  const [statusFilter, setStatusFilter] = useState<"all" | "success" | "error">("all")
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(30)
  const [total, setTotal] = useState(0)
  const [lastUpdated, setLastUpdated] = useState<string>("")
  const [autoRefreshSecs, setAutoRefreshSecs] = useState<number>(0)
  const [selectedLog, setSelectedLog] = useState<AuditEntry | null>(null)

  const loadLogs = useCallback(
    async (silent = false) => {
      if (silent) {
        setRefreshing(true)
      } else {
        setLoading(true)
      }

      try {
        const params = new URLSearchParams({
          source,
          page: String(page),
          page_size: String(pageSize),
        })
        if (activeSearch.trim()) params.set("q", activeSearch.trim())

        const res = await fetchApi(`/admin/audit-logs?${params.toString()}`)
        const data = (res.data ?? res) as AuditResponse
        setLogs(data.items || [])
        setTotal(data.total || 0)
        setLastUpdated(new Date().toLocaleTimeString())
      } catch (err: unknown) {
        toast.error(err instanceof Error ? err.message : "加载操作日志失败")
      } finally {
        setLoading(false)
        setRefreshing(false)
      }
    },
    [source, page, pageSize, activeSearch],
  )

  useEffect(() => {
    loadLogs()
  }, [loadLogs])

  // Auto refresh interval
  useEffect(() => {
    if (autoRefreshSecs <= 0) return
    const timer = setInterval(() => {
      loadLogs(true)
    }, autoRefreshSecs * 1000)
    return () => clearInterval(timer)
  }, [autoRefreshSecs, loadLogs])

  const handleSearchSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    setPage(1)
    setActiveSearch(keyword)
  }

  const handleClearSearch = () => {
    setKeyword("")
    setActiveSearch("")
    setPage(1)
  }

  const handleResetAllFilters = () => {
    setKeyword("")
    setActiveSearch("")
    setStatusFilter("all")
    setSource("all")
    setPage(1)
  }

  // Filter logs by status code if client filter is set
  const displayedLogs = useMemo(() => {
    if (statusFilter === "all") return logs
    if (statusFilter === "success") {
      return logs.filter((l) => !l.status_code || (l.status_code >= 200 && l.status_code < 400))
    }
    if (statusFilter === "error") {
      return logs.filter((l) => l.status_code && l.status_code >= 400)
    }
    return logs
  }, [logs, statusFilter])

  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  const startItem = total === 0 ? 0 : (page - 1) * pageSize + 1
  const endItem = Math.min(total, page * pageSize)

  const hasActiveFilters = activeSearch.trim() !== "" || statusFilter !== "all" || source !== "all"

  return (
    <div className="space-y-6">
      {/* 1. Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div className="space-y-1 max-w-2xl">
          <div className="flex items-center gap-2.5">
            <div className="w-9 h-9 rounded-xl bg-primary/10 text-primary flex items-center justify-center shrink-0">
              <ShieldAlert className="w-5 h-5" />
            </div>
            <h1 className="text-xl sm:text-2xl font-bold tracking-tight text-foreground">
              审计日志
            </h1>
            <span className="hidden xs:inline-flex items-center gap-1 rounded-full border border-border bg-muted/60 px-2.5 py-0.5 text-xs font-medium text-muted-foreground">
              <ShieldCheck className="h-3 w-3 text-emerald-500" />
              <span>安全合规追溯</span>
            </span>
          </div>
          <p className="text-xs sm:text-sm text-muted-foreground leading-relaxed">
            统一记录管理后台操作、SDK 代理调用以及资金账户变动，保障多租户平台透明安全。
          </p>
        </div>

        {/* Header Actions */}
        <div className="flex items-center gap-2.5 shrink-0 flex-wrap">
          {lastUpdated && (
            <span className="text-xs text-muted-foreground hidden md:inline-flex items-center gap-1 font-mono">
              <Clock className="w-3.5 h-3.5" />
              <span>{lastUpdated} 更新</span>
            </span>
          )}

          {/* Auto Refresh Selector */}
          <div className="relative inline-flex items-center">
            <select
              aria-label="自动刷新频率"
              value={autoRefreshSecs}
              onChange={(e) => setAutoRefreshSecs(Number(e.target.value))}
              className="h-8 rounded-lg border border-border bg-background px-2.5 pr-7 text-xs font-medium text-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring appearance-none cursor-pointer"
            >
              <option value={0}>手动刷新</option>
              <option value={10}>每 10 秒刷新</option>
              <option value={30}>每 30 秒刷新</option>
              <option value={60}>每 60 秒刷新</option>
            </select>
            <ChevronDown className="w-3.5 h-3.5 pointer-events-none absolute right-2 text-muted-foreground" />
          </div>

          <Button
            variant="outline"
            size="sm"
            onClick={() => loadLogs(false)}
            disabled={loading || refreshing}
            className="h-8 rounded-lg px-3 text-xs"
          >
            <RefreshCw className={`mr-1.5 h-3.5 w-3.5 ${loading || refreshing ? "animate-spin" : ""}`} />
            <span>刷新</span>
          </Button>
        </div>
      </div>

      {/* 2. Overview Stat Cards (Interactive Source Filters) */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        {(["all", "panel", "sdk", "balance"] as const).map((s) => {
          const cfg = SOURCE_CONFIG[s]
          const Icon = cfg.icon
          const isActive = source === s

          return (
            <button
              key={s}
              type="button"
              onClick={() => {
                setSource(s)
                setPage(1)
              }}
              className={`rounded-xl border p-3.5 text-left transition-all ${
                isActive
                  ? cfg.activeCardClass
                  : "border-border bg-card hover:bg-muted/40 text-card-foreground"
              }`}
            >
              <div className="flex items-center justify-between mb-1.5">
                <span className="text-xs font-medium text-muted-foreground">{cfg.label}</span>
                <Icon className={`w-4 h-4 ${isActive ? "text-primary" : "text-muted-foreground"}`} />
              </div>
              <div className="text-base sm:text-lg font-bold tracking-tight text-foreground">
                {isActive && s === "all" ? `${total} 条` : cfg.label}
              </div>
              <div className="text-[11px] text-muted-foreground truncate mt-0.5">
                {cfg.description}
              </div>
            </button>
          )
        })}
      </div>

      {/* 3. Filter & Search Toolbar */}
      <Card className="p-3.5 border-border bg-card shadow-2xs">
        <div className="flex flex-col lg:flex-row gap-3 items-stretch lg:items-center justify-between">
          {/* Tabs */}
          <Tabs
            value={source}
            onValueChange={(v) => {
              setSource(v as LogSource)
              setPage(1)
            }}
          >
            <TabsList className="bg-muted/70 p-1">
              <TabsTrigger value="all" className="text-xs sm:text-sm">
                全部
              </TabsTrigger>
              <TabsTrigger value="panel" className="text-xs sm:text-sm">
                面板操作
              </TabsTrigger>
              <TabsTrigger value="sdk" className="text-xs sm:text-sm">
                SDK 调用
              </TabsTrigger>
              <TabsTrigger value="balance" className="text-xs sm:text-sm">
                余额变动
              </TabsTrigger>
            </TabsList>
          </Tabs>

          {/* Search and Secondary Filter */}
          <div className="flex flex-wrap items-center gap-2">
            {/* Status Filter */}
            <div className="relative inline-flex items-center">
              <select
                aria-label="状态筛选"
                value={statusFilter}
                onChange={(e) => setStatusFilter(e.target.value as "all" | "success" | "error")}
                className="h-9 rounded-lg border border-border bg-background px-2.5 pr-7 text-xs font-medium text-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring appearance-none cursor-pointer"
              >
                <option value="all">全部状态</option>
                <option value="success">仅成功 (2xx)</option>
                <option value="error">仅异常 (4xx/5xx)</option>
              </select>
              <ChevronDown className="w-3.5 h-3.5 pointer-events-none absolute right-2 text-muted-foreground" />
            </div>

            {/* Keyword Search Form */}
            <form onSubmit={handleSearchSubmit} className="flex items-center gap-1.5 flex-1 sm:flex-initial">
              <div className="relative flex-1 sm:w-64">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
                <Input
                  placeholder="搜索动作 / 邮箱 / 路径 / IP..."
                  value={keyword}
                  onChange={(e) => setKeyword(e.target.value)}
                  className="pl-8 pr-7 h-9 text-xs"
                />
                {keyword && (
                  <button
                    type="button"
                    onClick={handleClearSearch}
                    aria-label="清除搜索"
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground p-0.5 rounded"
                  >
                    <X className="w-3.5 h-3.5" />
                  </button>
                )}
              </div>
              <Button type="submit" size="sm" variant="secondary" className="h-9 px-3 text-xs">
                搜索
              </Button>
            </form>

            {hasActiveFilters && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={handleResetAllFilters}
                className="h-9 px-2 text-xs text-muted-foreground hover:text-foreground"
              >
                重置
              </Button>
            )}
          </div>
        </div>
      </Card>

      {/* 4. Logs Table Card */}
      <Card className="border-border bg-card overflow-hidden shadow-2xs">
        <div className="overflow-x-auto">
          <Table>
            <TableHeader className="bg-muted/40">
              <TableRow className="border-border hover:bg-transparent">
                <TableHead className="w-28 text-xs font-semibold">来源</TableHead>
                <TableHead className="w-44 text-xs font-semibold">动作类型</TableHead>
                <TableHead className="text-xs font-semibold">对象 / 路由</TableHead>
                <TableHead className="w-52 text-xs font-semibold">触发主体与 IP</TableHead>
                <TableHead className="w-24 text-xs font-semibold text-center">状态</TableHead>
                <TableHead className="w-44 text-right text-xs font-semibold">发生时间</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {loading ? (
                // Skeleton Rows
                Array.from({ length: 6 }).map((_, i) => (
                  <TableRow key={i} className="border-border">
                    <TableCell>
                      <div className="h-5 w-16 rounded-md bg-muted animate-pulse" />
                    </TableCell>
                    <TableCell>
                      <div className="space-y-1.5">
                        <div className="h-4 w-20 rounded bg-muted animate-pulse" />
                        <div className="h-3 w-28 rounded bg-muted/60 animate-pulse" />
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="h-4 w-48 rounded bg-muted animate-pulse" />
                    </TableCell>
                    <TableCell>
                      <div className="space-y-1.5">
                        <div className="h-4 w-32 rounded bg-muted animate-pulse" />
                        <div className="h-3 w-20 rounded bg-muted/60 animate-pulse" />
                      </div>
                    </TableCell>
                    <TableCell className="text-center">
                      <div className="h-5 w-12 rounded-full bg-muted animate-pulse mx-auto" />
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="h-4 w-28 rounded bg-muted animate-pulse ml-auto" />
                    </TableCell>
                  </TableRow>
                ))
              ) : displayedLogs.length === 0 ? (
                <TableRow className="border-border hover:bg-transparent">
                  <TableCell colSpan={6} className="p-0">
                    <div className="p-12 text-center">
                      <EmptyState
                        size="compact"
                        icon={hasActiveFilters ? Filter : ShieldAlert}
                        title={hasActiveFilters ? "未找到匹配的审计日志" : "还没有审计记录"}
                        description={
                          hasActiveFilters
                            ? "请尝试清除搜索关键词或调整状态筛选条件。"
                            : "管理员的价格调整、配额分配、凭证修改及资金变动均将在此实时记录。"
                        }
                      />
                      {hasActiveFilters && (
                        <div className="mt-4">
                          <Button variant="outline" size="sm" onClick={handleResetAllFilters}>
                            清除所有筛选
                          </Button>
                        </div>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              ) : (
                displayedLogs.map((l) => {
                  const srcMeta = SOURCE_CONFIG[l.source] || SOURCE_CONFIG.all
                  const actMeta = getActionMeta(l.action)
                  const statusInfo = getStatusBadgeClass(l.status_code)
                  const SrcIcon = srcMeta.icon

                  return (
                    <TableRow
                      key={l.id}
                      onClick={() => setSelectedLog(l)}
                      className="cursor-pointer border-border hover:bg-muted/50 transition-colors group"
                    >
                      {/* Source */}
                      <TableCell>
                        <Badge
                          variant="outline"
                          className={`inline-flex items-center gap-1 font-medium text-xs py-0.5 px-2 ${srcMeta.badgeClass}`}
                        >
                          <SrcIcon className="w-3 h-3 shrink-0" />
                          <span>{srcMeta.label}</span>
                        </Badge>
                      </TableCell>

                      {/* Action */}
                      <TableCell>
                        <div className="space-y-0.5">
                          <div className="text-xs font-semibold text-foreground flex items-center gap-1.5">
                            <span>{actMeta.label}</span>
                          </div>
                          <code className="font-mono text-[11px] text-muted-foreground block truncate">
                            {l.action}
                          </code>
                        </div>
                      </TableCell>

                      {/* Target / Path */}
                      <TableCell>
                        <div className="flex items-center gap-2 max-w-[22rem] truncate">
                          {l.method && (
                            <span
                              className={`shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] font-bold border ${getMethodBadgeClass(
                                l.method,
                              )}`}
                            >
                              {l.method}
                            </span>
                          )}
                          <span
                            className="font-mono text-xs text-foreground/90 truncate select-all"
                            title={l.target || l.path || "-"}
                          >
                            {l.target || l.path || "-"}
                          </span>
                        </div>
                      </TableCell>

                      {/* Actor & IP */}
                      <TableCell>
                        <div className="space-y-0.5">
                          <div className="text-xs font-medium text-foreground truncate max-w-[12rem]">
                            {l.actor_email ? (
                              l.actor_email
                            ) : l.actor_id ? (
                              <span className="font-mono text-muted-foreground">UID: {l.actor_id}</span>
                            ) : (
                              <span className="text-muted-foreground">-</span>
                            )}
                          </div>
                          {l.ip_address && (
                            <div className="flex items-center gap-1 text-[11px] text-muted-foreground font-mono">
                              <Globe className="w-3 h-3 shrink-0 opacity-70" />
                              <span>{l.ip_address}</span>
                            </div>
                          )}
                        </div>
                      </TableCell>

                      {/* Status */}
                      <TableCell className="text-center">
                        {l.status_code ? (
                          <span
                            className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-semibold font-mono ${statusInfo.badgeClass}`}
                          >
                            <span className={`h-1.5 w-1.5 rounded-full ${statusInfo.dotClass}`} />
                            <span>{statusInfo.text}</span>
                          </span>
                        ) : (
                          <span className="text-muted-foreground text-xs font-mono">-</span>
                        )}
                      </TableCell>

                      {/* Time */}
                      <TableCell className="text-right">
                        <div className="space-y-0.5">
                          <div className="text-xs font-medium text-foreground">
                            {formatRelativeTime(l.created_at)}
                          </div>
                          <div className="text-[11px] text-muted-foreground font-mono tabular-nums">
                            {formatExactTime(l.created_at)}
                          </div>
                        </div>
                      </TableCell>
                    </TableRow>
                  )
                })
              )}
            </TableBody>
          </Table>
        </div>
      </Card>

      {/* 5. Pagination Footer */}
      <div className="flex flex-col sm:flex-row justify-between items-center gap-4 text-xs sm:text-sm text-muted-foreground px-1">
        <div>
          共 <span className="font-semibold text-foreground font-mono">{total}</span> 条记录
          {total > 0 && (
            <span>
              {" "}· 显示第 <span className="font-mono">{startItem}</span> - <span className="font-mono">{endItem}</span> 条
            </span>
          )}
          <span> · 第 <span className="font-mono">{page}</span> / <span className="font-mono">{totalPages}</span> 页</span>
        </div>

        <div className="flex items-center gap-3">
          {/* Page Size Selector */}
          <div className="flex items-center gap-1.5">
            <span className="text-xs">每页</span>
            <div className="relative inline-flex items-center">
              <select
                aria-label="每页显示条数"
                value={pageSize}
                onChange={(e) => {
                  setPageSize(Number(e.target.value))
                  setPage(1)
                }}
                className="h-8 rounded-lg border border-border bg-background px-2 pr-6 text-xs font-medium text-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring appearance-none cursor-pointer"
              >
                <option value={20}>20 条</option>
                <option value={30}>30 条</option>
                <option value={50}>50 条</option>
                <option value={100}>100 条</option>
              </select>
              <ChevronDown className="w-3.5 h-3.5 pointer-events-none absolute right-1.5 text-muted-foreground" />
            </div>
          </div>

          {/* Prev / Next Buttons */}
          <div className="flex items-center gap-1.5">
            <Button
              variant="outline"
              size="sm"
              disabled={page <= 1 || loading}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              className="h-8 px-2.5 text-xs gap-1"
            >
              <ChevronLeft className="h-3.5 w-3.5" />
              <span>上一页</span>
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={page >= totalPages || loading}
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
              className="h-8 px-2.5 text-xs gap-1"
            >
              <span>下一页</span>
              <ChevronRight className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>

      {/* 6. Slide-Out Detail Drawer */}
      <AuditLogDetailDrawer entry={selectedLog} onClose={() => setSelectedLog(null)} />
    </div>
  )
}
