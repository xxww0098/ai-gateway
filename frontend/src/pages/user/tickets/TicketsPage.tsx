import { useState, useEffect, useCallback, useRef, memo, useMemo } from "react"
import { useParams, useNavigate, useSearchParams } from "react-router-dom"
import { useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import { toast } from "sonner"
import { Button } from "@/shared/components/ui/button"
import { Textarea } from "@/shared/components/ui/textarea"
import { Input } from "@/shared/components/ui/input"
import {
  ChevronLeft,
  ChevronRight,
  Send,
  Loader2,
  User,
  ShieldCheck,
  Ticket,
  XCircle,
  Headphones,
  Plus,
  RefreshCw,
  Search,
  Wrench,
  CreditCard,
  UserCheck,
  Sparkles,
  Copy,
  Check,
  Clock,
  ArrowLeft,
  CheckCircle2,
  FileQuestion,
} from "lucide-react"
import { TicketImageUploadButton } from "@/features/tickets/TicketImageUploadButton"
import { uploadTicketImageFile } from "@/features/tickets/api"
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

const STATUS_CONFIG: Record<string, { label: string; dotClass: string; badgeClass: string }> = {
  open: {
    label: "待处理",
    dotClass: "bg-amber-500",
    badgeClass: "bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-500/20",
  },
  pending: {
    label: "处理中",
    dotClass: "bg-blue-500 animate-pulse",
    badgeClass: "bg-blue-500/10 text-blue-700 dark:text-blue-300 border-blue-500/20",
  },
  resolved: {
    label: "已解决",
    dotClass: "bg-emerald-500",
    badgeClass: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 border-emerald-500/20",
  },
  closed: {
    label: "已关闭",
    dotClass: "bg-muted-foreground/60",
    badgeClass: "bg-muted text-muted-foreground border-border/70",
  },
}

const CATEGORY_CONFIG: Record<
  string,
  { label: string; icon: typeof Wrench; desc: string; placeholder: string }
> = {
  technical: {
    label: "技术问题",
    icon: Wrench,
    desc: "接口调用报错、超时断连、协议兼容或 SDK 异常",
    placeholder: "请提供具体调用的模型、返回的错误码、请求时间或 curl/代码片段…",
  },
  payment: {
    label: "支付与账单",
    icon: CreditCard,
    desc: "充值到账咨询、Token 扣费明细核验或发票问题",
    placeholder: "请提供充值订单号、涉及的扣费记录时间或支付凭据说明…",
  },
  account: {
    label: "账户与配额",
    icon: UserCheck,
    desc: "申请调高单模型 RPM/TPM 速率限制或专属通道",
    placeholder: "请说明您需要调高额度的模型名称、预估并发量及业务调用场景…",
  },
  other: {
    label: "其他与建议",
    icon: Sparkles,
    desc: "新增模型建议、产品功能反馈或其他定制诉求",
    placeholder: "请留下您对 AI-GateWay 的功能建议、期望接入的模型或其它诉求…",
  },
}

const FILTER_KEYS = [
  { key: "", label: "全部" },
  { key: "open", label: "待处理" },
  { key: "pending", label: "处理中" },
  { key: "resolved", label: "已解决" },
  { key: "closed", label: "已关闭" },
] as const

function fmtDateTime(iso: string): string {
  if (!iso) return ""
  const d = new Date(iso)
  if (isNaN(d.getTime())) return iso
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function fmtRelativeOrShortTime(iso: string): string {
  if (!iso) return ""
  const d = new Date(iso)
  if (isNaN(d.getTime())) return iso
  const now = Date.now()
  const diff = now - d.getTime()
  if (diff < 60 * 1000) return "刚刚"
  if (diff < 60 * 60 * 1000) return `${Math.floor(diff / (60 * 1000))} 分钟前`
  if (diff < 24 * 60 * 60 * 1000) return `${Math.floor(diff / (60 * 60 * 1000))} 小时前`
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function deriveTicketTitle(message: string): string {
  const first = message.trim().split(/\r?\n/).find((l) => l.trim().length > 0) ?? ""
  const oneLine = first.replace(/\s+/g, " ").trim()
  if (!oneLine) {
    const t = fmtDateTime(new Date().toISOString())
    return `技术咨询 · ${t}`.slice(0, 200)
  }
  return oneLine.length <= 200 ? oneLine : `${oneLine.slice(0, 197)}…`
}


type UserComposePaneProps = {
  initialCategory?: string
  initialTopic?: string
  onCreated: (id: number) => void
  onCancel?: () => void
}

const UserComposePane = memo(function UserComposePane({
  initialCategory = "technical",
  initialTopic = "",
  onCreated,
  onCancel,
}: UserComposePaneProps) {
  const [category, setCategory] = useState<string>(initialCategory)
  const [title, setTitle] = useState(initialTopic)
  const [text, setText] = useState("")
  const [uploadingPaste, setUploadingPaste] = useState(false)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const createTicketMutation = useCreateTicket()

  useEffect(() => {
    if (initialCategory) setCategory(initialCategory)
  }, [initialCategory])

  useEffect(() => {
    if (initialTopic) setTitle(initialTopic)
  }, [initialTopic])

  const handlePaste = useCallback(async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = e.clipboardData?.items
    if (!items) return
    for (let i = 0; i < items.length; i++) {
      const item = items[i]
      if (item.type.startsWith("image/")) {
        const file = item.getAsFile()
        if (file) {
          e.preventDefault()
          setUploadingPaste(true)
          try {
            const md = await uploadTicketImageFile(file)
            setText((prev) => (prev ? `${prev}\n${md}` : md))
            toast.success("截图已自动上传并插入")
          } catch (err) {
            toast.error(err instanceof Error ? err.message : "图片上传失败")
          } finally {
            setUploadingPaste(false)
          }
          break
        }
      }
    }
  }, [])

  const submit = async () => {
    const content = text.trim()
    if (!content) {
      toast.error("请详细描述您遇到的问题或诉求")
      textareaRef.current?.focus()
      return
    }

    const finalTitle = title.trim() ? title.trim() : deriveTicketTitle(content)

    try {
      const res = await createTicketMutation.mutateAsync({
        title: finalTitle,
        category,
        priority: "medium",
        content,
      })
      setText("")
      setTitle("")
      onCreated((res as { id: number }).id)
    } catch {
      // error handled by hook
    }
  }

  const sending = createTicketMutation.isPending || uploadingPaste
  const activeCategoryConfig = CATEGORY_CONFIG[category] || CATEGORY_CONFIG.technical

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-card">
      {/* Header */}
      <header className="flex shrink-0 items-center justify-between border-b border-border px-5 py-3.5 bg-muted/20">
        <div className="flex items-center gap-2.5">
          {onCancel && (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8 text-muted-foreground hover:text-foreground"
              onClick={onCancel}
              title="返回"
            >
              <ArrowLeft className="h-4 w-4" />
            </Button>
          )}
          <div>
            <h2 className="text-sm sm:text-base font-bold text-foreground">发起新工单</h2>
            <p className="text-xs text-muted-foreground">
              请选择问题分类并提供详细信息，我们的技术支持将尽快排查跟进
            </p>
          </div>
        </div>
        {onCancel && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8 text-xs text-muted-foreground hover:text-foreground"
            onClick={onCancel}
          >
            取消
          </Button>
        )}
      </header>

      {/* Form Content */}
      <div className="min-h-0 flex-1 overflow-y-auto p-4 sm:p-6">
        <div className="mx-auto max-w-2xl space-y-5">
          {/* Category selection */}
          <div className="space-y-2">
            <label className="text-xs font-semibold text-foreground flex items-center gap-1.5">
              <span>问题分类</span>
              <span className="text-primary font-mono">*</span>
            </label>
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
              {Object.entries(CATEGORY_CONFIG).map(([key, item]) => {
                const Icon = item.icon
                const isSelected = category === key
                return (
                  <button
                    key={key}
                    type="button"
                    onClick={() => setCategory(key)}
                    className={cn(
                      "flex flex-col items-start p-3 rounded-xl border text-left transition-all",
                      isSelected
                        ? "border-primary bg-primary/5 text-foreground shadow-2xs ring-1 ring-primary/40"
                        : "border-border/80 bg-background text-muted-foreground hover:bg-muted/40 hover:text-foreground"
                    )}
                  >
                    <div className="flex items-center gap-1.5 w-full">
                      <Icon className={cn("h-4 w-4 shrink-0", isSelected ? "text-primary" : "text-muted-foreground")} />
                      <span className={cn("text-xs font-semibold truncate", isSelected && "text-foreground")}>
                        {item.label}
                      </span>
                    </div>
                    <span className="text-[10px] text-muted-foreground mt-1 line-clamp-1">
                      {item.desc}
                    </span>
                  </button>
                )
              })}
            </div>
          </div>

          {/* Title Input */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-xs font-semibold text-foreground">
                工单主题 <span className="text-[11px] font-normal text-muted-foreground">（选填，留空将根据正文自动提炼）</span>
              </label>
            </div>
            <Input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="简要描述核心问题，例如：claude-3-5-sonnet 流式调用偶发 504 网关超时"
              className="h-10 text-sm rounded-xl"
              maxLength={200}
            />
          </div>

          {/* Detailed Content */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <label className="text-xs font-semibold text-foreground flex items-center gap-1">
                <span>问题描述与详细说明</span>
                <span className="text-primary font-mono">*</span>
              </label>
              <span className="text-[11px] text-muted-foreground">
                支持直接粘贴截图 (Ctrl/Cmd + V)
              </span>
            </div>
            <div className="relative">
              <Textarea
                ref={textareaRef}
                value={text}
                onChange={(e) => setText(e.target.value)}
                onPaste={(e) => void handlePaste(e)}
                rows={6}
                placeholder={activeCategoryConfig.placeholder}
                className="resize-y min-h-[130px] text-sm leading-relaxed rounded-xl p-3.5"
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault()
                    void submit()
                  }
                }}
              />
              {uploadingPaste && (
                <div className="absolute inset-0 flex items-center justify-center bg-background/80 rounded-xl backdrop-blur-xs text-xs font-medium text-primary gap-2">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  正在上传粘贴的截图…
                </div>
              )}
            </div>
          </div>

          {/* Attachments / Image Upload Bar */}
          <div className="flex flex-wrap items-center justify-between gap-2.5 rounded-xl border border-dashed border-border/80 bg-muted/25 px-3.5 py-2.5">
            <div className="flex items-center gap-2">
              <TicketImageUploadButton
                disabled={sending}
                onInsert={(md) => setText((t) => (t ? `${t}\n${md}` : md))}
              />
              <span className="text-xs text-muted-foreground">
                上传报错截图或请求响应示例（支持 JPEG / PNG / WebP，最大 4MB）
              </span>
            </div>
            <span className="text-[11px] text-muted-foreground hidden sm:inline-block">
              截图可辅助技术人员快速复现定位
            </span>
          </div>
        </div>
      </div>

      {/* Footer */}
      <footer className="flex shrink-0 items-center justify-between border-t border-border px-5 py-3 bg-muted/15">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Clock className="h-3.5 w-3.5 text-muted-foreground/80" />
          <span className="hidden sm:inline">工作时间内通常在 15-30 分钟内首次响应</span>
          <span className="sm:hidden">快捷键：⌘ + Enter</span>
        </div>
        <div className="flex items-center gap-2.5">
          {onCancel && (
            <Button type="button" variant="outline" size="sm" className="h-9 px-4 rounded-xl text-xs" onClick={onCancel}>
              取消
            </Button>
          )}
          <Button
            type="button"
            onClick={() => void submit()}
            disabled={sending || !text.trim()}
            className="h-9 px-5 rounded-xl text-xs font-semibold shadow-xs gap-1.5"
          >
            {sending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-3.5 w-3.5" />}
            <span>提交工单</span>
          </Button>
        </div>
      </footer>
    </div>
  )
})

type UserTicketChatPaneProps = {
  ticketId: string | undefined
  onTicketUpdated: () => void
  onNewConversation: (cat?: string, topic?: string) => void
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
  const [copied, setCopied] = useState(false)
  const [uploadingPaste, setUploadingPaste] = useState(false)
  const messagesEndRef = useRef<HTMLDivElement>(null)

  const { data: ticket, isLoading: loading, refetch } = useTicketDetail(ticketId)
  const replyMutation = useReplyTicket()

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" })
  }, [ticket?.replies?.length, ticketId])

  const copyId = useCallback(() => {
    if (!ticket?.id) return
    void navigator.clipboard.writeText(String(ticket.id))
    setCopied(true)
    toast.success(`已复制工单编号 #${ticket.id}`)
    setTimeout(() => setCopied(false), 2000)
  }, [ticket?.id])

  const handleReplyPaste = useCallback(async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = e.clipboardData?.items
    if (!items) return
    for (let i = 0; i < items.length; i++) {
      const item = items[i]
      if (item.type.startsWith("image/")) {
        const file = item.getAsFile()
        if (file) {
          e.preventDefault()
          setUploadingPaste(true)
          try {
            const md = await uploadTicketImageFile(file)
            setReplyContent((prev) => (prev ? `${prev}\n${md}` : md))
            toast.success("截图已插入回复")
          } catch (err) {
            toast.error(err instanceof Error ? err.message : "图片上传失败")
          } finally {
            setUploadingPaste(false)
          }
          break
        }
      }
    }
  }, [])

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
        // error handled by hook
      }
    },
    [ticketId, replyMutation, refetch, onTicketUpdated]
  )

  const handleClose = async () => {
    if (!ticketId) return
    const confirmed = window.confirm("确认要关闭此工单吗？关闭后将无法继续在此工单追问。")
    if (!confirmed) return
    setClosing(true)
    try {
      await replyMutation.mutateAsync({ ticketId, body: { content: "用户确认已解决并关闭工单" } })
      await refetch()
      onTicketUpdated()
      toast.success("工单已关闭")
    } catch {
      // error handled by hook
    } finally {
      setClosing(false)
    }
  }

  const sending = replyMutation.isPending || uploadingPaste

  // 1. Welcome / Guidance State when NO ticket is selected
  if (!ticketId) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center p-6 sm:p-10 text-center bg-card">
        <div className="max-w-xl flex flex-col items-center space-y-6">
          {/* Support Icon */}
          <div className="relative flex items-center justify-center">
            <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-primary/10 text-primary shadow-xs ring-8 ring-primary/5">
              <Headphones className="h-8 w-8 text-primary" />
            </div>
          </div>

          {/* Title & Introduction */}
          <div className="space-y-2">
            <h2 className="text-xl sm:text-2xl font-bold tracking-tight text-foreground">
              AI-GateWay 技术支持与工单中心
            </h2>
            <p className="text-sm text-muted-foreground leading-relaxed">
              针对接口异常排查、扣费疑问核算、模型配额调整及业务接入提供一对一专业技术跟进。
            </p>
          </div>

          {/* 4 Quick Category Assistance Cards */}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 w-full text-left pt-2">
            {Object.entries(CATEGORY_CONFIG).map(([catKey, item]) => {
              const Icon = item.icon
              return (
                <button
                  key={catKey}
                  type="button"
                  onClick={() => onNewConversation(catKey)}
                  className="group flex flex-col justify-between p-4 rounded-xl border border-border/80 bg-background/60 hover:bg-muted/40 hover:border-primary/40 transition-all text-left shadow-2xs hover:shadow-xs"
                >
                  <div className="flex items-center gap-2.5">
                    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary group-hover:scale-105 transition-transform">
                      <Icon className="h-4 w-4" />
                    </div>
                    <div>
                      <div className="text-xs font-bold text-foreground group-hover:text-primary transition-colors">
                        {item.label}
                      </div>
                      <div className="text-[11px] text-muted-foreground line-clamp-1 mt-0.5">
                        {item.desc}
                      </div>
                    </div>
                  </div>
                  <div className="mt-3 flex items-center text-[11px] font-medium text-primary opacity-90 group-hover:translate-x-0.5 transition-all">
                    <span>快捷发起工单 →</span>
                  </div>
                </button>
              )
            })}
          </div>

          {/* Primary Action & Notes */}
          <div className="flex flex-col items-center gap-3 pt-2">
            <Button
              type="button"
              size="default"
              onClick={() => onNewConversation()}
              className="rounded-xl px-6 font-semibold shadow-xs gap-2 h-10"
            >
              <Plus className="h-4 w-4" />
              <span>发起新工单</span>
            </Button>
            <div className="flex items-center gap-2 text-xs text-muted-foreground/80">
              <Clock className="h-3.5 w-3.5" />
              <span>工作时间内平均响应时效 15-30 分钟 · 支持直接上传报错日志截图</span>
            </div>
          </div>
        </div>
      </div>
    )
  }

  // 2. Loading State
  if (loading) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 text-sm text-muted-foreground bg-card">
        <Loader2 className="h-6 w-6 animate-spin text-primary" />
        <span>正在加载工单详情…</span>
      </div>
    )
  }

  // 3. Error State
  if (!ticket) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-4 px-4 text-center bg-card">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-destructive/10 text-destructive">
          <XCircle className="h-6 w-6" />
        </div>
        <div className="space-y-1">
          <p className="text-sm font-semibold text-foreground">无法加载该工单</p>
          <p className="text-xs text-muted-foreground">工单可能已被删除或您没有访问权限</p>
        </div>
        <Button variant="outline" size="sm" className="rounded-xl" onClick={onBackToList}>
          返回列表
        </Button>
      </div>
    )
  }

  // 4. Detail Chat View
  const statusCfg = STATUS_CONFIG[ticket.status] ?? {
    label: ticket.status,
    dotClass: "bg-muted-foreground",
    badgeClass: "bg-muted text-muted-foreground border-border",
  }
  const categoryCfg = CATEGORY_CONFIG[ticket.category] ?? {
    label: ticket.category || "技术支持",
    icon: Wrench,
    desc: "",
    placeholder: "",
  }
  const isClosed = ticket.status === "closed"

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-card">
      {/* Header */}
      <header className="shrink-0 border-b border-border px-4 py-3 bg-muted/15">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2 min-w-0">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8 shrink-0 text-muted-foreground hover:text-foreground lg:hidden"
              onClick={onBackToList}
              title="返回工单列表"
            >
              <ChevronLeft className="h-5 w-5" />
            </Button>
            <div className="min-w-0 space-y-1">
              <div className="flex items-center gap-2 flex-wrap">
                <button
                  type="button"
                  onClick={copyId}
                  className="group inline-flex items-center gap-1 rounded-md bg-muted/60 hover:bg-muted px-1.5 py-0.5 text-xs font-mono font-medium text-foreground transition-colors"
                  title="点击复制工单编号"
                >
                  <span>#{ticket.id}</span>
                  {copied ? (
                    <Check className="h-3 w-3 text-emerald-600" />
                  ) : (
                    <Copy className="h-3 w-3 text-muted-foreground group-hover:text-foreground" />
                  )}
                </button>
                <span
                  className={cn(
                    "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium",
                    statusCfg.badgeClass
                  )}
                >
                  <span className={cn("h-1.5 w-1.5 rounded-full", statusCfg.dotClass)} />
                  <span>{statusCfg.label}</span>
                </span>
                <span className="inline-flex items-center gap-1 rounded-full border border-border bg-background px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
                  <span>{categoryCfg.label}</span>
                </span>
              </div>
              <h2 className="truncate text-sm sm:text-base font-bold text-foreground" title={ticket.title}>
                {ticket.title}
              </h2>
            </div>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 px-3 text-xs rounded-xl hidden sm:inline-flex"
              onClick={() => onNewConversation()}
            >
              <Plus className="h-3.5 w-3.5 mr-1 text-muted-foreground" />
              <span>新建工单</span>
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-8 px-2.5 text-xs text-muted-foreground hover:text-foreground rounded-xl"
              onClick={() => void refetch()}
              title="刷新最新消息"
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </Button>
            {!isClosed && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={closing}
                onClick={() => void handleClose()}
                className="h-8 px-2.5 text-xs text-muted-foreground hover:text-destructive rounded-xl"
                title="关闭此工单"
              >
                <span>关闭工单</span>
              </Button>
            )}
          </div>
        </div>
      </header>

      {/* Message Stream */}
      <div className="min-h-0 flex-1 overflow-y-auto p-4 sm:p-5 space-y-4">
        {/* Ticket Created Notice Banner */}
        <div className="flex items-center justify-center">
          <div className="inline-flex items-center gap-2 rounded-full border border-border/60 bg-muted/30 px-3 py-1 text-[11px] text-muted-foreground">
            <Clock className="h-3 w-3" />
            <span>工单发起于 {fmtDateTime(ticket.created_at)}</span>
          </div>
        </div>

        {ticket.replies.length === 0 ? (
          <div className="flex flex-col items-center justify-center p-8 text-center space-y-2">
            <FileQuestion className="h-8 w-8 text-muted-foreground/50" />
            <p className="text-xs text-muted-foreground">
              管理员尚未回复，您也可以在下方继续补充说明或截图。
            </p>
          </div>
        ) : (
          ticket.replies.map((reply, idx) => {
            const isStaff = reply.is_staff
            const isInitialQuestion = idx === 0 && !isStaff

            return (
              <div
                key={reply.id}
                className={cn("flex flex-col", isStaff ? "items-start" : "items-end")}
              >
                <div
                  className={cn(
                    "flex max-w-[min(100%,600px)] gap-2.5",
                    isStaff ? "flex-row" : "flex-row-reverse"
                  )}
                >
                  {/* Avatar */}
                  <div
                    className={cn(
                      "flex h-8 w-8 shrink-0 items-center justify-center rounded-full border text-xs font-bold",
                      isStaff
                        ? "bg-primary/10 border-primary/25 text-primary"
                        : "bg-muted border-border/80 text-foreground"
                    )}
                  >
                    {isStaff ? (
                      <ShieldCheck className="h-4 w-4 text-primary" />
                    ) : (
                      <User className="h-4 w-4 text-muted-foreground" />
                    )}
                  </div>

                  {/* Bubble */}
                  <div className="flex flex-col space-y-1 min-w-0">
                    <div
                      className={cn(
                        "flex items-center gap-1.5 text-[11px] text-muted-foreground",
                        isStaff ? "justify-start" : "justify-end"
                      )}
                    >
                      <span className="font-semibold text-foreground/90">
                        {isStaff ? "技术支持 · 官方" : "我"}
                      </span>
                      {isInitialQuestion && (
                        <span className="rounded bg-muted px-1.5 py-0.2 text-[10px] text-muted-foreground border border-border/50">
                          首条问题
                        </span>
                      )}
                      <span>·</span>
                      <span className="tabular-nums">{fmtDateTime(reply.created_at)}</span>
                    </div>

                    <div
                      className={cn(
                        "rounded-2xl px-4 py-3 text-sm leading-relaxed shadow-2xs border",
                        isStaff
                          ? "bg-primary/5 dark:bg-primary/10 text-foreground border-primary/20 rounded-tl-xs"
                          : "bg-background text-foreground border-border/80 rounded-tr-xs"
                      )}
                    >
                      <TicketRichContent content={reply.content} />
                    </div>
                  </div>
                </div>
              </div>
            )
          })
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Reply Input Box */}
      {!isClosed ? (
        <div
          className="shrink-0 border-t border-border p-3.5 bg-background/95 backdrop-blur-xs sticky bottom-0"
          style={{ paddingBottom: "max(0.875rem, env(safe-area-inset-bottom, 0px))" }}
        >
          <div className="relative flex flex-col gap-2 rounded-xl border border-border/80 bg-muted/20 p-2 focus-within:border-primary/50 focus-within:ring-1 focus-within:ring-primary/40 transition-all">
            <Textarea
              value={replyContent}
              onChange={(e) => setReplyContent(e.target.value)}
              onPaste={(e) => void handleReplyPaste(e)}
              placeholder="输入回复内容（支持 Ctrl/Cmd+V 粘贴截图，Cmd/Ctrl + Enter 发送）…"
              rows={2}
              className="resize-none border-0 bg-transparent p-1.5 text-sm focus-visible:ring-0 focus-visible:ring-offset-0 min-h-[50px]"
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault()
                  void submitReply(replyContent)
                }
              }}
            />

            {uploadingPaste && (
              <div className="absolute inset-0 flex items-center justify-center bg-background/80 rounded-xl backdrop-blur-xs text-xs font-medium text-primary gap-2">
                <Loader2 className="h-4 w-4 animate-spin" />
                正在上传粘贴的截图…
              </div>
            )}

            <div className="flex items-center justify-between pt-1 border-t border-border/40">
              <div className="flex items-center gap-2">
                <TicketImageUploadButton
                  compact
                  disabled={sending}
                  onInsert={(md) => setReplyContent((t) => (t ? `${t}\n${md}` : md))}
                />
                <span className="text-[11px] text-muted-foreground hidden sm:inline">
                  支持上传或粘贴截图
                </span>
              </div>

              <div className="flex items-center gap-2">
                <span className="text-[11px] text-muted-foreground hidden sm:inline">
                  Cmd/Ctrl + Enter 发送
                </span>
                <Button
                  type="button"
                  onClick={() => void submitReply(replyContent)}
                  disabled={sending || !replyContent.trim()}
                  size="sm"
                  className="h-8 px-3 rounded-lg text-xs font-semibold gap-1.5 shadow-xs"
                >
                  {sending ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Send className="h-3.5 w-3.5" />}
                  <span>发送</span>
                </Button>
              </div>
            </div>
          </div>
        </div>
      ) : (
        <div className="shrink-0 border-t border-border p-4 text-center bg-muted/20">
          <div className="inline-flex flex-col sm:flex-row items-center justify-center gap-2 text-xs text-muted-foreground">
            <div className="inline-flex items-center gap-1.5 text-muted-foreground font-medium">
              <CheckCircle2 className="h-4 w-4 text-emerald-600" />
              <span>工单已关闭</span>
            </div>
            <span>·</span>
            <span>如问题未解决或有新需求，可随时发起新工单</span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 px-2.5 text-xs rounded-lg ml-1"
              onClick={() => onNewConversation()}
            >
              <Plus className="h-3 w-3 mr-1" />
              发起新工单
            </Button>
          </div>
        </div>
      )}
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
  const [searchQuery, setSearchQuery] = useState("")
  const [initialComposeCategory, setInitialComposeCategory] = useState<string>("technical")
  const [initialComposeTopic, setInitialComposeTopic] = useState<string>("")
  const qc = useQueryClient()

  const composeMode = searchParams.get("compose") === "1" && !routeTicketId

  const { data, isLoading: listBusy, refetch: loadList } = useTickets(
    page,
    pageSize,
    filterStatus || undefined
  )
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

  const goCompose = useCallback((category?: string, topic?: string) => {
    if (category) setInitialComposeCategory(category)
    if (topic) setInitialComposeTopic(topic)
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
      if (String(id) === routeTicketId) {
        goList()
        return
      }
      navigate(userTicketPath(id))
    },
    [routeTicketId, goList, navigate]
  )

  // Quick live search by ticket title, id, or category
  const filteredTickets = useMemo(() => {
    if (!searchQuery.trim()) return tickets
    const q = searchQuery.toLowerCase().trim()
    return tickets.filter(
      (t) =>
        String(t.id).includes(q) ||
        (t.title && t.title.toLowerCase().includes(q)) ||
        (CATEGORY_CONFIG[t.category]?.label || "").includes(q)
    )
  }, [tickets, searchQuery])

  // Mobile master-detail toggle: show only list OR only detail/compose (desktop keeps split)
  const detailOpen = Boolean(routeTicketId) || composeMode

  return (
    <div className="space-y-5">
      {/* Top Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div className="space-y-1">
          <div className="flex items-center gap-2.5">
            <h1 className="text-2xl sm:text-3xl font-extrabold tracking-tight text-foreground">
              工单支持
            </h1>
            <span className="inline-flex items-center gap-1 rounded-full border border-border bg-muted/60 px-2.5 py-0.5 text-xs font-medium text-muted-foreground">
              <Headphones className="h-3 w-3 text-primary" />
              <span>在线技术支持</span>
            </span>
          </div>
          <p className="text-xs sm:text-sm text-muted-foreground leading-relaxed">
            遇到接口调用异常、计费疑问或模型配额调整需求，可随时发起工单获取技术协助。
          </p>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <button
            type="button"
            onClick={() => void loadList()}
            disabled={listBusy}
            aria-label="刷新工单列表"
            className="btn btn-secondary h-9 px-3.5 text-xs font-semibold rounded-xl border-border shadow-2xs gap-1.5"
          >
            <RefreshCw className={cn("w-3.5 h-3.5", listBusy && "animate-spin text-primary")} />
            <span>刷新</span>
          </button>
          <button
            type="button"
            onClick={() => goCompose()}
            className="btn btn-primary h-9 px-4 text-xs font-semibold rounded-xl shadow-xs gap-1.5"
          >
            <Plus className="w-4 h-4" />
            <span>发起新工单</span>
          </button>
        </div>
      </div>

      {/* Main Split-Pane Workspace */}
      <div
        className={cn(
          "flex flex-col rounded-2xl border border-border/80 bg-card lg:flex-row lg:overflow-hidden shadow-xs",
          "min-h-[calc(100dvh-13rem)] lg:h-[min(100dvh-13rem,760px)]"
        )}
      >
        {/* Left Ticket List Sidebar */}
        <aside
          className={cn(
            "flex w-full flex-col border-border lg:w-[320px] xl:w-[350px] lg:shrink-0 lg:border-r lg:bg-muted/10",
            detailOpen && "hidden lg:flex"
          )}
        >
          {/* Out of list notice */}
          {routeTicketId && !selectedFromList && (
            <div className="shrink-0 border-b border-border bg-amber-50/60 dark:bg-amber-950/20 px-3 py-2 text-xs text-amber-800 dark:text-amber-300">
              当前工单 #{routeTicketId} 不在本页列表中。
            </div>
          )}

          {/* Status Filter Segmented Control */}
          <div className="shrink-0 border-b border-border p-2.5 space-y-2">
            <div className="flex items-center gap-1 rounded-xl bg-muted/60 p-1">
              {FILTER_KEYS.map((f) => (
                <button
                  key={f.key || "all"}
                  type="button"
                  onClick={() => handleFilter(f.key)}
                  className={cn(
                    "flex-1 min-h-7 rounded-lg text-xs font-semibold transition-all text-center",
                    filterStatus === f.key
                      ? "bg-background text-foreground shadow-2xs"
                      : "text-muted-foreground hover:text-foreground"
                  )}
                >
                  {f.label}
                </button>
              ))}
            </div>

            {/* Quick Search Input */}
            <div className="relative flex items-center">
              <Search className="absolute left-2.5 h-3.5 w-3.5 text-muted-foreground/70 pointer-events-none" />
              <input
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="搜索编号、标题或分类…"
                className="w-full h-8 pl-8 pr-3 text-xs rounded-lg border border-border/70 bg-background placeholder:text-muted-foreground/60 focus:outline-none focus:ring-1 focus:ring-primary/50"
              />
              {searchQuery && (
                <button
                  type="button"
                  onClick={() => setSearchQuery("")}
                  className="absolute right-2 text-xs text-muted-foreground hover:text-foreground"
                  title="清除搜索"
                >
                  ×
                </button>
              )}
            </div>
          </div>

          {/* Ticket Items Scroll Area */}
          <div className={cn("min-h-0 flex-1 overflow-y-auto", listBusy && tickets.length > 0 && "opacity-75")}>
            {listBusy && tickets.length === 0 ? (
              <div className="flex flex-col items-center justify-center p-8 text-center space-y-2 text-muted-foreground">
                <Loader2 className="h-5 w-5 animate-spin text-primary" />
                <span className="text-xs">加载工单列表中…</span>
              </div>
            ) : filteredTickets.length === 0 ? (
              <div className="flex flex-col items-center justify-center p-8 text-center space-y-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
                  <Ticket className="h-5 w-5" />
                </div>
                <div className="space-y-1">
                  <p className="text-xs font-semibold text-foreground">
                    {searchQuery ? "未找到匹配的工单" : "暂无工单记录"}
                  </p>
                  <p className="text-[11px] text-muted-foreground max-w-[200px]">
                    {searchQuery
                      ? "尝试更换关键词或清除搜索条件"
                      : "遇到问题时可随时发起新工单联系管理员"}
                  </p>
                </div>
                {searchQuery ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 text-xs"
                    onClick={() => setSearchQuery("")}
                  >
                    清除搜索
                  </Button>
                ) : (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-7 text-xs rounded-xl"
                    onClick={() => goCompose()}
                  >
                    发起新工单
                  </Button>
                )}
              </div>
            ) : (
              <ul className="divide-y divide-border/50">
                {filteredTickets.map((ticket) => {
                  const active = String(ticket.id) === routeTicketId
                  const st = STATUS_CONFIG[ticket.status] ?? {
                    label: ticket.status,
                    dotClass: "bg-muted-foreground",
                    badgeClass: "bg-muted text-muted-foreground border-border",
                  }
                  const cat = CATEGORY_CONFIG[ticket.category]?.label || ticket.category || "技术支持"

                  return (
                    <li key={ticket.id}>
                      <button
                        type="button"
                        onClick={() => selectTicket(ticket.id)}
                        className={cn(
                          "group relative flex w-full flex-col gap-1.5 p-3.5 text-left transition-colors",
                          active
                            ? "bg-primary/10 dark:bg-primary/15 text-foreground"
                            : "hover:bg-muted/40 active:bg-muted/60"
                        )}
                      >
                        {/* Active Accent Bar */}
                        {active && (
                          <div className="absolute left-0 top-0 bottom-0 w-1 bg-primary rounded-r" />
                        )}

                        {/* Top: #ID, Status Pill, Time */}
                        <div className="flex items-center justify-between gap-2">
                          <div className="flex items-center gap-1.5 min-w-0">
                            <span className="text-xs font-mono font-bold text-foreground">
                              #{ticket.id}
                            </span>
                            <span
                              className={cn(
                                "inline-flex items-center gap-1 rounded-full border px-1.5 py-0.2 text-[10px] font-medium",
                                st.badgeClass
                              )}
                            >
                              <span className={cn("h-1.5 w-1.5 rounded-full", st.dotClass)} />
                              <span>{st.label}</span>
                            </span>
                          </div>
                          <span className="shrink-0 text-[10px] text-muted-foreground tabular-nums">
                            {fmtRelativeOrShortTime(ticket.updated_at)}
                          </span>
                        </div>

                        {/* Middle: Title */}
                        <p
                          className={cn(
                            "text-xs leading-snug line-clamp-2",
                            active ? "font-semibold text-foreground" : "text-foreground/90 font-medium"
                          )}
                        >
                          {ticket.title || "未命名工单"}
                        </p>

                        {/* Bottom: Category Tag */}
                        <div className="flex items-center justify-between text-[10px] text-muted-foreground">
                          <span className="rounded bg-muted/60 px-1.5 py-0.5 border border-border/50">
                            {cat}
                          </span>
                        </div>
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
          </div>

          {/* Sidebar Footer Pagination */}
          {total > 0 && (
            <div className="flex shrink-0 items-center justify-between border-t border-border px-3 py-2 text-xs text-muted-foreground bg-muted/20">
              <span className="tabular-nums text-[11px]">
                共 {total} 条 · 第 {page}/{totalPages} 页
              </span>
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  disabled={page <= 1 || listBusy}
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                  className="rounded-lg p-1 text-muted-foreground hover:bg-background hover:text-foreground disabled:opacity-30 transition-colors"
                  aria-label="上一页"
                >
                  <ChevronLeft className="h-4 w-4" />
                </button>
                <button
                  type="button"
                  disabled={page >= totalPages || listBusy}
                  onClick={() => setPage((p) => p + 1)}
                  className="rounded-lg p-1 text-muted-foreground hover:bg-background hover:text-foreground disabled:opacity-30 transition-colors"
                  aria-label="下一页"
                >
                  <ChevronRight className="h-4 w-4" />
                </button>
              </div>
            </div>
          )}
        </aside>

        {/* Right Content Pane: Chat Thread or Compose or Welcome */}
        <div
          className={cn(
            "flex min-h-0 min-w-0 flex-1 flex-col",
            !detailOpen && "hidden lg:flex"
          )}
        >
          {composeMode ? (
            <UserComposePane
              initialCategory={initialComposeCategory}
              initialTopic={initialComposeTopic}
              onCreated={onTicketCreated}
              onCancel={goList}
            />
          ) : (
            <UserTicketChatPane
              ticketId={routeTicketId}
              onTicketUpdated={() => void loadList()}
              onNewConversation={(cat, topic) => goCompose(cat, topic)}
              onBackToList={goList}
            />
          )}
        </div>
      </div>
    </div>
  )
}
