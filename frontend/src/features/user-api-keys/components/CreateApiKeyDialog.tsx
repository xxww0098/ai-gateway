import { useState } from "react"
import * as DialogPrimitive from "@radix-ui/react-dialog"
import { Plus, X, ChevronDown } from "lucide-react"
import type { CreateKeyForm, AvailableGroup } from "../types"

interface Props {
  onCreate: (form: CreateKeyForm, onSuccess: () => void) => Promise<void>
  groups: AvailableGroup[]
  groupsLoading: boolean
}

const EXPIRATION_PRESETS = [
  { label: "7 天", days: 7 },
  { label: "30 天", days: 30 },
  { label: "90 天", days: 90 },
] as const

export function CreateApiKeyDialog({ onCreate, groups, groupsLoading }: Props) {
  const [open, setOpen] = useState(false)
  const [name, setName] = useState("")
  const [quota, setQuota] = useState("")
  const [rate5h, setRate5h] = useState("")
  const [rate1d, setRate1d] = useState("")
  const [rate7d, setRate7d] = useState("")
  const [rate30d, setRate30d] = useState("")
  const [creating, setCreating] = useState(false)

  // Group selector state
  const [selectedGroupId, setSelectedGroupId] = useState<string>("")

  // Expiration picker state
  const [expirationMode, setExpirationMode] = useState<"preset" | "custom" | "permanent">("permanent")
  const [selectedPresetDays, setSelectedPresetDays] = useState<number | null>(null)
  const [customDays, setCustomDays] = useState("")

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

  const handleSubmit = async (e: { preventDefault: () => void }) => {
    e.preventDefault()
    setCreating(true)
    const form: CreateKeyForm = {
      name,
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
    await onCreate(form, () => {
      setOpen(false)
      resetForm()
    })
    setCreating(false)
  }

  const selectedGroup = groups.find((g) => String(g.id) === selectedGroupId)

  return (
    <DialogPrimitive.Root open={open} onOpenChange={setOpen}>
      <DialogPrimitive.Trigger asChild>
        <button className="btn btn-primary px-5 shadow-glow">
          <Plus className="h-4 w-4" />
          新建 Key
        </button>
      </DialogPrimitive.Trigger>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-lg translate-x-[-50%] translate-y-[-50%] gap-4 border border-border bg-white dark:bg-dark-900 p-6 shadow-2xl duration-200 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[state=closed]:slide-out-to-left-1/2 data-[state=closed]:slide-out-to-top-[48%] data-[state=open]:slide-in-from-left-1/2 data-[state=open]:slide-in-from-top-[48%] sm:rounded-2xl max-h-[85vh] overflow-y-auto">
          <div className="flex flex-col space-y-1.5 text-center sm:text-left mb-6">
            <DialogPrimitive.Title className="text-xl font-semibold leading-none tracking-tight">创建新的 API Key</DialogPrimitive.Title>
            <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300">
              为这个 Key 命名，这有助于您区分它的用途。
            </DialogPrimitive.Description>
          </div>
          <form onSubmit={handleSubmit} className="space-y-4">
            {/* Name */}
            <div className="space-y-1.5">
              <label className="input-label">名称</label>
              <input
                className="input"
                placeholder="例如：本地开发测试环境"
                value={name}
                onChange={(e) => setName(e.target.value)}
                maxLength={50}
                required
              />
            </div>

            {/* Quota */}
            <div className="space-y-1.5">
              <label className="input-label">总额度上限 (可选)</label>
              <input
                type="number"
                className="input"
                placeholder="留空表示无限制"
                value={quota}
                onChange={(e) => setQuota(e.target.value)}
                step="0.01"
                min="0"
              />
            </div>

            {/* Group Selector */}
            <div className="space-y-1.5">
              <label className="input-label">绑定分组 (可选)</label>
              <div className="relative">
                <select
                  className="input appearance-none pr-8 cursor-pointer"
                  value={selectedGroupId}
                  onChange={(e) => setSelectedGroupId(e.target.value)}
                  disabled={groupsLoading}
                >
                  <option value="">
                    {groupsLoading ? "加载中..." : "不绑定分组"}
                  </option>
                  {groups.map((g) => (
                    <option key={g.id} value={String(g.id)}>
                      {g.name} — {g.subscription_type === "subscription" ? "订阅" : "标准"}
                    </option>
                  ))}
                </select>
                <ChevronDown className="absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-gray-400 pointer-events-none" />
              </div>
              {selectedGroup && (
                <div className="flex items-center gap-2 mt-2 p-2.5 rounded-lg bg-gray-50 dark:bg-dark-800/60 border border-gray-100 dark:border-dark-700">
                  <span
                    className={`inline-flex items-center rounded-md px-2 py-0.5 text-xs font-semibold ${
                      selectedGroup.subscription_type === "subscription"
                        ? "bg-violet-100 text-violet-700 dark:bg-violet-900/30 dark:text-violet-400"
                        : "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400"
                    }`}
                  >
                    {selectedGroup.subscription_type === "subscription" ? "订阅" : "标准"}
                  </span>
                  <span className="text-sm text-gray-700 dark:text-dark-200 font-medium">{selectedGroup.name}</span>
                  {selectedGroup.description && (
                    <span className="text-xs text-gray-400 dark:text-dark-400 truncate">{selectedGroup.description}</span>
                  )}
                </div>
              )}
            </div>

            {/* Expiration Picker */}
            <div className="space-y-2">
              <label className="input-label">有效期</label>
              <div className="flex gap-2 flex-wrap">
                {EXPIRATION_PRESETS.map((preset) => (
                  <button
                    key={preset.days}
                    type="button"
                    className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all border ${
                      expirationMode === "preset" && selectedPresetDays === preset.days
                        ? "bg-blue-50 border-blue-300 text-blue-700 dark:bg-blue-900/30 dark:border-blue-600 dark:text-blue-400 shadow-sm"
                        : "bg-white border-gray-200 text-gray-600 hover:border-gray-300 hover:bg-gray-50 dark:bg-dark-800 dark:border-dark-600 dark:text-dark-300 dark:hover:border-dark-500"
                    }`}
                    onClick={() => {
                      setExpirationMode("preset")
                      setSelectedPresetDays(preset.days)
                      setCustomDays("")
                    }}
                  >
                    {preset.label}
                  </button>
                ))}
                <button
                  type="button"
                  className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all border ${
                    expirationMode === "permanent"
                      ? "bg-blue-50 border-blue-300 text-blue-700 dark:bg-blue-900/30 dark:border-blue-600 dark:text-blue-400 shadow-sm"
                      : "bg-white border-gray-200 text-gray-600 hover:border-gray-300 hover:bg-gray-50 dark:bg-dark-800 dark:border-dark-600 dark:text-dark-300 dark:hover:border-dark-500"
                  }`}
                  onClick={() => {
                    setExpirationMode("permanent")
                    setSelectedPresetDays(null)
                    setCustomDays("")
                  }}
                >
                  永久
                </button>
              </div>
              <div className="flex items-center gap-2 mt-1">
                <button
                  type="button"
                  className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all border ${
                    expirationMode === "custom"
                      ? "bg-blue-50 border-blue-300 text-blue-700 dark:bg-blue-900/30 dark:border-blue-600 dark:text-blue-400 shadow-sm"
                      : "bg-white border-gray-200 text-gray-600 hover:border-gray-300 hover:bg-gray-50 dark:bg-dark-800 dark:border-dark-600 dark:text-dark-300 dark:hover:border-dark-500"
                  }`}
                  onClick={() => {
                    setExpirationMode("custom")
                    setSelectedPresetDays(null)
                  }}
                >
                  自定义
                </button>
                {expirationMode === "custom" && (
                  <div className="flex items-center gap-1.5">
                    <input
                      type="number"
                      className="input w-24"
                      placeholder="天数"
                      value={customDays}
                      onChange={(e) => setCustomDays(e.target.value)}
                      min="1"
                      max="3650"
                      autoFocus
                    />
                    <span className="text-sm text-gray-500 dark:text-dark-400">天</span>
                  </div>
                )}
              </div>
              <p className="text-xs text-gray-400 dark:text-dark-500">
                {expirationMode === "permanent"
                  ? "此 Key 将永不过期"
                  : expirationMode === "preset"
                    ? `将在 ${selectedPresetDays} 天后过期`
                    : customDays
                      ? `将在 ${customDays} 天后过期`
                      : "请输入自定义天数"}
              </p>
            </div>

            {/* Rate Limits */}
            <details className="group border border-border rounded-xl p-4 bg-gray-50/50 dark:bg-dark-800/50 cursor-pointer outline-none">
              <summary className="text-sm font-medium text-gray-900 dark:text-white outline-none select-none cursor-pointer">高级：时间窗口速率限制 (USD)</summary>
              <div className="mt-4 grid grid-cols-2 gap-4">
                <div className="space-y-1.5">
                  <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase">5h 限制</label>
                  <input type="number" placeholder="无限制" className="input" value={rate5h} onChange={(e) => setRate5h(e.target.value)} step="0.01" min="0" />
                </div>
                <div className="space-y-1.5">
                  <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase">1 天限制</label>
                  <input type="number" placeholder="无限制" className="input" value={rate1d} onChange={(e) => setRate1d(e.target.value)} step="0.01" min="0" />
                </div>
                <div className="space-y-1.5">
                  <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase">7 天限制</label>
                  <input type="number" placeholder="无限制" className="input" value={rate7d} onChange={(e) => setRate7d(e.target.value)} step="0.01" min="0" />
                </div>
                <div className="space-y-1.5">
                  <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase">30 天限制</label>
                  <input type="number" placeholder="无限制" className="input" value={rate30d} onChange={(e) => setRate30d(e.target.value)} step="0.01" min="0" />
                </div>
              </div>
            </details>

            <div className="flex justify-end gap-3 mt-6 pt-2">
              <DialogPrimitive.Close asChild>
                <button type="button" className="btn btn-ghost px-5">取消</button>
              </DialogPrimitive.Close>
              <button type="submit" className="btn btn-primary px-6" disabled={creating || !name.trim()}>
                {creating ? "创建中..." : "立刻创建"}
              </button>
            </div>
          </form>
          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-sm opacity-70 ring-offset-background transition-opacity hover:opacity-100 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:pointer-events-none data-[state=open]:bg-accent data-[state=open]:text-muted-foreground">
            <X className="h-4 w-4" />
            <span className="sr-only">Close</span>
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
