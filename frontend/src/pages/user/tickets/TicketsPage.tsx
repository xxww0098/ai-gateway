import { useState, useEffect, useCallback, useRef, memo } from "react"
import { useParams, useNavigate, useSearchParams } from "react-router-dom"
import { useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import { toast } from "sonner"
import { Button } from "@/shared/components/ui/button"
import { Textarea } from "@/shared/components/ui/textarea"
import {
  ChevronLeft,
  ChevronRight,
  Send,
  Loader2,
  User,
  Shield,
  MessageSquare,
  XCircle,
} from "lucide-react"
import { TicketImageUploadButton } from "@/features/tickets/TicketImageUploadButton"
import { TicketRichContent } from "@/features/tickets/TicketRichContent"
import {
  useTickets,
  useTicketDetail,
  useCreateTicket,
  useReplyTicket,
} from "@/features/tickets/hooks"
import type { TicketItem } from "@/features/tickets/types"
import { userRoutes, userTicketPath } from "@/shared/routes/user"
import { cn } from "@/shared/utils/utils"

const STATUS_LABEL: Record<string, string> = {
  open: "待处理",
  pending: "处理中",
  resolved: "已解决",
  closed: "已关闭",
}

const CATEGORY_MAP: Record<string, string> = {
  payment: "支付问题",
  technical: "技术问题",
  account: "账户问题",
  other: "其他",
}

const FILTER_KEYS = [
  { key: "", label: "全部" },
  { key: "open", label: "待处理" },
  { key: "pending", label: "处理中" },
  { key: "resolved", label: "已解决" },
  { key: "closed", label: "已关闭" },
] as const

function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function fmtShortTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function deriveTicketTitle(message: string): string {
  const first = message.trim().split(/\r?\n/).find((l) => l.trim().length > 0) ?? ""
  const oneLine = first.replace(/\s+/g, " ").trim()
  if (!oneLine) {
    const t = fmtDateTime(new Date().toISOString())
    return `在线咨询 · ${t}`.slice(0, 200)
  }
  return oneLine.length <= 200 ? oneLine : `${oneLine.slice(0, 197)}…`
}

type UserComposePaneProps = {
  onCreated: (id: number) => void
  onCancel?: () => void
}

const UserComposePane = memo(function UserComposePane({ onCreated, onCancel }: UserComposePaneProps) {
  const [text, setText] = useState("")
  const createTicketMutation = useCreateTicket()

  const submit = async () => {
    const content = text.trim()
    if (!content) {
      toast.error("请先输入要说明的内容")
      return
    }
    try {
      const res = await createTicketMutation.mutateAsync({
        title: deriveTicketTitle(content),
        category: "other",
        priority: "medium",
        content,
      })
      setText("")
      onCreated((res as { id: number }).id)
    } catch {
      // error handled by hook's onError
    }
  }

  const sending = createTicketMutation.isPending

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-white dark:bg-dark-900">
      <header className="shrink-0 border-b border-border px-4 py-2.5">
        <div className="flex items-start justify-between gap-2">
          <div>
            <h3 className="text-sm font-medium text-gray-900 dark:text-white">新对话</h3>
            <p className="mt-0.5 text-xs text-gray-500 dark:text-dark-400">
              像聊天一样输入即可，首条消息会作为工单说明。
            </p>
          </div>
          {onCancel && (
            <Button type="button" variant="ghost" size="sm" className="h-8 text-xs shrink-0" onClick={onCancel}>
              取消
            </Button>
          )}
        </div>
      </header>
      <div className="flex min-h-0 flex-1 flex-col justify-end p-4">
        <div className="mb-4 rounded-lg border border-dashed border-border bg-gray-50/80 p-4 text-xs text-gray-500 dark:bg-dark-800/50 dark:text-dark-400">
          从这里开始描述问题即可，无需再填单独表单。
        </div>
        <div className="mb-2 flex flex-wrap items-center gap-2">
          <TicketImageUploadButton
            disabled={sending}
            onInsert={(md) => setText((t) => (t ? `${t}\n` : "") + md)}
          />
          <span className="text-[11px] text-gray-400 dark:text-dark-500">插入后可继续打字说明</span>
        </div>
        <div className="flex gap-2">
          <Textarea
            placeholder="请描述您遇到的问题…"
            value={text}
            onChange={(e) => setText(e.target.value)}
            rows={4}
            className="min-h-0 flex-1 resize-none text-sm"
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault()
                void submit()
              }
            }}
          />
          <Button type="button" onClick={() => void submit()} disabled={sending || !text.trim()} className="shrink-0 self-end">
            {sending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
          </Button>
        </div>
        <p className="mt-2 text-[11px] text-gray-400 dark:text-dark-500">Cmd/Ctrl + Enter 发送</p>
      </div>
    </div>
  )
})

type UserTicketChatPaneProps = {
  ticketId: string | undefined
  onTicketUpdated: () => void
  onNewConversation: () => void
  onBackToList: () => void
}

const UserTicketChatPane = memo(function UserTicketChatPane({
  ticketId,
  onTicketUpdated,
  onNewConversation,
  onBackToList,
}: UserTicketChatPaneProps) {
  const [replyContent, setReplyContent] = useState("")
  const [closing, setClosing] = useState(false)
  const messagesEndRef = useRef<HTMLDivElement>(null)

  const { data: ticket, isLoading: loading, refetch } = useTicketDetail(ticketId)
  const replyMutation = useReplyTicket()

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "auto", block: "end" })
  }, [ticket?.replies?.length, ticketId])

  const submitReply = useCallback(
    async (raw: string) => {
      const content = raw.trim()
      if (!ticketId || !content) return
      try {
        await replyMutation.mutateAsync({ ticketId, body: { content } })
        setReplyContent("")
        await refetch()
        onTicketUpdated()
      } catch {
        // error handled by hook's onError
      }
    },
    [ticketId, replyMutation, refetch, onTicketUpdated]
  )

  const handleClose = async () => {
    if (!ticketId) return
    setClosing(true)
    try {
      await replyMutation.mutateAsync({ ticketId, body: { content: "用户确认关闭工单" } })
      await refetch()
      onTicketUpdated()
    } catch {
      // error handled by hook's onError
    } finally {
      setClosing(false)
    }
  }

  const sending = replyMutation.isPending

  if (!ticketId) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 py-12 text-center">
        <MessageSquare className="h-10 w-10 text-gray-300 dark:text-dark-600" />
        <div className="max-w-sm space-y-1">
          <p className="text-sm font-medium text-gray-800 dark:text-gray-200">从列表选择一条工单</p>
          <p className="text-xs text-gray-500 dark:text-dark-400">或发起新对话，查看与回复消息。</p>
        </div>
        <Button type="button" variant="default" size="sm" className="rounded-xl" onClick={onNewConversation}>
          发起新对话
        </Button>
      </div>
    )
  }

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-gray-500 dark:text-dark-400">
        <Loader2 className="mr-2 h-5 w-5 animate-spin text-primary-500" />
        加载中
      </div>
    )
  }

  if (!ticket) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 text-sm text-gray-500 dark:text-dark-400">
        无法加载该工单
      </div>
    )
  }

  const statusLabel = STATUS_LABEL[ticket.status] ?? ticket.status
  const cat = CATEGORY_MAP[ticket.category] || ticket.category
  const isClosed = ticket.status === "closed"

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-white dark:bg-dark-900">
      <header className="shrink-0 border-b border-border px-4 py-2.5">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="min-w-0 flex items-start gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-8 w-8 shrink-0 p-0 lg:hidden"
              onClick={onBackToList}
              title="返回列表"
              aria-label="返回列表"
            >
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <div className="min-w-0">
              <h3 className="truncate text-sm font-medium text-gray-900 dark:text-white">{ticket.title}</h3>
              <p className="mt-0.5 text-xs text-gray-500 dark:text-dark-400">
                #{ticket.id} · {cat} · {statusLabel}
              </p>
            </div>
          </div>
          <div className="flex shrink-0 gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="hidden h-8 text-xs lg:inline-flex"
              onClick={onBackToList}
            >
              关闭详情
            </Button>
            <Button type="button" variant="outline" size="sm" className="h-8 text-xs" onClick={onNewConversation}>
              新对话
            </Button>
            <button
              type="button"
              onClick={() => void refetch()}
              className="text-xs text-gray-500 underline-offset-2 hover:underline dark:text-dark-400"
            >
              刷新
            </button>
          </div>
        </div>
      </header>

      <div className="flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
          {ticket.replies.length === 0 ? (
            <p className="py-8 text-center text-xs text-gray-400 dark:text-dark-500">暂无消息</p>
          ) : (
            ticket.replies.map((reply) => {
              const isStaff = reply.is_staff
              return (
                <div key={reply.id} className={isStaff ? "flex justify-start" : "flex justify-end"}>
                  <div className={`flex max-w-[min(100%,480px)] gap-2 ${isStaff ? "flex-row" : "flex-row-reverse"}`}>
                    <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-gray-100 text-gray-500 dark:bg-dark-800 dark:text-gray-400">
                      {isStaff ? <Shield className="h-3.5 w-3.5" /> : <User className="h-3.5 w-3.5" />}
                    </div>
                    <div
                      className={`min-w-0 rounded-lg px-3 py-2 text-sm ${
                        isStaff
                          ? "bg-primary-500/15 text-gray-900 dark:bg-primary-500/20 dark:text-gray-100"
                          : "bg-gray-100 text-gray-900 dark:bg-dark-800 dark:text-gray-100"
                      }`}
                    >
                      <div
                        className={`mb-1 text-[11px] text-gray-500 dark:text-dark-400 ${isStaff ? "" : "text-right"}`}
                      >
                        {isStaff ? "客服" : "我"} · {fmtDateTime(reply.created_at)}
                      </div>
                      <TicketRichContent
                        content={reply.content}
                        className={isStaff ? "" : "text-right"}
                      />
                    </div>
                  </div>
                </div>
              )
            })
          )}
          <div ref={messagesEndRef} />
        </div>

        {!isClosed && (
          <div
            className="shrink-0 border-t border-border p-3 bg-white dark:bg-dark-900 sticky bottom-0"
            style={{ paddingBottom: "max(0.75rem, env(safe-area-inset-bottom, 0px))" }}
          >
            <div className="flex gap-2">
              <Textarea
                placeholder="输入回复…"
                value={replyContent}
                onChange={(e) => setReplyContent(e.target.value)}
                rows={2}
                className="min-h-[44px] flex-1 resize-none text-base sm:text-sm"
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault()
                    void submitReply(replyContent)
                  }
                }}
              />
              <TicketImageUploadButton
                compact
                disabled={sending || isClosed}
                onInsert={(md) => setReplyContent((t) => (t ? `${t}\n` : "") + md)}
              />
              <div className="flex flex-col gap-2">
                <Button
                  type="button"
                  onClick={() => void submitReply(replyContent)}
                  disabled={sending || !replyContent.trim()}
                  size="sm"
                  className="min-h-11 min-w-11 shrink-0"
                  title="发送"
                >
                  {sending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
                </Button>
              </div>
            </div>
            <div className="mt-2 flex items-center justify-between gap-2">
              <p className="text-[11px] text-gray-400 dark:text-dark-500 hidden sm:block">Cmd/Ctrl + Enter 发送</p>
              <button
                type="button"
                onClick={() => void handleClose()}
                disabled={closing}
                className="text-[11px] text-gray-500 underline-offset-2 hover:underline dark:text-dark-400"
              >
                关闭工单
              </button>
            </div>
          </div>
        )}

        {isClosed && (
          <div className="shrink-0 border-t border-border py-3 text-center text-xs text-gray-500 dark:text-dark-400">
            <div className="inline-flex items-center gap-1.5">
              <XCircle className="h-3.5 w-3.5" />
              已关闭，无法继续回复
            </div>
          </div>
        )}
      </div>
    </div>
  )
})

export default function TicketsPage() {
  const { id: routeTicketId } = useParams<{ id?: string }>()
  const navigate = useNavigate()
  const [searchParams, setSearchParams] = useSearchParams()
  const [page, setPage] = useState(1)
  const [pageSize] = useState(20)
  const [filterStatus, setFilterStatus] = useState("")
  const qc = useQueryClient()

  const composeMode = searchParams.get("compose") === "1" && !routeTicketId

  const { data, isLoading: listBusy, refetch: loadList } = useTickets(page, pageSize, filterStatus || undefined)
  const tickets: TicketItem[] = data?.items || []
  const total = data?.total || 0

  useEffect(() => {
    if (!routeTicketId) return
    if (searchParams.get("compose") !== "1") return
    const p = new URLSearchParams(searchParams)
    p.delete("compose")
    setSearchParams(p, { replace: true })
  }, [routeTicketId, searchParams, setSearchParams])

  useEffect(() => {
    if (searchParams.get("create") !== "1") return
    navigate({ pathname: userRoutes.tickets, search: "?compose=1" }, { replace: true })
  }, [searchParams, navigate])

  const handleFilter = useCallback((status: string) => {
    setFilterStatus(status)
    setPage(1)
  }, [])

  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  const selectedFromList = tickets.find((t) => String(t.id) === routeTicketId)

  const goCompose = useCallback(() => {
    navigate({ pathname: userRoutes.tickets, search: "?compose=1" }, { replace: true })
  }, [navigate])

  const goList = useCallback(() => {
    navigate(userRoutes.tickets, { replace: true })
  }, [navigate])

  const onTicketCreated = useCallback(
    (id: number) => {
      navigate(userTicketPath(id), { replace: true })
      qc.invalidateQueries({ queryKey: queryKeys.tickets.all() })
    },
    [navigate, qc]
  )

  const selectTicket = useCallback(
    (id: number) => {
      // Toggle off when clicking the active ticket → clean list URL
      if (String(id) === routeTicketId) {
        goList()
        return
      }
      navigate(userTicketPath(id))
    },
    [routeTicketId, goList, navigate]
  )

  // Mobile master-detail: show only list OR only chat/compose (lg keeps split)
  const detailOpen = Boolean(routeTicketId) || composeMode

  return (
    <div className="flex flex-col gap-2 sm:gap-3 animate-in fade-in slide-in-from-bottom-4 duration-500">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-lg font-semibold text-gray-900 dark:text-white">工单</h2>
        {!detailOpen && (
          <Button type="button" size="sm" className="lg:hidden h-10" onClick={goCompose}>
            新对话
          </Button>
        )}
      </div>

      <div
        className={cn(
          "flex flex-col rounded-lg border border-border bg-white dark:bg-dark-900 lg:flex-row lg:overflow-hidden",
          "min-h-[calc(100dvh-11rem)] lg:min-h-[min(100dvh-10rem,720px)]"
        )}
      >
        <aside
          className={cn(
            "flex w-full flex-col border-border lg:w-[min(100%,260px)] lg:max-w-[260px] lg:shrink-0 lg:border-r lg:bg-gray-50/50 dark:lg:bg-dark-950/50",
            detailOpen && "hidden lg:flex"
          )}
        >
          {routeTicketId && !selectedFromList && (
            <div className="shrink-0 border-b border-border px-3 py-2 text-xs text-gray-500 dark:text-dark-400">
              工单 #{routeTicketId} 不在本页列表，仍可在此查看对话。
            </div>
          )}

          <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-2 py-2">
            {FILTER_KEYS.map((f) => (
              <button
                key={f.key || "all"}
                type="button"
                onClick={() => handleFilter(f.key)}
                className={`min-h-9 rounded-lg px-3 py-1.5 text-xs font-medium ${
                  filterStatus === f.key
                    ? "bg-gray-200 text-gray-900 dark:bg-dark-700 dark:text-white"
                    : "text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-dark-800"
                }`}
              >
                {f.label}
              </button>
            ))}
            <button
              type="button"
              onClick={() => void loadList()}
              disabled={listBusy}
              className="ml-auto min-h-9 px-3 py-1.5 text-xs text-gray-500 hover:text-gray-700 disabled:opacity-50 dark:text-dark-400 dark:hover:text-gray-200"
            >
              {listBusy ? "更新中" : "刷新"}
            </button>
          </div>

          <div className={`min-h-0 flex-1 overflow-y-auto ${listBusy && tickets.length > 0 ? "opacity-70" : ""}`}>
            {listBusy && tickets.length === 0 ? (
              <div className="px-3 py-8 text-center text-xs text-gray-400 dark:text-dark-500">加载中</div>
            ) : tickets.length === 0 ? (
              <div className="px-3 py-8 text-center text-xs text-gray-400 dark:text-dark-500">
                暂无工单
                <button type="button" className="mt-3 block w-full min-h-11 rounded-xl bg-primary-600 text-sm font-medium text-white" onClick={goCompose}>
                  发起新对话
                </button>
              </div>
            ) : (
              <ul>
                {tickets.map((ticket) => {
                  const active = String(ticket.id) === routeTicketId
                  const st = STATUS_LABEL[ticket.status] ?? ticket.status
                  const cat = CATEGORY_MAP[ticket.category] || ticket.category
                  return (
                    <li key={ticket.id} className="border-b border-border last:border-b-0">
                      <button
                        type="button"
                        onClick={() => selectTicket(ticket.id)}
                        className={`flex w-full min-h-[3.25rem] gap-2 px-3 py-3 text-left ${
                          active ? "bg-gray-100 dark:bg-dark-800" : "hover:bg-gray-50 dark:hover:bg-dark-800/80 active:bg-gray-100 dark:active:bg-dark-800"
                        }`}
                      >
                        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-gray-200 text-xs font-medium text-gray-600 dark:bg-dark-700 dark:text-gray-300">
                          {(ticket.title || "?").trim().charAt(0).toUpperCase() || "?"}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="flex items-baseline justify-between gap-2">
                            <span className="truncate text-sm text-gray-900 dark:text-gray-100">#{ticket.id}</span>
                            <span className="shrink-0 tabular-nums text-[10px] text-gray-400 dark:text-dark-500">
                              {fmtShortTime(ticket.updated_at)}
                            </span>
                          </span>
                          <span className="mt-0.5 line-clamp-2 block text-xs text-gray-500 dark:text-dark-400">{ticket.title}</span>
                          <span className="mt-0.5 block text-[10px] text-gray-400 dark:text-dark-500">
                            {st} · {cat}
                          </span>
                        </span>
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
          </div>

          {total > 0 && (
            <div className="flex shrink-0 items-center justify-between border-t border-border px-2 py-1.5 text-[11px] text-gray-500 dark:text-dark-400">
              <span className="tabular-nums">
                {total} 条 · {page}/{totalPages}
              </span>
              <span className="flex gap-1">
                <button
                  type="button"
                  disabled={page <= 1 || listBusy}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                  className="rounded p-1 text-gray-500 hover:bg-gray-100 disabled:opacity-30 dark:hover:bg-dark-800"
                  aria-label="上一页"
                >
                  <ChevronLeft className="h-4 w-4" />
                </button>
                <button
                  type="button"
                  disabled={page >= totalPages || listBusy}
                  onClick={() => setPage((p) => p + 1)}
                  className="rounded p-1 text-gray-500 hover:bg-gray-100 disabled:opacity-30 dark:hover:bg-dark-800"
                  aria-label="下一页"
                >
                  <ChevronRight className="h-4 w-4" />
                </button>
              </span>
            </div>
          )}
        </aside>

        <div
          className={cn(
            "flex min-h-0 min-w-0 flex-1 flex-col",
            !detailOpen && "hidden lg:flex"
          )}
        >
          {composeMode ? (
            <UserComposePane onCreated={onTicketCreated} onCancel={goList} />
          ) : (
            <UserTicketChatPane
              ticketId={routeTicketId}
              onTicketUpdated={() => void loadList()}
              onNewConversation={goCompose}
              onBackToList={goList}
            />
          )}
        </div>
      </div>
    </div>
  )
}
