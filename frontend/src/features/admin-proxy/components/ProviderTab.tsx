import { useEffect, useRef, useState, useCallback, memo } from 'react'
import type { ReactNode } from 'react'
import { Button } from '@/shared/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/shared/components/ui/dialog'
import { Loader2, RefreshCcw, Plus, Trash2, Globe, Search, Pencil, KeyRound, Sparkles, SlidersHorizontal, ChevronDown, ChevronUp, Copy, Check } from 'lucide-react'
import { toast } from 'sonner'
import { copyTextToClipboard } from '@/shared/utils/clipboard'
import { fetchProviderConfig, updateProviderConfig, fetchApiKeyUsage } from '../api'
import { Input } from '@/shared/components/ui/input'
import { Textarea } from '@/shared/components/ui/textarea'
import { Switch } from '@/shared/components/ui/switch'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/components/ui/table'
import { EmptyState } from '@/shared/components/EmptyState'
import { cn } from '@/shared/utils/utils'
import {
  buildProviderAddArray,
  buildProviderDeleteArray,
  buildProviderEditArray,
  normalizeProviderItems,
  providerLabel,
  type ApiKeyUsageResponse,
  type BaseChannelItem,
  type ProviderKind,
  type ProviderStructuredForm,
} from '../providerConfig'
import {
  matchOpenAiCompatPreset,
  openAiCompatPresetForm,
  OPENAI_COMPAT_PRESETS,
  type OpenAiCompatPreset,
} from '../openaiCompatPresets'
import { AuthProviderBrandIcon } from './AuthProviderBrandIcon'
import { OpenAiEditDialogBody } from './OpenAiEditDialogBody'

export type { BaseChannelItem } from '../providerConfig'

interface ProviderTabProps {
  providerKind: ProviderKind
  endpoint: string
  refreshSignal?: number
  onOpenModelsDialog?: (item: BaseChannelItem) => void
  onProbeForm?: (apiKey: string, baseUrl: string, name: string, modelsUrl?: string, providerKind?: ProviderKind) => void
}

const emptyForm: ProviderStructuredForm = {
  name: '',
  apiKey: '',
  baseUrl: '',
  modelsUrl: '',
  proxyUrl: '',
  apiKeyProxyUrl: '',
  priority: '',
  prefix: '',
  headersText: '',
  modelsText: '',
  excludedModelsText: '',
  advancedText: '',
  disabled: false,
  websockets: false,
  experimentalCCHSigning: false,
}

export const ProviderTab = memo(function ProviderTab({ providerKind, endpoint, refreshSignal, onOpenModelsDialog, onProbeForm }: ProviderTabProps) {
  const [items, setItems] = useState<BaseChannelItem[]>([])
  const [loading, setLoading] = useState(true)
  const [newForm, setNewForm] = useState<ProviderStructuredForm>(emptyForm)
  const [editingItem, setEditingItem] = useState<BaseChannelItem | null>(null)
  const [editForm, setEditForm] = useState<ProviderStructuredForm>(emptyForm)
  const [showAdvanced, setShowAdvanced] = useState(false)
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const mountedRef = useRef(false)

  const provider = providerLabel(providerKind)
  const isOpenAI = providerKind === 'openai'

  const handleCopyKey = async (key?: string) => {
    if (!key) return
    const ok = await copyTextToClipboard(key)
    if (ok) {
      setCopiedKey(key)
      toast.success('API Key 已复制')
      setTimeout(() => setCopiedKey(null), 1800)
    }
  }

  const fetchItems = useCallback(async (silent = false) => {
    if (!silent) setLoading(true)
    try {
      const [data, usage] = await Promise.all([
        fetchProviderConfig(endpoint),
        fetchApiKeyUsage<ApiKeyUsageResponse>().catch(() => undefined),
      ])
      setItems(normalizeProviderItems(providerKind, data, usage))
    } catch (e: unknown) {
      toast.error(`获取 ${provider} 数据失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setLoading(false)
    }
  }, [endpoint, provider, providerKind])

  useEffect(() => {
    const timer = window.setTimeout(() => void fetchItems(), 0)
    return () => window.clearTimeout(timer)
  }, [fetchItems])

  useEffect(() => {
    if (!mountedRef.current) {
      mountedRef.current = true
      return
    }
    void fetchItems(true)
  }, [refreshSignal, fetchItems])

  const updateNewForm = (patch: ProviderStructuredForm) => setNewForm(prev => ({ ...prev, ...patch }))
  const updateEditForm = (patch: ProviderStructuredForm) => setEditForm(prev => ({ ...prev, ...patch }))

  /** 内置平台预设：只改连接字段，已经填好的 API Key 留着。 */
  const applyPreset = (preset: OpenAiCompatPreset) => updateNewForm(openAiCompatPresetForm(preset))
  const activePreset = isOpenAI ? matchOpenAiCompatPreset({ name: newForm.name, baseUrl: newForm.baseUrl }) : undefined

  const handleAdd = async () => {
    try {
      const latest = await fetchProviderConfig(endpoint)
      const updatedArray = buildProviderAddArray(providerKind, latest, newForm)
      await updateProviderConfig(endpoint, updatedArray)
      toast.success('添加成功')
      setNewForm(emptyForm)
      fetchItems(true)
    } catch (e) {
      toast.error(`添加失败: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  const handleDelete = async (item: BaseChannelItem) => {
    try {
      const latest = await fetchProviderConfig(endpoint)
      const updatedArray = buildProviderDeleteArray(providerKind, latest, item)
      await updateProviderConfig(endpoint, updatedArray)
      toast.success('删除成功')
      if (editingItem?._id === item._id) setEditingItem(null)
      fetchItems(true)
    } catch (e) {
      toast.error(`删除失败: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  const startEdit = (item: BaseChannelItem) => {
    setEditingItem(item)
    setEditForm(formFromItem(item))
  }

  /** `closeDialog=false` 时不弹「配置已更新」（由调用方提示），且不关闭弹窗 */
  const persistEdit = async (formOverride?: ProviderStructuredForm, closeDialog = true) => {
    if (!editingItem) return
    const f = formOverride ?? editForm
    try {
      const latest = await fetchProviderConfig(endpoint)
      const updatedArray = buildProviderEditArray(providerKind, latest, editingItem, f)
      await updateProviderConfig(endpoint, updatedArray)
      setEditForm(f)
      if (closeDialog) {
        toast.success('配置已更新')
        setEditingItem(null)
      }
      void fetchItems(true)
    } catch (e) {
      toast.error(`更新失败: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  const handleOpenModelsDialog = (item: BaseChannelItem) => {
    onOpenModelsDialog?.(item)
  }

  return (
    <div className="space-y-6 mt-3">
      {/* Form Card */}
      <div className="rounded-xl border border-border/80 bg-card p-4 sm:p-5 shadow-2xs space-y-4">
        {isOpenAI && (
          <div className="space-y-2.5 pb-3.5 border-b border-border/60">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-1.5">
                <Sparkles className="h-3.5 w-3.5 text-primary" />
                <span className="text-xs font-semibold text-foreground">内置平台快捷预设</span>
              </div>
              <span className="text-[11px] text-muted-foreground">
                选择后自动填入 Base URL、前缀与模型规则，仅需再填 API Key
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-1.5 sm:gap-2">
              {OPENAI_COMPAT_PRESETS.map((preset) => {
                const isSelected = activePreset?.key === preset.key
                return (
                  <button
                    key={preset.key}
                    type="button"
                    className={cn(
                      "inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-lg font-medium transition-all border cursor-pointer select-none",
                      isSelected
                        ? "bg-primary/10 text-primary border-primary/40 ring-1 ring-primary/20 shadow-2xs"
                        : "bg-muted/40 hover:bg-muted/80 text-muted-foreground hover:text-foreground border-border/60"
                    )}
                    onClick={() => applyPreset(preset)}
                    title={`${preset.label}：Base URL / 前缀 / 模型列表 URL 由预设写入，只需再填 API Key`}
                  >
                    <AuthProviderBrandIcon provider={preset.iconProvider} size={15} />
                    <span>{preset.label}</span>
                    {isSelected && <Check className="h-3 w-3 text-primary ml-0.5" />}
                  </button>
                )
              })}
            </div>
          </div>
        )}

        {/* Primary Inputs */}
        <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
          {isOpenAI && (
            <Field label="渠道标识" required>
              <Input
                className="bg-background shadow-2xs h-9 text-xs sm:text-sm"
                placeholder="例如: openrouter"
                value={newForm.name || ''}
                onChange={(e) => updateNewForm({ name: e.target.value })}
              />
            </Field>
          )}
          <Field label="网关端点 (Base URL)" required={isOpenAI}>
            <Input
              className="bg-background shadow-2xs h-9 text-xs sm:text-sm font-mono"
              placeholder={isOpenAI ? "https://..." : "留空使用官方默认网关"}
              value={newForm.baseUrl || ''}
              onChange={(e) => updateNewForm({ baseUrl: e.target.value })}
            />
          </Field>
          {isOpenAI && (
            <Field label="模型列表 URL" optional>
              <Input
                className="bg-background shadow-2xs h-9 text-xs sm:text-sm font-mono"
                placeholder="留空按 Base URL 推导 /models"
                value={newForm.modelsUrl || ''}
                onChange={(e) => updateNewForm({ modelsUrl: e.target.value })}
              />
            </Field>
          )}
          <Field label="API Key / 凭证密钥" required>
            <Input
              type="password"
              className="bg-background shadow-2xs h-9 text-xs sm:text-sm font-mono"
              placeholder="输入凭证密钥..."
              value={newForm.apiKey || ''}
              onChange={(e) => updateNewForm({ apiKey: e.target.value })}
              onKeyDown={(e) => e.key === 'Enter' && handleAdd()}
            />
          </Field>
        </div>

        {/* Collapsible Advanced Settings */}
        <div className="pt-1">
          <button
            type="button"
            onClick={() => setShowAdvanced((v) => !v)}
            className="inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors cursor-pointer py-1 select-none"
          >
            <SlidersHorizontal className="h-3.5 w-3.5" />
            <span>高级路由与自定义模型配置</span>
            {showAdvanced ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
            {Boolean(newForm.proxyUrl || newForm.apiKeyProxyUrl || newForm.prefix || newForm.priority || newForm.modelsText) && (
              <span className="w-1.5 h-1.5 rounded-full bg-primary" />
            )}
          </button>

          {showAdvanced && (
            <div className="mt-3 p-3.5 rounded-lg border border-border/60 bg-muted/20 space-y-3.5">
              <div className="grid gap-3 sm:grid-cols-3">
                <Field label={isOpenAI ? 'Key Proxy URL' : 'Proxy URL'} optional>
                  <Input
                    className="bg-background shadow-2xs h-8.5 text-xs font-mono"
                    value={(isOpenAI ? newForm.apiKeyProxyUrl : newForm.proxyUrl) || ''}
                    onChange={(e) => updateNewForm(isOpenAI ? { apiKeyProxyUrl: e.target.value } : { proxyUrl: e.target.value })}
                    placeholder="direct / http://..."
                  />
                </Field>
                <Field label="模型路由前缀" optional>
                  <Input
                    className="bg-background shadow-2xs h-8.5 text-xs font-mono"
                    value={newForm.prefix || ''}
                    onChange={(e) => updateNewForm({ prefix: e.target.value })}
                    placeholder="例如: teamA"
                  />
                </Field>
                <Field label="调度优先级 (Priority)" optional>
                  <Input
                    className="bg-background shadow-2xs h-8.5 text-xs"
                    type="number"
                    value={newForm.priority || ''}
                    onChange={(e) => updateNewForm({ priority: e.target.value })}
                    placeholder="0 (数字越小越优先)"
                  />
                </Field>
              </div>

              <Field label="自定义模型列表 (Models JSON)" optional>
                <Textarea
                  className="font-mono text-xs h-20 bg-background shadow-2xs resize-y"
                  value={newForm.modelsText || ''}
                  onChange={(e) => updateNewForm({ modelsText: e.target.value })}
                  placeholder='[{"name":"gpt-4o","alias":"gpt-4"}]'
                />
              </Field>
            </div>
          )}
        </div>

        {/* Action Row */}
        <div className="flex flex-wrap items-center justify-between gap-3 pt-3 border-t border-border/60">
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => fetchItems(false)}
              disabled={loading}
              className="h-8.5 px-3 text-xs gap-1.5 cursor-pointer shadow-2xs"
            >
              <RefreshCcw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
              刷新
            </Button>
            <span className="text-xs text-muted-foreground">
              已配置 <span className="font-semibold text-foreground">{items.length}</span> 个渠道凭证
            </span>
          </div>

          <div className="flex items-center gap-2">
            <Button
              variant="secondary"
              size="sm"
              className="h-8.5 px-3 text-xs gap-1.5 cursor-pointer shadow-2xs"
              onClick={() => {
                if (isOpenAI && !newForm.name?.trim()) { toast.error('测试前需填提供商名称'); return }
                onProbeForm?.(
                  newForm.apiKey?.trim() || '',
                  newForm.baseUrl?.trim() || '',
                  newForm.name?.trim() || provider,
                  newForm.modelsUrl?.trim() || '',
                  providerKind
                )
              }}
              title="测试当前填写的凭证与模型连通性"
            >
              <Search className="h-3.5 w-3.5" />
              探测模型
            </Button>

            <Button
              onClick={handleAdd}
              size="sm"
              className="h-8.5 px-3.5 text-xs gap-1.5 cursor-pointer shadow-2xs"
            >
              <Plus className="h-3.5 w-3.5" />
              添加 / 更新
            </Button>
          </div>
        </div>
      </div>

      {/* Edit Dialog */}
      <Dialog
        open={editingItem !== null}
        onOpenChange={(open) => {
          if (!open) setEditingItem(null)
        }}
      >
        <DialogContent className="max-w-4xl w-[96vw] max-h-[92vh] overflow-y-auto z-[100] gap-0 p-0 sm:max-w-4xl border-border/80 shadow-xl">
          {editingItem && (
            <>
              <div className="px-6 pt-6 pb-4 border-b border-border/80 bg-muted/10">
                <DialogHeader className="space-y-1.5 text-left">
                  <DialogTitle className="text-lg font-semibold flex items-center gap-2">
                    {isOpenAI ? (
                      <>
                        <div className="p-1 rounded-md bg-muted/60 border border-border/50">
                          <AuthProviderBrandIcon provider={matchOpenAiCompatPreset(editingItem)?.iconProvider} size={18} />
                        </div>
                        <span>编辑 OpenAI 兼容渠道 · {editingItem.name}</span>
                      </>
                    ) : (
                      <span>编辑渠道凭证 · {provider}</span>
                    )}
                  </DialogTitle>
                  <DialogDescription className="text-xs text-muted-foreground">
                    {isOpenAI ? '配置连接端点、从上游探测模型列表，并控制对用户模型页的展示状态。' : '编辑渠道凭证密钥与高级路由选项。'}
                  </DialogDescription>
                </DialogHeader>
              </div>
              <div className="px-6 py-5 min-h-0">
                {isOpenAI ? (
                  <OpenAiEditDialogBody editForm={editForm} updateEditForm={updateEditForm} persistEdit={persistEdit} />
                ) : (
                  <ProviderEditFields providerKind={providerKind} form={editForm} onChange={updateEditForm} />
                )}
              </div>
              <DialogFooter className="gap-2 sm:gap-2 px-6 py-3.5 border-t border-border/80 bg-muted/20">
                <Button type="button" variant="outline" size="sm" onClick={() => setEditingItem(null)}>
                  取消
                </Button>
                <Button type="button" size="sm" onClick={() => void persistEdit(undefined, true)}>
                  保存配置
                </Button>
              </DialogFooter>
            </>
          )}
        </DialogContent>
      </Dialog>

      {/* Channels Table Card */}
      <div className="rounded-xl border border-border/80 bg-card overflow-hidden shadow-2xs">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/30 hover:bg-muted/30">
              {isOpenAI && <TableHead className="w-48 text-xs font-semibold">提供商 / 渠道标识</TableHead>}
              <TableHead className="text-xs font-semibold">网关端点 (Base URL)</TableHead>
              <TableHead className="w-52 text-xs font-semibold">API Key</TableHead>
              <TableHead className="w-32 text-xs font-semibold">路由调度</TableHead>
              <TableHead className="w-36 text-xs font-semibold">近期请求</TableHead>
              <TableHead className="w-32 text-right text-xs font-semibold">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={isOpenAI ? 6 : 5} className="h-28 text-center text-muted-foreground">
                  <div className="flex flex-col items-center justify-center gap-2">
                    <Loader2 className="h-5 w-5 animate-spin text-primary" />
                    <span className="text-xs">加载凭证列表中…</span>
                  </div>
                </TableCell>
              </TableRow>
            ) : items.length === 0 ? (
              <TableRow>
                <TableCell colSpan={isOpenAI ? 6 : 5} className="p-0">
                  <EmptyState
                    size="compact"
                    tone="first-use"
                    icon={KeyRound}
                    title="还没有上游渠道凭证"
                    description="在上方表单中配置 API 密钥或选择内置平台预设，发往该上游的请求将自动完成身份鉴权与多账号故障转移。"
                  />
                </TableCell>
              </TableRow>
            ) : (
              items.map((item) => (
                <TableRow key={item._id} className="hover:bg-muted/40 transition-colors">
                  {isOpenAI && (
                    <TableCell className="font-medium text-sm">
                      <div className="flex items-center gap-2.5">
                        <div className="p-1 rounded-md bg-muted/60 border border-border/50 shrink-0">
                          <AuthProviderBrandIcon provider={matchOpenAiCompatPreset(item)?.iconProvider} size={16} />
                        </div>
                        <div className="flex flex-col min-w-0">
                          <span className="font-semibold text-xs text-foreground truncate" title={item.name}>
                            {item.name}
                          </span>
                          {item.disabled && (
                            <span className="text-[10px] text-rose-500 font-medium">已禁用</span>
                          )}
                        </div>
                      </div>
                    </TableCell>
                  )}
                  <TableCell>
                    <div className="space-y-1">
                      {item.baseUrl ? (
                        <span
                          className="inline-flex items-center gap-1.5 text-xs font-mono text-muted-foreground bg-muted/50 max-w-[320px] px-2 py-0.5 rounded border border-border/40 truncate"
                          title={item.baseUrl}
                        >
                          <Globe className="h-3 w-3 text-muted-foreground/70 shrink-0" />
                          <span className="truncate">{item.baseUrl}</span>
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 text-xs text-muted-foreground bg-muted/30 px-2 py-0.5 rounded">
                          官方默认网关
                        </span>
                      )}
                      {(item.proxyUrl || item.apiKeyProxyUrl) && (
                        <div className="text-[11px] font-mono text-muted-foreground truncate max-w-[320px]" title={item.proxyUrl || item.apiKeyProxyUrl}>
                          proxy: {item.proxyUrl || item.apiKeyProxyUrl}
                        </div>
                      )}
                    </div>
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-1.5">
                      <span className="font-mono text-xs text-foreground/85 select-all" title={item.apiKey}>
                        {maskApiKey(item.apiKey)}
                      </span>
                      {item.apiKey && (
                        <button
                          type="button"
                          onClick={() => handleCopyKey(item.apiKey)}
                          className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
                          title="复制完整 API Key"
                        >
                          {copiedKey === item.apiKey ? (
                            <Check className="h-3.5 w-3.5 text-emerald-500" />
                          ) : (
                            <Copy className="h-3.5 w-3.5" />
                          )}
                        </button>
                      )}
                    </div>
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap items-center gap-1">
                      <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[11px] font-medium bg-primary/10 text-primary border border-primary/20">
                        P{item.priority ?? 0}
                      </span>
                      {item.prefix ? (
                        <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[11px] bg-muted text-muted-foreground truncate max-w-[90px]" title={item.prefix}>
                          {item.prefix}
                        </span>
                      ) : null}
                    </div>
                  </TableCell>
                  <TableCell>
                    {item.usage ? (
                      <span
                        className={cn(
                          "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium border",
                          (item.usage.failed || 0) === 0
                            ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-500/20"
                            : "bg-amber-500/10 text-amber-600 dark:text-amber-400 border-amber-500/20"
                        )}
                      >
                        <span
                          className={cn(
                            "w-1.5 h-1.5 rounded-full",
                            (item.usage.failed || 0) === 0 ? "bg-emerald-500" : "bg-amber-500"
                          )}
                        />
                        {item.usage.success || 0} / {item.usage.failed || 0}
                      </span>
                    ) : (
                      <span className="text-xs text-muted-foreground/70">无调用记录</span>
                    )}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end items-center gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8 text-blue-600 hover:text-blue-700 hover:bg-blue-50 dark:hover:bg-blue-950/40"
                        onClick={() => handleOpenModelsDialog(item)}
                        title="探测可用模型"
                      >
                        <Search className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8 hover:bg-muted"
                        onClick={() => startEdit(item)}
                        title="编辑配置"
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        type="button"
                        variant="dangerIcon"
                        className="h-8 w-8"
                        onClick={() => handleDelete(item)}
                        title="删除凭证"
                        aria-label="删除凭证"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  )
})

function maskApiKey(key?: string) {
  if (!key) return '<空>'
  if (key.length <= 8) return key
  return `${key.slice(0, 4)}••••${key.slice(-4)}`
}

function ProviderEditFields({ providerKind, form, onChange }: { providerKind: ProviderKind; form: ProviderStructuredForm; onChange: (patch: ProviderStructuredForm) => void }) {
  const isOpenAI = providerKind === 'openai'
  return (
    <div className="space-y-4">
      <div className={cn("grid gap-4", isOpenAI ? "md:grid-cols-3" : "md:grid-cols-2")}>
        {isOpenAI && (
          <Field label="提供商名称" required>
            <Input value={form.name || ''} onChange={(e) => onChange({ name: e.target.value })} />
          </Field>
        )}
        <Field label="API Key / 凭证密钥" required>
          <Input className="font-mono text-xs" value={form.apiKey || ''} onChange={(e) => onChange({ apiKey: e.target.value })} />
        </Field>
        <Field label="网关端点 (Base URL)">
          <Input className="font-mono text-xs" value={form.baseUrl || ''} onChange={(e) => onChange({ baseUrl: e.target.value })} />
        </Field>
      </div>
      <div className="grid gap-4 md:grid-cols-4">
        <Field label={isOpenAI ? 'Key Proxy URL' : 'Proxy URL'} optional>
          <Input className="font-mono text-xs" value={(isOpenAI ? form.apiKeyProxyUrl : form.proxyUrl) || ''} onChange={(e) => onChange(isOpenAI ? { apiKeyProxyUrl: e.target.value } : { proxyUrl: e.target.value })} />
        </Field>
        <Field label="模型前缀" optional>
          <Input className="font-mono text-xs" value={form.prefix || ''} onChange={(e) => onChange({ prefix: e.target.value })} />
        </Field>
        <Field label="调度优先级" optional>
          <Input type="number" value={form.priority || ''} onChange={(e) => onChange({ priority: e.target.value })} />
        </Field>
        <div className="flex items-end gap-4 pb-1">
          {isOpenAI && <SwitchField label="禁用渠道" checked={!!form.disabled} onCheckedChange={(v) => onChange({ disabled: v })} />}
          {providerKind === 'codex' && <SwitchField label="WebSocket" checked={!!form.websockets} onCheckedChange={(v) => onChange({ websockets: v })} />}
          {providerKind === 'claude' && <SwitchField label="CCH Signing" checked={!!form.experimentalCCHSigning} onCheckedChange={(v) => onChange({ experimentalCCHSigning: v })} />}
        </div>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        <Field label="Headers JSON" optional>
          <Textarea className="font-mono text-xs h-24" value={form.headersText || ''} onChange={(e) => onChange({ headersText: e.target.value })} />
        </Field>
        <Field label="Models JSON" optional>
          <Textarea className="font-mono text-xs h-24" value={form.modelsText || ''} onChange={(e) => onChange({ modelsText: e.target.value })} />
        </Field>
        {!isOpenAI && (
          <Field label="Excluded Models" optional>
            <Textarea className="font-mono text-xs h-20" value={form.excludedModelsText || ''} onChange={(e) => onChange({ excludedModelsText: e.target.value })} />
          </Field>
        )}
        <Field label={isOpenAI ? '高级 JSON (Provider)' : '高级 JSON (Credential)'} optional>
          <Textarea className="font-mono text-xs h-32" value={form.advancedText || ''} onChange={(e) => onChange({ advancedText: e.target.value })} />
        </Field>
      </div>
    </div>
  )
}

function Field({
  label,
  children,
  required,
  optional,
}: {
  label: string
  children: ReactNode
  required?: boolean
  optional?: boolean
}) {
  return (
    <div className="space-y-1.5 min-w-0">
      <div className="flex items-center justify-between">
        <label className="text-xs font-semibold text-foreground/85 ml-0.5">{label}</label>
        {required && <span className="text-[10px] text-primary font-medium">必填</span>}
        {optional && <span className="text-[10px] text-muted-foreground">可选</span>}
      </div>
      {children}
    </div>
  )
}

function SwitchField({ label, checked, onCheckedChange }: { label: string; checked: boolean; onCheckedChange: (v: boolean) => void }) {
  return (
    <div className="flex items-center gap-2 pb-2">
      <Switch checked={checked} onCheckedChange={onCheckedChange} />
      <span className="text-xs text-muted-foreground">{label}</span>
    </div>
  )
}

function formFromItem(item: BaseChannelItem): ProviderStructuredForm {
  return {
    name: item.name || '',
    apiKey: item.apiKey || '',
    baseUrl: item.baseUrl || '',
    modelsUrl: item.modelsUrl || '',
    proxyUrl: item.proxyUrl || '',
    apiKeyProxyUrl: item.apiKeyProxyUrl || '',
    priority: typeof item.priority === 'number' ? String(item.priority) : '',
    prefix: item.prefix || '',
    headersText: item.headers ? JSON.stringify(item.headers, null, 2) : '',
    modelsText: item.models ? JSON.stringify(item.models, null, 2) : '',
    excludedModelsText: item.excludedModels?.join('\n') || '',
    disabled: item.disabled || false,
    websockets: item.websockets || false,
    experimentalCCHSigning: item.experimentalCCHSigning || false,
    advancedText: JSON.stringify(item.originalPayload, null, 2),
  }
}
