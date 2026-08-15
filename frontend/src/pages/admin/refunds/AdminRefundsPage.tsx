import { useState, useEffect, useCallback } from "react"
import { errorMessage, fetchApi } from "@/shared/api/client"
import { Card } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Badge } from "@/shared/components/ui/badge"
import { toast } from "sonner"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/shared/components/ui/table"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/components/ui/select"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/components/ui/dialog"
import {
  ArrowLeftRight,
  CheckCircle2,
  XCircle,
  Clock,
  Calculator,
  CalendarDays,
  Search,
  AlertTriangle,
} from "lucide-react"

interface RefundRecord {
  id: number
  user_id: number
  subscription_id: number
  amount: number
  reason: string
  status: "pending" | "approved" | "rejected"
  days_used: number
  total_days: number
  daily_rate: number
  processed_at: string | null
  processed_by: number | null
  created_at: string
}

type AdminRefundsProps = {
  /** When true, omit page chrome (embedded in Commerce tabs). */
  embedded?: boolean
}

export default function AdminRefunds({ embedded = false }: AdminRefundsProps) {
  const [records, setRecords] = useState<RefundRecord[]>([])
  const [loading, setLoading] = useState(true)
  const [page, setPage] = useState(1)
  const [total, setTotal] = useState(0)
  const [statusFilter, setStatusFilter] = useState<string>("all")
  const [detailOpen, setDetailOpen] = useState(false)
  const [selectedRecord, setSelectedRecord] = useState<RefundRecord | null>(null)
  const [actionLoading, setActionLoading] = useState<number | null>(null)
  const pageSize = 15

  const loadData = useCallback(async () => {
    setLoading(true)
    try {
      const statusQuery = statusFilter !== "all" ? `&status=${statusFilter}` : ""
      const res = await fetchApi(`/admin/refunds?page=${page}&page_size=${pageSize}${statusQuery}`)
      setRecords(res?.data?.items || [])
      setTotal(res?.data?.total || 0)
    } catch (err: unknown) {
      toast.error(errorMessage(err, "加载失败"))
    } finally {
      setLoading(false)
    }
  }, [page, statusFilter])

  useEffect(() => {
    loadData()
  }, [loadData])

  const handleApprove = async (id: number) => {
    if (!confirm("确定要通过此退订申请吗？通过后将取消对应订阅权益，并按规则调整用户账户余额。")) return
    setActionLoading(id)
    try {
      await fetchApi(`/admin/refund/${id}/approve`, { method: "PUT" })
      toast.success("退款申请已通过")
      loadData()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "操作失败"))
    } finally {
      setActionLoading(null)
    }
  }

  const handleReject = async (id: number) => {
    if (!confirm("确定要拒绝此退款申请吗？")) return
    setActionLoading(id)
    try {
      await fetchApi(`/admin/refund/${id}/reject`, { method: "PUT" })
      toast.success("退款申请已拒绝")
      loadData()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "操作失败"))
    } finally {
      setActionLoading(null)
    }
  }

  const openDetail = (record: RefundRecord) => {
    setSelectedRecord(record)
    setDetailOpen(true)
  }

  const statusBadge = (status: string) => {
    switch (status) {
      case "pending":
        return (
          <Badge variant="outline" className="text-amber-600 border-amber-200 bg-amber-50 dark:bg-amber-950/20 dark:border-amber-800">
            <Clock className="w-3 h-3 mr-1" />
            待审核
          </Badge>
        )
      case "approved":
        return (
          <Badge className="bg-emerald-500 hover:bg-emerald-600 text-white border-transparent">
            <CheckCircle2 className="w-3 h-3 mr-1" />
            已通过
          </Badge>
        )
      case "rejected":
        return (
          <Badge variant="destructive">
            <XCircle className="w-3 h-3 mr-1" />
            已拒绝
          </Badge>
        )
      default:
        return <Badge variant="outline">{status}</Badge>
    }
  }

  const totalPages = Math.ceil(total / pageSize)

  const pendingCount = records.filter((r) => r.status === "pending").length

  return (
    <div
      className={`space-y-6 ${embedded ? "" : "animate-in fade-in duration-500"}`}
      style={embedded ? undefined : { willChange: "transform, opacity" }}
    >
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        {!embedded ? (
          <div className="space-y-1">
            <h3 className="text-2xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
              <ArrowLeftRight className="w-6 h-6 text-primary" />
              退款审核
            </h3>
            <p className="text-sm text-gray-500 max-w-2xl">
              审核用户的退订申请。金额小于 $100 的退订已自动通过，此处仅显示需要人工审核的申请。
            </p>
          </div>
        ) : (
          <p className="text-sm text-gray-500 max-w-2xl">
            审核退订申请；小额退款可能已自动通过。
          </p>
        )}

        <div className="flex items-center gap-2">
          {pendingCount > 0 && (
            <Badge variant="outline" className="text-amber-600 border-amber-200 bg-amber-50 dark:bg-amber-950/20">
              <AlertTriangle className="w-3 h-3 mr-1" />
              {pendingCount} 待审核
            </Badge>
          )}
          <Select value={statusFilter} onValueChange={setStatusFilter}>
            <SelectTrigger className="w-36">
              <Search className="w-3.5 h-3.5 mr-2 text-gray-400" />
              <SelectValue placeholder="筛选状态" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">全部状态</SelectItem>
              <SelectItem value="pending">待审核</SelectItem>
              <SelectItem value="approved">已通过</SelectItem>
              <SelectItem value="rejected">已拒绝</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      <Card className="shadow-sm border-border overflow-hidden">
        <Table>
          <TableHeader className="bg-gray-50/50 dark:bg-dark-900/50">
            <TableRow>
              <TableHead>ID</TableHead>
              <TableHead>用户</TableHead>
              <TableHead>订阅</TableHead>
              <TableHead>金额</TableHead>
              <TableHead>状态</TableHead>
              <TableHead>申请时间</TableHead>
              <TableHead className="text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={7} className="h-32 text-center text-muted-foreground">
                  加载中...
                </TableCell>
              </TableRow>
            ) : records.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} className="h-32 text-center text-muted-foreground">
                  <div className="flex flex-col items-center gap-2">
                    <ArrowLeftRight className="w-8 h-8 text-gray-300 dark:text-gray-700" />
                    <p>暂无退款记录</p>
                  </div>
                </TableCell>
              </TableRow>
            ) : (
              records.map((r) => (
                <TableRow key={r.id} className="group">
                  <TableCell className="font-mono text-xs text-gray-400">#{r.id}</TableCell>
                  <TableCell>
                    <span className="text-sm font-medium">User #{r.user_id}</span>
                  </TableCell>
                  <TableCell>
                    <span className="text-sm text-gray-600 dark:text-gray-400">
                      Sub #{r.subscription_id}
                    </span>
                  </TableCell>
                  <TableCell>
                    <span className="text-sm font-bold text-gray-900 dark:text-white">
                      ${r.amount.toFixed(2)}
                    </span>
                  </TableCell>
                  <TableCell>{statusBadge(r.status)}</TableCell>
                  <TableCell>
                    <span className="text-xs text-gray-500">
                      {new Date(r.created_at).toLocaleDateString()}
                    </span>
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-1">
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => openDetail(r)}
                        className="text-gray-400 hover:text-primary hover:bg-primary/5"
                      >
                        详情
                      </Button>
                      {r.status === "pending" && (
                        <>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => handleApprove(r.id)}
                            disabled={actionLoading === r.id}
                            className="text-emerald-600 hover:text-emerald-700 hover:bg-emerald-50 dark:hover:bg-emerald-950/30"
                          >
                            {actionLoading === r.id ? "处理中..." : "通过"}
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => handleReject(r.id)}
                            disabled={actionLoading === r.id}
                            className="text-red-600 hover:text-red-700 hover:bg-red-50 dark:hover:bg-red-950/30"
                          >
                            拒绝
                          </Button>
                        </>
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
            <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => setPage(page - 1)}>
              上一页
            </Button>
            <span className="text-sm text-gray-500 self-center">
              {page} / {totalPages}
            </span>
            <Button size="sm" variant="outline" disabled={page >= totalPages} onClick={() => setPage(page + 1)}>
              下一页
            </Button>
          </div>
        )}
      </Card>

      <Dialog open={detailOpen} onOpenChange={setDetailOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Calculator className="w-5 h-5 text-primary" />
              退款详情
            </DialogTitle>
            <DialogDescription>
              退款申请 #{selectedRecord?.id} 的详细计算信息
            </DialogDescription>
          </DialogHeader>

          {selectedRecord && (
            <div className="space-y-4 pt-2">
              <div className="flex items-center justify-between">
                <span className="text-sm text-gray-500">状态</span>
                {statusBadge(selectedRecord.status)}
              </div>

              <div className="bg-gray-50 dark:bg-dark-800/50 rounded-xl p-4 space-y-3">
                <h4 className="text-sm font-semibold text-gray-700 dark:text-gray-300">计算详情</h4>
                <div className="grid grid-cols-2 gap-3 text-sm">
                  <div className="flex justify-between">
                    <span className="text-gray-500">退款金额</span>
                    <span className="font-bold">${selectedRecord.amount.toFixed(2)}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-500">服务总天数</span>
                    <span className="font-medium">{selectedRecord.total_days} 天</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-500">已使用天数</span>
                    <span className="font-medium">{selectedRecord.days_used} 天</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-500">日单价</span>
                    <span className="font-medium">${selectedRecord.daily_rate.toFixed(4)}</span>
                  </div>
                </div>
                <div className="pt-2 border-t border-gray-200 dark:border-dark-700">
                  <div className="flex justify-between text-sm">
                    <span className="text-gray-500">计算验证</span>
                    <span className="font-medium text-gray-700 dark:text-gray-300">
                      {selectedRecord.total_days - selectedRecord.days_used} 天 × ${selectedRecord.daily_rate.toFixed(4)} = $
                      {((selectedRecord.total_days - selectedRecord.days_used) * selectedRecord.daily_rate).toFixed(2)}
                    </span>
                  </div>
                </div>
              </div>

              {selectedRecord.reason && (
                <div className="space-y-1">
                  <span className="text-sm text-gray-500">退订原因</span>
                  <div className="text-sm text-gray-700 dark:text-gray-300 bg-gray-50 dark:bg-dark-800/50 rounded-lg p-3">
                    {selectedRecord.reason}
                  </div>
                </div>
              )}

              <div className="grid grid-cols-2 gap-3 text-xs text-gray-400">
                <div className="flex items-center gap-1">
                  <CalendarDays className="w-3.5 h-3.5" />
                  申请: {new Date(selectedRecord.created_at).toLocaleString()}
                </div>
                {selectedRecord.processed_at && (
                  <div className="flex items-center gap-1">
                    <Clock className="w-3.5 h-3.5" />
                    处理: {new Date(selectedRecord.processed_at).toLocaleString()}
                  </div>
                )}
              </div>

              {selectedRecord.status === "pending" && (
                <div className="flex gap-2 pt-2">
                  <Button
                    className="flex-1 bg-emerald-600 hover:bg-emerald-700"
                    onClick={() => {
                      setDetailOpen(false)
                      handleApprove(selectedRecord.id)
                    }}
                    disabled={actionLoading === selectedRecord.id}
                  >
                    <CheckCircle2 className="w-4 h-4 mr-1" />
                    通过退款
                  </Button>
                  <Button
                    variant="outline"
                    className="flex-1 border-red-200 text-red-600 hover:bg-red-50 hover:text-red-700"
                    onClick={() => {
                      setDetailOpen(false)
                      handleReject(selectedRecord.id)
                    }}
                    disabled={actionLoading === selectedRecord.id}
                  >
                    <XCircle className="w-4 h-4 mr-1" />
                    拒绝退款
                  </Button>
                </div>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
