import { useState, useEffect, useCallback } from "react"
import { errorMessage, fetchApi } from "@/shared/api/client"
import { Card, CardContent } from "@/shared/components/ui/card"
import { Badge } from "@/shared/components/ui/badge"
import { Button } from "@/shared/components/ui/button"
import { toast } from "sonner"
import { ArrowLeftRight, Clock, CalendarDays, AlertCircle, CheckCircle2, XCircle } from "lucide-react"
import { userRoutes } from "@/shared/routes/user"
import { EmptyState } from "@/shared/components/EmptyState"

interface RefundRecord {
  id: number
  subscription_id: number
  amount: number
  reason: string
  status: "pending" | "approved" | "rejected"
  days_used: number
  total_days: number
  daily_rate: number
  processed_at: string | null
  created_at: string
}

export default function Refunds() {
  const [records, setRecords] = useState<RefundRecord[]>([])
  const [loading, setLoading] = useState(true)
  const [page, setPage] = useState(1)
  const [total, setTotal] = useState(0)
  const pageSize = 10

  const loadData = useCallback(async () => {
    setLoading(true)
    try {
      const res = await fetchApi(`/refund/list?page=${page}&page_size=${pageSize}`)
      setRecords(res?.data?.items || [])
      setTotal(res?.data?.total || 0)
    } catch (err: unknown) {
      toast.error(errorMessage(err, "加载失败"))
    } finally {
      setLoading(false)
    }
  }, [page])

  useEffect(() => {
    loadData()
  }, [loadData])

  const statusBadge = (status: string) => {
    switch (status) {
      case "pending":
        return (
          <Badge variant="outline" className="text-amber-600 border-amber-200 bg-amber-50 dark:bg-amber-950/20 dark:border-amber-800">
            <Clock className="w-3 h-3 mr-1" />
            审核中
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

  if (loading) {
    return (
      <div className="space-y-6">
        <div className="h-8 w-48 bg-gray-200 dark:bg-dark-800 rounded animate-pulse" />
        <div className="space-y-4">
          {[1, 2, 3].map((i) => (
            <div key={i} className="h-32 bg-gray-100 dark:bg-dark-800 rounded-xl animate-pulse" />
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-8 animate-in fade-in duration-500 max-w-4xl mx-auto" style={{ willChange: "transform, opacity" }}>
      <div className="space-y-2">
        <h3 className="text-2xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
          <ArrowLeftRight className="w-6 h-6 text-primary" />
          退款记录
        </h3>
        <p className="text-gray-500">
          查看您的退订申请记录和审核状态。审核通过后会取消对应订阅权益，并按规则调整账户余额。
        </p>
      </div>

      {records.length === 0 ? (
        <EmptyState
          bordered
          icon={ArrowLeftRight}
          title="暂无退款申请记录"
          description="提交订阅退款申请后，审核进度与退回金额将在此处展示。"
          action={{ label: '前往我的订阅', to: userRoutes.subscriptions }}
        />
      ) : (
        <div className="space-y-4">
          {records.map((r) => (
            <Card
              key={r.id}
              className={`transition-all hover:shadow-md ${
                r.status === "approved"
                  ? "border-emerald-100 dark:border-emerald-900/30"
                  : r.status === "rejected"
                  ? "border-red-100 dark:border-red-900/30"
                  : "border-gray-100 dark:border-dark-800"
              }`}
            >
              <CardContent className="p-5">
                <div className="flex flex-col sm:flex-row sm:items-start justify-between gap-4">
                  <div className="space-y-3 flex-1">
                    <div className="flex items-center gap-3">
                      {statusBadge(r.status)}
                      <span className="text-xs text-gray-400">#{r.id}</span>
                    </div>

                    <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 text-sm">
                      <div>
                        <div className="text-xs text-gray-400 mb-0.5">退款金额</div>
                        <div className="font-bold text-gray-900 dark:text-white">${r.amount.toFixed(2)}</div>
                      </div>
                      <div>
                        <div className="text-xs text-gray-400 mb-0.5">服务总天数</div>
                        <div className="font-medium text-gray-700 dark:text-gray-300">{r.total_days} 天</div>
                      </div>
                      <div>
                        <div className="text-xs text-gray-400 mb-0.5">已使用天数</div>
                        <div className="font-medium text-gray-700 dark:text-gray-300">{r.days_used} 天</div>
                      </div>
                      <div>
                        <div className="text-xs text-gray-400 mb-0.5">日单价</div>
                        <div className="font-medium text-gray-700 dark:text-gray-300">${r.daily_rate.toFixed(4)}</div>
                      </div>
                    </div>

                    {r.reason && (
                      <div className="text-sm text-gray-600 dark:text-gray-400 bg-gray-50 dark:bg-dark-800/50 rounded-lg p-3">
                        <span className="text-xs text-gray-400 block mb-1">退订原因</span>
                        {r.reason}
                      </div>
                    )}

                    <div className="flex items-center gap-1 text-xs text-gray-400">
                      <CalendarDays className="w-3.5 h-3.5" />
                      申请时间: {new Date(r.created_at).toLocaleString()}
                    </div>
                  </div>

                  <div className="flex flex-col items-end gap-2">
                    <div className="text-2xl font-bold text-gray-900 dark:text-white">
                      ${r.amount.toFixed(2)}
                    </div>
                    {r.status === "pending" && (
                      <div className="flex items-center gap-1 text-xs text-amber-600 dark:text-amber-400">
                        <AlertCircle className="w-3.5 h-3.5" />
                        等待审核
                      </div>
                    )}
                    {r.status === "approved" && r.processed_at && (
                      <div className="text-xs text-emerald-600 dark:text-emerald-400">
                        处理时间: {new Date(r.processed_at).toLocaleString()}
                      </div>
                    )}
                    {r.status === "rejected" && r.processed_at && (
                      <div className="text-xs text-red-600 dark:text-red-400">
                        处理时间: {new Date(r.processed_at).toLocaleString()}
                      </div>
                    )}
                  </div>
                </div>
              </CardContent>
            </Card>
          ))}

          {totalPages > 1 && (
            <div className="flex justify-center gap-2 pt-4">
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
        </div>
      )}
    </div>
  )
}
