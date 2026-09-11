import { useState, useMemo, type ReactNode } from "react"
import * as DialogPrimitive from "@radix-ui/react-dialog"
import {
  Plus,
  X,
  ChevronDown,
  Copy,
  Check,
  KeyRound,
  Sparkles,
  AlertTriangle,
  Clock,
  Layers,
  SlidersHorizontal,
  Loader2,
  Terminal,
  ShieldAlert,
} from "lucide-react"
import type { CreateKeyForm, AvailableGroup } from "../types"

interface Props {
  onCreate: (form: CreateKeyForm, onSuccess: (plaintext: string) => void) => Promise<void>
  groups: AvailableGroup[]
  groupsLoading: boolean
  trigger?: ReactNode
}

const EXPIRATION_PRESETS = [
  { label: "7 天", days: 7 },
  { label: "30 天", days: 30 },
  { label: "90 天", days: 90 },
] as const

const QUOTA_PRESETS = [
  { label: "不设限", value: "" },
  { label: "$10", value: "10" },
  { label: "$50", value: "50" },
  { label: "$100", value: "100" },
] as const

function formatExpirationDate(days: number): string {
  const d = new Date(Date.now() + days * 86400000)
  const y = d.getFullYear()
  const m = String(d.getMonth() + 1).padStart(2, "0")
  const day = String(d.getDate()).padStart(2, "0")
  const h = String(d.getHours()).padStart(2, "0")
  const min = String(d.getMinutes()).padStart(2, "0")
  return `${y}-${m}-${day} ${h}:${min}`
}

export function CreateApiKeyDialog({ onCreate, groups, groupsLoading, trigger }: Props) {
  const [open, setOpen] = useState(false)
  const [name, setName] = useState("")
  const [quota, setQuota] = useState("")
  const [rate5h, setRate5h] = useState("")
  const [rate1d, setRate1d] = useState("")
  const [rate7d, setRate7d] = useState("")
  const [rate30d, setRate30d] = useState("")
  const [creating, setCreating] = useState(false)
  const [createdKey, setCreatedKey] = useState<string | null>(null)
  const [copiedKey, setCopiedKey] = useState(false)
  const [copiedCurl, setCopiedCurl] = useState(false)
  const [curlProtocol, setCurlProtocol] = useState<"openai" | "claude">("openai")

  // Group selector state
  const [selectedGroupId, setSelectedGroupId] = useState<string>("")

  // Expiration picker state
  const [expirationMode, setExpirationMode] = useState<"preset" | "custom" | "permanent">("permanent")
  const [selectedPresetDays, setSelectedPresetDays] = useState<number | null>(null)
  const [customDays, setCustomDays] = useState("")

  // Collapsible advanced rate limits
  const [showAdvanced, setShowAdvanced] = useState(false)

  const resetForm = () => {
    setName("")
    setQuota("")
    setRate5h("")
    setRate1d("")
    setRate7d("")
    setRate30d("")
    setSelectedGroupId("")
    setExpirationMode("permanent")
    setSelectedPresetDays(null)
    setCustomDays("")
    setShowAdvanced(false)
    setCreatedKey(null)
    setCopiedKey(false)
    setCopiedCurl(false)
    setCurlProtocol("openai")
  }

  const getExpiresInDays = (): number | null | undefined => {
    if (expirationMode === "permanent") return undefined
    if (expirationMode === "preset") return selectedPresetDays ?? undefined
    if (expirationMode === "custom") {
      const val = parseInt(customDays, 10)
      return isNaN(val) || val <= 0 ? undefined : val
    }
    return undefined
  }

  const effectiveDays = getExpiresInDays()

  const expirationSummary = useMemo(() => {
    if (expirationMode === "permanent") {
      return { text: "永久有效（永不过期）", isPermanent: true }
    }
    if (effectiveDays && effectiveDays > 0) {
      return {
        text: `将于 ${formatExpirationDate(effectiveDays)} 到期（${effectiveDays} 天后）`,
        isPermanent: false,
      }
    }
    return { text: "请输入有效的到期天数", isPermanent: false }
  }, [expirationMode, effectiveDays])

  const handleSubmit = async (e: { preventDefault: () => void }) => {
    e.preventDefault()
    if (!name.trim()) return

    setCreating(true)
    try {
      const form: CreateKeyForm = {
        name: name.trim(),
        quota,
        rate_5h: rate5h,
        rate_1d: rate1d,
        rate_7d: rate7d,
        rate_30d: rate30d,
      }
      if (selectedGroupId) {
        form.group_id = parseInt(selectedGroupId, 10)
      }
      const expiresInDays = getExpiresInDays()
      if (expiresInDays !== undefined) {
        form.expires_in_days = expiresInDays
      }
      await onCreate(form, (plaintext) => {
        setCreatedKey(plaintext)
      })
    } finally {
      setCreating(false)
    }
  }

  const selectedGroup = groups.find((g) => String(g.id) === selectedGroupId)

  const origin = typeof window !== "undefined" ? window.location.origin : "http://localhost:8888"

  const sampleCurl = useMemo(() => {
    const key = createdKey || "agw-your-api-key"
    if (curlProtocol === "claude") {
      return `curl ${origin}/v1/messages \\
  -H "x-api-key: ${key}" \\
  -H "anthropic-version: 2023-06-01" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "claude-3-5-sonnet-20241022",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "Hello!"}]
  }'`
    }
    return `curl ${origin}/v1/chat/completions \\
  -H "Authorization: Bearer ${key}" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'`
  }, [createdKey, curlProtocol, origin])

  return (
    <DialogPrimitive.Root
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) resetForm()
      }}
    >
      <DialogPrimitive.Trigger asChild>
        {trigger ? (
          trigger
        ) : (
          <button className="btn btn-primary px-5 shadow-glow">
            <Plus className="h-4 w-4" />
            新建 Key
          </button>
        )}
      </DialogPrimitive.Trigger>

      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />

        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 flex flex-col w-full max-w-xl max-h-[88vh] translate-x-[-50%] translate-y-[-50%] border border-border bg-card text-card-foreground p-6 sm:p-7 shadow-2xl duration-200 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[state=closed]:slide-out-to-left-1/2 data-[state=closed]:slide-out-to-top-[48%] data-[state=open]:slide-in-from-left-1/2 data-[state=open]:slide-in-from-top-[48%] rounded-2xl overflow-hidden">
          {/* Header - Fixed */}
          <div className="flex items-start gap-3.5 mb-5 pr-7 shrink-0">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary ring-1 ring-primary/20">
              <KeyRound className="h-5 w-5" />
            </div>
            <div className="space-y-0.5 text-left">
              <DialogPrimitive.Title className="text-lg sm:text-xl font-bold tracking-tight text-foreground">
                {createdKey ? "API 密钥已生成" : "创建新的 API 密钥"}
              </DialogPrimitive.Title>
              <DialogPrimitive.Description className="text-xs sm:text-sm text-muted-foreground leading-relaxed">
                {createdKey
                  ? "密钥已成功生成并生效，请妥善保存明文凭证。"
                  : "配置密钥名称、限额与有效期。所有凭据均以 agw- 开头，统一兼容 OpenAI 与 Claude 原生端点。"}
              </DialogPrimitive.Description>
            </div>
          </div>

          {createdKey ? (
            /* ── Success view ── */
            <div className="space-y-4 overflow-y-auto flex-1 pr-1 pb-1 animate-in fade-in-50 duration-200">
              {/* Security Alert Banner */}
              <div className="flex items-start gap-3 p-3.5 rounded-xl bg-amber-500/10 border border-amber-500/20 text-amber-900 dark:text-amber-200 text-xs sm:text-sm leading-relaxed">
                <ShieldAlert className="h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400 mt-0.5" />
                <div>
                  <span className="font-semibold text-amber-800 dark:text-amber-300">安全提醒：</span>
                  此密钥明文仅在此处展示一次。关闭窗口后出于安全考虑将无法再次查看完整密钥，请立即复制并保存至安全位置。
                </div>
              </div>

              {/* Key Box */}
              <div className="space-y-1.5">
                <div className="flex items-center justify-between">
                  <label className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                    API 密钥明文 (Secret Key)
                  </label>
                  <span className="text-xs font-mono text-primary font-medium">agw- 格式</span>
                </div>
                <div className="flex items-center gap-2 p-2 rounded-xl border border-border bg-muted/40 dark:bg-dark-800/60 focus-within:border-primary/50 transition-colors">
                  <code className="flex-1 px-2.5 py-1 font-mono text-xs sm:text-sm text-foreground break-all select-all font-semibold">
                    {createdKey}
                  </code>
                  <button
                    type="button"
                    className="btn btn-primary shrink-0 px-3.5 py-2 text-xs font-medium shadow-glow"
                    onClick={() => {
                      void navigator.clipboard.writeText(createdKey)
                      setCopiedKey(true)
                      setTimeout(() => setCopiedKey(false), 2000)
                    }}
                  >
                    {copiedKey ? (
                      <>
                        <Check className="h-3.5 w-3.5 text-primary-foreground" />
                        <span>已复制</span>
                      </>
                    ) : (
                      <>
                        <Copy className="h-3.5 w-3.5" />
                        <span>复制密钥</span>
                      </>
                    )}
                  </button>
                </div>
              </div>

              {/* Quick Integration Snippet */}
              <div className="space-y-2 rounded-xl border border-border bg-muted/30 p-3.5">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <Terminal className="h-4 w-4 text-primary" />
                    <span className="text-xs font-semibold text-foreground">开箱即用测试指令</span>
                  </div>

                  <div className="flex items-center gap-2">
                    <div className="flex items-center gap-1 bg-background/80 p-0.5 rounded-lg border border-border text-xs">
                      <button
                        type="button"
                        className={`px-2 py-0.5 rounded-md font-medium transition-colors ${
                          curlProtocol === "openai"
                            ? "bg-primary text-primary-foreground shadow-2xs"
                            : "text-muted-foreground hover:text-foreground"
                        }`}
                        onClick={() => setCurlProtocol("openai")}
                      >
                        OpenAI
                      </button>
                      <button
                        type="button"
                        className={`px-2 py-0.5 rounded-md font-medium transition-colors ${
                          curlProtocol === "claude"
                            ? "bg-primary text-primary-foreground shadow-2xs"
                            : "text-muted-foreground hover:text-foreground"
                        }`}
                        onClick={() => setCurlProtocol("claude")}
                      >
                        Claude
                      </button>
                    </div>

                    <button
                      type="button"
                      className="btn btn-secondary py-1 px-2.5 h-7 text-xs flex items-center gap-1.5 shadow-2xs"
                      onClick={() => {
                        void navigator.clipboard.writeText(sampleCurl)
                        setCopiedCurl(true)
                        setTimeout(() => setCopiedCurl(false), 2000)
                      }}
                    >
                      {copiedCurl ? (
                        <>
                          <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
                          <span>已复制</span>
                        </>
                      ) : (
                        <>
                          <Copy className="h-3.5 w-3.5" />
                          <span>复制指令</span>
                        </>
                      )}
                    </button>
                  </div>
                </div>

                <div className="mt-2">
                  <pre className="overflow-x-auto rounded-lg bg-background dark:bg-dark-900 p-3 text-[11px] sm:text-xs font-mono text-muted-foreground border border-border/80 leading-relaxed max-h-36 select-all">
                    {sampleCurl}
                  </pre>
                </div>
              </div>

              {/* Bottom Complete Button */}
              <div className="pt-2 flex justify-end">
                <button
                  type="button"
                  className="btn btn-primary w-full sm:w-auto px-8 py-2.5 font-semibold shadow-glow"
                  onClick={() => {
                    setOpen(false)
                    resetForm()
                  }}
                >
                  我已保存密钥
                </button>
              </div>
            </div>
          ) : (
            /* ── Creation Form ── */
            <form onSubmit={handleSubmit} className="flex flex-col flex-1 min-h-0">
              <div className="space-y-4 overflow-y-auto flex-1 pr-1 pb-2">
                {/* Field 1: Name */}
                <div className="space-y-1">
                  <div className="flex items-center justify-between">
                    <label htmlFor="create-key-name" className="input-label mb-0">
                      密钥名称 <span className="text-destructive">*</span>
                    </label>
                    <span className="text-[11px] text-muted-foreground font-mono">
                      {name.length}/50
                    </span>
                  </div>
                  <input
                    id="create-key-name"
                    className="input text-sm"
                    placeholder="例如：本地开发环境 / Cursor IDE / 生产后端"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    maxLength={50}
                    required
                    autoFocus
                  />
                </div>

                {/* Field 2: Quota with Presets */}
                <div className="space-y-1">
                  <div className="flex items-center justify-between">
                    <label htmlFor="create-key-quota" className="input-label mb-0">
                      总额度上限 (USD)
                    </label>
                    <span className="text-xs text-muted-foreground">可选</span>
                  </div>
                  <div className="relative">
                    <div className="absolute inset-y-0 left-0 flex items-center pl-3.5 pointer-events-none text-muted-foreground font-medium text-sm">
                      $
                    </div>
                    <input
                      id="create-key-quota"
                      type="number"
                      className="input pl-8 text-sm"
                      placeholder="留空表示无限制"
                      value={quota}
                      onChange={(e) => setQuota(e.target.value)}
                      step="0.01"
                      min="0"
                    />
                  </div>
                  {/* Preset quota chips */}
                  <div className="flex items-center gap-1.5 pt-0.5">
                    <span className="text-[11px] text-muted-foreground">快捷填入：</span>
                    {QUOTA_PRESETS.map((p) => (
                      <button
                        key={p.label}
                        type="button"
                        className={`px-2 py-0.5 rounded-md text-xs transition-colors border ${
                          quota === p.value
                            ? "bg-primary/10 border-primary/30 text-primary font-medium"
                            : "bg-background border-border text-muted-foreground hover:bg-muted hover:text-foreground"
                        }`}
                        onClick={() => setQuota(p.value)}
                      >
                        {p.label}
                      </button>
                    ))}
                  </div>
                </div>

                {/* Field 3: Group Selector */}
                <div className="space-y-1">
                  <div className="flex items-center justify-between">
                    <label htmlFor="create-key-group" className="input-label mb-0">
                      绑定通道分组
                    </label>
                    <span className="text-xs text-muted-foreground">可选</span>
                  </div>
                  <div className="relative">
                    <div className="absolute inset-y-0 left-0 flex items-center pl-3.5 pointer-events-none text-muted-foreground">
                      <Layers className="h-4 w-4" />
                    </div>
                    <select
                      id="create-key-group"
                      className="input appearance-none pl-9 pr-9 cursor-pointer text-sm"
                      value={selectedGroupId}
                      onChange={(e) => setSelectedGroupId(e.target.value)}
                      disabled={groupsLoading}
                    >
                      <option value="">
                        {groupsLoading ? "正在载入通道分组..." : "不绑定分组（默认全量通道）"}
                      </option>
                      {groups.map((g) => (
                        <option key={g.id} value={String(g.id)}>
                          {g.name} — {g.subscription_type === "subscription" ? "订阅专享" : "标准通用"}
                          {g.rate_multiplier !== 1 ? ` (${g.rate_multiplier}x 费率)` : ""}
                        </option>
                      ))}
                    </select>
                    <ChevronDown className="absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground pointer-events-none" />
                  </div>

                  {/* Selected group preview callout */}
                  {selectedGroup && (
                    <div className="flex flex-col gap-1 mt-1.5 p-2.5 rounded-xl bg-muted/40 border border-border text-xs">
                      <div className="flex items-center gap-2">
                        <span
                          className={`inline-flex items-center rounded-md px-2 py-0.5 font-semibold text-[11px] ${
                            selectedGroup.subscription_type === "subscription"
                              ? "bg-violet-500/15 text-violet-700 dark:text-violet-300 border border-violet-500/20"
                              : "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300 border border-emerald-500/20"
                          }`}
                        >
                          {selectedGroup.subscription_type === "subscription" ? "订阅专享" : "标准通道"}
                        </span>
                        <span className="font-semibold text-foreground">{selectedGroup.name}</span>
                        <span className="text-muted-foreground font-mono">
                          倍率: {selectedGroup.rate_multiplier}x
                        </span>
                      </div>
                      {selectedGroup.description && (
                        <p className="text-muted-foreground leading-relaxed">
                          {selectedGroup.description}
                        </p>
                      )}
                      {selectedGroup.subscription_type === "subscription" && (
                        <div className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400 mt-0.5">
                          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                          <span>该分组需具备有效订阅方可调用；未订阅请求将被网关拒绝。</span>
                        </div>
                      )}
                    </div>
                  )}
                </div>

                {/* Field 4: Expiration Picker */}
                <div className="space-y-1.5">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-1.5">
                      <Clock className="h-3.5 w-3.5 text-muted-foreground" />
                      <span className="input-label mb-0">有效期</span>
                    </div>
                    <span className="text-xs text-muted-foreground font-mono">
                      {expirationMode === "permanent" ? "永不过期" : `${effectiveDays ?? 0} 天`}
                    </span>
                  </div>

                  {/* Segmented Preset Buttons */}
                  <div className="grid grid-cols-5 gap-1.5 sm:gap-2">
                    {EXPIRATION_PRESETS.map((preset) => {
                      const active = expirationMode === "preset" && selectedPresetDays === preset.days
                      return (
                        <button
                          key={preset.days}
                          type="button"
                          className={`py-2 rounded-xl text-xs sm:text-sm font-medium transition-all border ${
                            active
                              ? "bg-primary text-primary-foreground border-primary shadow-xs font-semibold"
                              : "bg-background border-border text-muted-foreground hover:border-border hover:bg-muted hover:text-foreground"
                          }`}
                          onClick={() => {
                            setExpirationMode("preset")
                            setSelectedPresetDays(preset.days)
                            setCustomDays("")
                          }}
                        >
                          {preset.label}
                        </button>
                      )
                    })}
                    <button
                      type="button"
                      className={`py-2 rounded-xl text-xs sm:text-sm font-medium transition-all border ${
                        expirationMode === "permanent"
                          ? "bg-primary text-primary-foreground border-primary shadow-xs font-semibold"
                          : "bg-background border-border text-muted-foreground hover:border-border hover:bg-muted hover:text-foreground"
                      }`}
                      onClick={() => {
                        setExpirationMode("permanent")
                        setSelectedPresetDays(null)
                        setCustomDays("")
                      }}
                    >
                      永久
                    </button>
                    <button
                      type="button"
                      className={`py-2 rounded-xl text-xs sm:text-sm font-medium transition-all border ${
                        expirationMode === "custom"
                          ? "bg-primary text-primary-foreground border-primary shadow-xs font-semibold"
                          : "bg-background border-border text-muted-foreground hover:border-border hover:bg-muted hover:text-foreground"
                      }`}
                      onClick={() => {
                        setExpirationMode("custom")
                        setSelectedPresetDays(null)
                      }}
                    >
                      自定义
                    </button>
                  </div>

                  {/* Custom days input */}
                  {expirationMode === "custom" && (
                    <div className="flex items-center gap-2 p-2 rounded-xl border border-border bg-muted/20 animate-in fade-in-50 duration-150">
                      <span className="text-xs text-muted-foreground whitespace-nowrap">有效天数：</span>
                      <input
                        id="create-key-custom-days"
                        type="number"
                        className="input py-1 px-3 h-8 w-28 text-sm"
                        placeholder="天数"
                        value={customDays}
                        onChange={(e) => setCustomDays(e.target.value)}
                        min="1"
                        max="3650"
                        autoFocus
                      />
                      <span className="text-xs text-muted-foreground">天（1 ~ 3650）</span>
                    </div>
                  )}

                  {/* Expiration live calculation preview */}
                  <div className="flex items-center gap-1.5 px-0.5 text-xs text-muted-foreground">
                    <Clock className="h-3 w-3 text-muted-foreground shrink-0" />
                    <span className={expirationSummary.isPermanent ? "text-primary font-medium" : ""}>
                      {expirationSummary.text}
                    </span>
                  </div>
                </div>

                {/* Field 5: Collapsible Advanced Rate Limits */}
                <div className="rounded-xl border border-border bg-muted/20 overflow-hidden transition-colors">
                  <button
                    type="button"
                    className="w-full flex items-center justify-between p-3 text-left text-xs font-medium text-foreground hover:bg-muted/40 transition-colors select-none"
                    onClick={() => setShowAdvanced(!showAdvanced)}
                  >
                    <div className="flex items-center gap-2">
                      <SlidersHorizontal className="h-3.5 w-3.5 text-primary" />
                      <span className="font-semibold">高级：时间窗口速率限制 (USD)</span>
                      <span className="rounded-full bg-muted px-2 py-0.2 text-[10px] text-muted-foreground">
                        可选
                      </span>
                    </div>
                    <ChevronDown
                      className={`h-4 w-4 text-muted-foreground transition-transform duration-200 ${
                        showAdvanced ? "rotate-180" : ""
                      }`}
                    />
                  </button>

                  {showAdvanced && (
                    <div className="p-3.5 pt-1 border-t border-border/60 space-y-3 animate-in fade-in-50 duration-200">
                      <p className="text-[11px] text-muted-foreground leading-relaxed">
                        基于滑动时间窗口的最高消费额（USD）。当特定周期内累计消费达到设定的限额时，网关将熔断拦截后续请求，有效防止突发高并发或死循环透支。
                      </p>
                      <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1">
                          <label htmlFor="create-key-rate-5h" className="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
                            5 小时限制
                          </label>
                          <div className="relative">
                            <span className="absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground text-xs pointer-events-none">
                              $
                            </span>
                            <input
                              id="create-key-rate-5h"
                              type="number"
                              placeholder="无限制"
                              className="input pl-7 py-1.5 text-xs"
                              value={rate5h}
                              onChange={(e) => setRate5h(e.target.value)}
                              step="0.01"
                              min="0"
                            />
                          </div>
                        </div>

                        <div className="space-y-1">
                          <label htmlFor="create-key-rate-1d" className="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
                            1 天限制 (24h)
                          </label>
                          <div className="relative">
                            <span className="absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground text-xs pointer-events-none">
                              $
                            </span>
                            <input
                              id="create-key-rate-1d"
                              type="number"
                              placeholder="无限制"
                              className="input pl-7 py-1.5 text-xs"
                              value={rate1d}
                              onChange={(e) => setRate1d(e.target.value)}
                              step="0.01"
                              min="0"
                            />
                          </div>
                        </div>

                        <div className="space-y-1">
                          <label htmlFor="create-key-rate-7d" className="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
                            7 天限制 (周)
                          </label>
                          <div className="relative">
                            <span className="absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground text-xs pointer-events-none">
                              $
                            </span>
                            <input
                              id="create-key-rate-7d"
                              type="number"
                              placeholder="无限制"
                              className="input pl-7 py-1.5 text-xs"
                              value={rate7d}
                              onChange={(e) => setRate7d(e.target.value)}
                              step="0.01"
                              min="0"
                            />
                          </div>
                        </div>

                        <div className="space-y-1">
                          <label htmlFor="create-key-rate-30d" className="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
                            30 天限制 (月)
                          </label>
                          <div className="relative">
                            <span className="absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground text-xs pointer-events-none">
                              $
                            </span>
                            <input
                              id="create-key-rate-30d"
                              type="number"
                              placeholder="无限制"
                              className="input pl-7 py-1.5 text-xs"
                              value={rate30d}
                              onChange={(e) => setRate30d(e.target.value)}
                              step="0.01"
                              min="0"
                            />
                          </div>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              </div>

              {/* Action Buttons - Sticky at Bottom */}
              <div className="flex items-center justify-end gap-3 pt-3 mt-2 border-t border-border shrink-0">
                <DialogPrimitive.Close asChild>
                  <button type="button" className="btn btn-secondary px-5 py-2 text-xs sm:text-sm">
                    取消
                  </button>
                </DialogPrimitive.Close>
                <button
                  type="submit"
                  className="btn btn-primary px-6 py-2 text-xs sm:text-sm font-semibold shadow-glow flex items-center gap-2"
                  disabled={creating || !name.trim()}
                >
                  {creating ? (
                    <>
                      <Loader2 className="h-4 w-4 animate-spin" />
                      <span>正在生成凭证...</span>
                    </>
                  ) : (
                    <>
                      <Sparkles className="h-4 w-4" />
                      <span>立刻创建</span>
                    </>
                  )}
                </button>
              </div>
            </form>
          )}

          {/* Close button */}
          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-xl p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted transition-colors focus:outline-none focus:ring-2 focus:ring-ring">
            <X className="h-4 w-4" />
            <span className="sr-only">Close</span>
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
