import { useState, useEffect } from "react"
import {
  X,
  Copy,
  Check,
  Shield,
  Clock,
  Globe,
  User,
  Hash,
  FileCode,
  DollarSign,
  Zap,
  Activity,
} from "lucide-react"
import { toast } from "sonner"
import {
  type AuditEntry,
  SOURCE_CONFIG,
  getActionMeta,
  formatExactTime,
  formatRelativeTime,
  getMethodBadgeClass,
  getStatusBadgeClass,
} from "./audit-utils"
import { Badge } from "@/shared/components/ui/badge"

interface Props {
  entry: AuditEntry | null
  onClose: () => void
}

export function AuditLogDetailDrawer({ entry, onClose }: Props) {
  const [copiedField, setCopiedField] = useState<string | null>(null)

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose()
    }
    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [onClose])

  if (!entry) return null

  const srcMeta = SOURCE_CONFIG[entry.source] || SOURCE_CONFIG.all
  const actMeta = getActionMeta(entry.action)
  const statusInfo = getStatusBadgeClass(entry.status_code)
  const SrcIcon = srcMeta.icon
  const ActIcon = actMeta.icon

  const copyText = (text: string, label: string) => {
    void navigator.clipboard.writeText(text)
    setCopiedField(label)
    toast.success(`已复制 ${label}`)
    setTimeout(() => setCopiedField(null), 1500)
  }

  const copyFullJson = () => {
    void navigator.clipboard.writeText(JSON.stringify(entry, null, 2))
    setCopiedField("json")
    toast.success("已复制完整记录 JSON")
    setTimeout(() => setCopiedField(null), 1500)
  }

  const metadata = entry.metadata || {}
  const hasMetadata = Object.keys(metadata).length > 0

  return (
    <div className="fixed inset-0 z-50 flex justify-end animate-in fade-in duration-150">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/40 backdrop-blur-xs transition-opacity"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Drawer Panel */}
      <div className="relative w-full max-w-lg bg-card h-full shadow-2xl border-l border-border flex flex-col animate-in slide-in-from-right duration-200 z-10">
        {/* Drawer Header */}
        <div className="px-6 py-4 border-b border-border flex items-center justify-between bg-card shrink-0">
          <div className="flex items-center gap-3 min-w-0">
            <div className="w-9 h-9 rounded-xl bg-primary/10 text-primary flex items-center justify-center shrink-0">
              <Shield className="w-5 h-5" />
            </div>
            <div className="min-w-0">
              <h2 className="text-base font-semibold text-foreground tracking-tight flex items-center gap-2">
                <span>审计记录详情</span>
                <span className="font-mono text-xs text-muted-foreground font-normal">
                  #{entry.id}
                </span>
              </h2>
              <p className="text-xs text-muted-foreground truncate">
                {srcMeta.label} · {actMeta.label}
              </p>
            </div>
          </div>

          <div className="flex items-center gap-1.5 shrink-0">
            <button
              type="button"
              onClick={copyFullJson}
              className="inline-flex items-center gap-1 h-8 px-2.5 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
              title="复制 JSON"
            >
              {copiedField === "json" ? (
                <Check className="w-3.5 h-3.5 text-emerald-500" />
              ) : (
                <Copy className="w-3.5 h-3.5" />
              )}
              <span>JSON</span>
            </button>
            <button
              type="button"
              onClick={onClose}
              aria-label="关闭详情"
              className="h-8 w-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>

        {/* Drawer Body */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6">
          {/* Summary Banner */}
          <div className="rounded-xl border border-border bg-muted/40 p-4 space-y-3">
            <div className="flex items-center justify-between gap-2 flex-wrap">
              <div className="flex items-center gap-2">
                <Badge variant="outline" className={`gap-1.5 py-1 px-2.5 ${srcMeta.badgeClass}`}>
                  <SrcIcon className="h-3.5 w-3.5" />
                  <span>{srcMeta.label}</span>
                </Badge>
                <Badge variant="outline" className="gap-1.5 py-1 px-2.5 bg-background font-medium">
                  <ActIcon className="h-3.5 w-3.5 text-primary" />
                  <span>{actMeta.label}</span>
                </Badge>
              </div>

              {entry.status_code ? (
                <span
                  className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-semibold ${statusInfo.badgeClass}`}
                >
                  <span className={`h-1.5 w-1.5 rounded-full ${statusInfo.dotClass}`} />
                  <span>HTTP {entry.status_code}</span>
                </span>
              ) : null}
            </div>

            <div className="flex items-center justify-between text-xs text-muted-foreground pt-1 border-t border-border/50">
              <span className="flex items-center gap-1.5">
                <Clock className="w-3.5 h-3.5" />
                <span>{formatRelativeTime(entry.created_at)}</span>
              </span>
              <span className="font-mono tabular-nums">
                {formatExactTime(entry.created_at)}
              </span>
            </div>
          </div>

          {/* Actor & Network Context */}
          <div className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-1.5">
              <User className="h-3.5 w-3.5" />
              <span>操作主体与网络信息</span>
            </h3>

            <div className="rounded-xl border border-border divide-y divide-border bg-card overflow-hidden text-sm">
              <div className="px-4 py-2.5 flex items-center justify-between gap-4">
                <span className="text-xs text-muted-foreground shrink-0">触发账号</span>
                <span className="font-medium truncate text-foreground">
                  {entry.actor_email || (entry.actor_id ? `UID: ${entry.actor_id}` : "系统服务")}
                </span>
              </div>

              {entry.actor_id ? (
                <div className="px-4 py-2.5 flex items-center justify-between gap-4">
                  <span className="text-xs text-muted-foreground shrink-0">用户 UID</span>
                  <span className="font-mono text-xs text-muted-foreground">
                    {entry.actor_id}
                  </span>
                </div>
              ) : null}

              <div className="px-4 py-2.5 flex items-center justify-between gap-4">
                <span className="text-xs text-muted-foreground shrink-0 flex items-center gap-1">
                  <Globe className="h-3.5 w-3.5" />
                  <span>客户端 IP</span>
                </span>
                <span className="font-mono text-xs text-foreground">
                  {entry.ip_address || "未记录"}
                </span>
              </div>

              {entry.request_id ? (
                <div className="px-4 py-2.5 flex items-center justify-between gap-4">
                  <span className="text-xs text-muted-foreground shrink-0 flex items-center gap-1">
                    <Hash className="h-3.5 w-3.5" />
                    <span>请求 ID</span>
                  </span>
                  <div className="flex items-center gap-1.5 min-w-0">
                    <code className="font-mono text-xs truncate max-w-[200px]" title={entry.request_id}>
                      {entry.request_id}
                    </code>
                    <button
                      type="button"
                      onClick={() => copyText(entry.request_id!, "请求 ID")}
                      className="text-muted-foreground hover:text-foreground p-1 rounded hover:bg-muted transition-colors"
                      title="复制请求 ID"
                    >
                      {copiedField === "请求 ID" ? (
                        <Check className="w-3.5 h-3.5 text-emerald-500" />
                      ) : (
                        <Copy className="w-3.5 h-3.5" />
                      )}
                    </button>
                  </div>
                </div>
              ) : null}
            </div>
          </div>

          {/* Target & Path */}
          <div className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-1.5">
              <Activity className="h-3.5 w-3.5" />
              <span>请求路由与目标资源</span>
            </h3>

            <div className="rounded-xl border border-border bg-card p-4 space-y-3">
              <div>
                <div className="text-xs text-muted-foreground mb-1">动作指令 (Action)</div>
                <div className="font-mono text-xs bg-muted/50 p-2 rounded-lg border border-border/80 break-all select-all">
                  {entry.action}
                </div>
              </div>

              {(entry.path || entry.target) && (
                <div>
                  <div className="text-xs text-muted-foreground mb-1">目标对象 / 路由</div>
                  <div className="flex items-center gap-2">
                    {entry.method && (
                      <span
                        className={`inline-flex items-center rounded-md border px-2 py-0.5 font-mono text-[11px] font-bold ${getMethodBadgeClass(
                          entry.method,
                        )}`}
                      >
                        {entry.method}
                      </span>
                    )}
                    <div className="font-mono text-xs bg-muted/50 p-2 rounded-lg border border-border/80 break-all select-all flex-1">
                      {entry.path || entry.target}
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Source Domain Specific Highlights */}
          {entry.source === "balance" && hasMetadata && (
            <div className="space-y-3">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-1.5">
                <DollarSign className="h-3.5 w-3.5" />
                <span>资金明细</span>
              </h3>
              <div className="rounded-xl border border-amber-200/60 dark:border-amber-900/40 bg-amber-500/5 p-4 space-y-2 text-sm">
                {"amount" in metadata && (
                  <div className="flex justify-between items-center py-1 border-b border-border/40">
                    <span className="text-muted-foreground text-xs">变动金额</span>
                    <span className="font-mono font-semibold text-foreground">
                      ${Number(metadata.amount || 0).toFixed(4)}
                    </span>
                  </div>
                )}
                {"balance_before" in metadata && (
                  <div className="flex justify-between items-center py-1 border-b border-border/40">
                    <span className="text-muted-foreground text-xs">变动前余额</span>
                    <span className="font-mono text-xs text-muted-foreground">
                      ${Number(metadata.balance_before || 0).toFixed(4)}
                    </span>
                  </div>
                )}
                {"balance_after" in metadata && (
                  <div className="flex justify-between items-center py-1 border-b border-border/40">
                    <span className="text-muted-foreground text-xs">变动后余额</span>
                    <span className="font-mono text-xs font-medium text-foreground">
                      ${Number(metadata.balance_after || 0).toFixed(4)}
                    </span>
                  </div>
                )}
                {"note" in metadata && Boolean(metadata.note) && (
                  <div className="flex justify-between items-center py-1">
                    <span className="text-muted-foreground text-xs">备注</span>
                    <span className="text-xs text-foreground">{String(metadata.note)}</span>
                  </div>
                )}
              </div>
            </div>
          )}

          {entry.source === "sdk" && hasMetadata && (
            <div className="space-y-3">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-1.5">
                <Zap className="h-3.5 w-3.5" />
                <span>API 代理调用信息</span>
              </h3>
              <div className="rounded-xl border border-emerald-200/60 dark:border-emerald-900/40 bg-emerald-500/5 p-4 space-y-2 text-sm">
                {"model" in metadata && (
                  <div className="flex justify-between items-center py-1 border-b border-border/40">
                    <span className="text-muted-foreground text-xs">模型</span>
                    <span className="font-mono font-medium text-xs text-foreground">
                      {String(metadata.model)}
                    </span>
                  </div>
                )}
                {"duration_ms" in metadata && (
                  <div className="flex justify-between items-center py-1 border-b border-border/40">
                    <span className="text-muted-foreground text-xs">响应耗时</span>
                    <span className="font-mono text-xs text-foreground">
                      {String(metadata.duration_ms)}ms
                    </span>
                  </div>
                )}
                {("tokens_in" in metadata || "tokens_out" in metadata) && (
                  <div className="flex justify-between items-center py-1 border-b border-border/40">
                    <span className="text-muted-foreground text-xs">Tokens (输入 / 输出)</span>
                    <span className="font-mono text-xs text-foreground">
                      {Number(metadata.tokens_in || 0).toLocaleString()} / {Number(metadata.tokens_out || 0).toLocaleString()}
                    </span>
                  </div>
                )}
                {"actual_cost" in metadata && (
                  <div className="flex justify-between items-center py-1">
                    <span className="text-muted-foreground text-xs">实扣费用</span>
                    <span className="font-mono font-semibold text-foreground">
                      ${Number(metadata.actual_cost || 0).toFixed(4)}
                    </span>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Metadata Inspector */}
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground flex items-center gap-1.5">
                <FileCode className="h-3.5 w-3.5" />
                <span>扩展元数据 (Metadata)</span>
              </h3>
              {hasMetadata && (
                <button
                  type="button"
                  onClick={() => copyText(JSON.stringify(metadata, null, 2), "元数据")}
                  className="text-xs text-muted-foreground hover:text-foreground inline-flex items-center gap-1"
                >
                  {copiedField === "元数据" ? (
                    <Check className="w-3 h-3 text-emerald-500" />
                  ) : (
                    <Copy className="w-3 h-3" />
                  )}
                  <span>复制元数据</span>
                </button>
              )}
            </div>

            <div className="rounded-xl border border-border bg-muted/40 p-3 overflow-hidden">
              {hasMetadata ? (
                <pre className="font-mono text-xs leading-relaxed overflow-x-auto text-foreground p-1 select-all whitespace-pre-wrap break-all max-h-64">
                  {JSON.stringify(metadata, null, 2)}
                </pre>
              ) : (
                <div className="text-xs text-muted-foreground py-2 text-center">
                  该记录未附带额外元数据
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
