import { useState, useEffect, useCallback } from "react"
import { fetchApi } from "@/shared/api/client"
import { Card } from "@/shared/components/ui/card"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/shared/components/ui/table"
import { Badge } from "@/shared/components/ui/badge"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Tabs, TabsList, TabsTrigger } from "@/shared/components/ui/tabs"
import { toast } from "sonner"
import { ShieldAlert, RefreshCcw, Search } from "lucide-react"

type LogSource = "all" | "panel" | "sdk" | "balance"

interface AuditEntry {
  id: string
  source: LogSource
  actor_id: number
  actor_email?: string
  action: string
  target?: string
  method?: string
  path?: string
  status_code?: number
  ip_address?: string
  request_id?: string
  metadata?: Record<string, unknown>
  created_at: string
}

interface AuditResponse {
  items: AuditEntry[]
  total: number
  page: number
  page_size: number
  source: LogSource
}

const SOURCE_LABEL: Record<LogSource, string> = {
  all: "全部",
  panel: "面板操作",
  sdk: "SDK 调用",
  balance: "余额变动",
}

const SOURCE_BADGE: Record<LogSource, string> = {
  all: "bg-muted text-muted-foreground",
  panel: "bg-blue-500/15 text-blue-600 dark:text-blue-300",
  sdk: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-300",
  balance: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
}

export default function AdminAuditLogsPage() {
  const [logs, setLogs] = useState<AuditEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [source, setSource] = useState<LogSource>("all")
  const [keyword, setKeyword] = useState("")
  const [page, setPage] = useState(1)
  const [pageSize] = useState(30)
  const [total, setTotal] = useState(0)

  const loadLogs = useCallback(async () => {
    setLoading(true)
    try {
      const params = new URLSearchParams({
        source,
        page: String(page),
        page_size: String(pageSize),
      })
      if (keyword.trim()) params.set("q", keyword.trim())
      const res = await fetchApi(`/admin/audit-logs?${params.toString()}`)
      const data = (res.data ?? res) as AuditResponse
      setLogs(data.items || [])
      setTotal(data.total || 0)
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "加载操作日志失败")
    } finally {
      setLoading(false)
    }
  }, [source, page, pageSize, keyword])

  useEffect(() => {
    loadLogs()
  }, [loadLogs])

  const onSearch = (e: React.FormEvent) => {
    e.preventDefault()
    setPage(1)
    loadLogs()
  }

  const totalPages = Math.max(1, Math.ceil(total / pageSize))

  return (
    <div className="space-y-6">
      <div className="flex flex-col sm:flex-row justify-between items-start sm:items-center gap-4">
        <div>
          <h2 className="text-3xl font-bold tracking-tight text-foreground flex items-center gap-2">
            <ShieldAlert className="h-8 w-8 text-primary" />
            系统操作日志
          </h2>
          <p className="text-muted-foreground mt-1">
            统一聚合面板操作、SDK /v1/* 调用与余额变动；管理员审计入口。
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={loadLogs} disabled={loading}>
          <RefreshCcw className={`h-4 w-4 mr-2 ${loading ? "animate-spin" : ""}`} />
          刷新
        </Button>
      </div>

      <Card className="p-4 space-y-4">
        <div className="flex flex-col md:flex-row gap-4 items-stretch md:items-center justify-between">
          <Tabs value={source} onValueChange={(v) => { setSource(v as LogSource); setPage(1) }}>
            <TabsList>
              <TabsTrigger value="all">全部</TabsTrigger>
              <TabsTrigger value="panel">面板操作</TabsTrigger>
              <TabsTrigger value="sdk">SDK 调用</TabsTrigger>
              <TabsTrigger value="balance">余额变动</TabsTrigger>
            </TabsList>
          </Tabs>
          <form onSubmit={onSearch} className="flex items-center gap-2">
            <div className="relative">
              <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
              <Input
                placeholder="action/email/model/reference"
                value={keyword}
                onChange={(e) => setKeyword(e.target.value)}
                className="pl-8 w-64"
              />
            </div>
            <Button type="submit" size="sm" variant="secondary">搜索</Button>
          </form>
        </div>
      </Card>

      <Card className="shadow-sm border-border overflow-hidden">
        <Table>
          <TableHeader className="bg-secondary/50">
            <TableRow>
              <TableHead className="w-24">来源</TableHead>
              <TableHead className="w-40">动作</TableHead>
              <TableHead>对象 / 路径</TableHead>
              <TableHead className="w-44">触发人</TableHead>
              <TableHead className="w-24">状态</TableHead>
              <TableHead className="w-40 text-right">时间</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={6} className="h-32 text-center text-muted-foreground">加载中...</TableCell>
              </TableRow>
            ) : logs.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="h-32 text-center text-muted-foreground">暂无记录</TableCell>
              </TableRow>
            ) : (
              logs.map((l) => (
                <TableRow key={l.id}>
                  <TableCell>
                    <Badge variant="outline" className={SOURCE_BADGE[l.source]}>
                      {SOURCE_LABEL[l.source] ?? l.source}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <code className="font-mono text-xs">{l.action}</code>
                  </TableCell>
                  <TableCell className="text-sm font-mono text-muted-foreground max-w-[24rem] truncate">
                    {l.target || l.path || "-"}
                    {l.method ? <span className="ml-2 text-xs opacity-70">{l.method}</span> : null}
                  </TableCell>
                  <TableCell className="text-sm">
                    {l.actor_email ? (
                      <span className="font-medium">{l.actor_email}</span>
                    ) : l.actor_id ? (
                      <span className="text-muted-foreground">UID: {l.actor_id}</span>
                    ) : (
                      <span className="text-muted-foreground">-</span>
                    )}
                    {l.ip_address ? (
                      <div className="text-xs text-muted-foreground">{l.ip_address}</div>
                    ) : null}
                  </TableCell>
                  <TableCell>
                    {l.status_code ? (
                      <Badge variant={l.status_code >= 400 ? "destructive" : "outline"} className="font-mono text-xs">
                        {l.status_code}
                      </Badge>
                    ) : (
                      <span className="text-muted-foreground text-xs">-</span>
                    )}
                  </TableCell>
                  <TableCell className="text-right text-muted-foreground text-sm">
                    {new Date(l.created_at).toLocaleString()}
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </Card>

      <div className="flex justify-between items-center text-sm text-muted-foreground">
        <div>共 {total} 条 · 第 {page} / {totalPages} 页</div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" disabled={page <= 1 || loading} onClick={() => setPage(page - 1)}>
            上一页
          </Button>
          <Button variant="outline" size="sm" disabled={page >= totalPages || loading} onClick={() => setPage(page + 1)}>
            下一页
          </Button>
        </div>
      </div>
    </div>
  )
}
