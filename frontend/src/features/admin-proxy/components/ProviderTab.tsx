import { useEffect, useRef, useState, useCallback } from 'react'
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
import { Loader2, RefreshCcw, Plus, Trash2, Globe, Search, Pencil, Activity } from 'lucide-react'
import { toast } from 'sonner'
import { fetchProviderConfig, updateProviderConfig, fetchApiKeyUsage } from '../api'
import { Input } from '@/shared/components/ui/input'
import { Textarea } from '@/shared/components/ui/textarea'
import { Switch } from '@/shared/components/ui/switch'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/components/ui/table'
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
import { OpenAiEditDialogBody } from './OpenAiEditDialogBody'

export type { BaseChannelItem } from '../providerConfig'

interface ProviderTabProps {
  providerKind: ProviderKind
  endpoint: string
  refreshSignal?: number
  onOpenModelsDialog?: (item: BaseChannelItem) => void
  onProbeForm?: (apiKey: string, baseUrl: string, name: string, modelsUrl?: string) => void
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

export function ProviderTab({ providerKind, endpoint, refreshSignal, onOpenModelsDialog, onProbeForm }: ProviderTabProps) {
  const [items, setItems] = useState<BaseChannelItem[]>([])
  const [loading, setLoading] = useState(true)
  const [newForm, setNewForm] = useState<ProviderStructuredForm>(emptyForm)
  const [editingItem, setEditingItem] = useState<BaseChannelItem | null>(null)
  const [editForm, setEditForm] = useState<ProviderStructuredForm>(emptyForm)
  const mountedRef = useRef(false)

  const provider = providerLabel(providerKind)
  const isOpenAI = providerKind === 'openai'

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
    <div className="space-y-6 mt-4">
      <div className="bg-gray-50/50 dark:bg-dark-800/30 border border-gray-100 dark:border-dark-700 p-4 pt-5 rounded-lg flex flex-col gap-5">
        <div className={cn("grid gap-4", isOpenAI ? "grid-cols-1 md:grid-cols-3" : "grid-cols-1 md:grid-cols-2")}>
          {isOpenAI && (
            <Field label="提供商名称">
              <Input className="bg-white dark:bg-dark-800 shadow-sm" placeholder="如: openrouter" value={newForm.name || ''} onChange={(e) => updateNewForm({ name: e.target.value })} />
            </Field>
          )}
          <Field label="Base URL">
            <Input className="bg-white dark:bg-dark-800 shadow-sm" placeholder="https://..." value={newForm.baseUrl || ''} onChange={(e) => updateNewForm({ baseUrl: e.target.value })} />
          </Field>
          <Field label="API Key">
            <Input
              className="bg-white dark:bg-dark-800 shadow-sm"
              placeholder="输入凭证..."
              value={newForm.apiKey || ''}
              onChange={(e) => updateNewForm({ apiKey: e.target.value })}
              onKeyDown={(e) => e.key === 'Enter' && handleAdd()}
            />
          </Field>
        </div>

        <div className="grid gap-4 md:grid-cols-4">
          <Field label={isOpenAI ? 'Key Proxy URL' : 'Proxy URL'}>
            <Input value={(isOpenAI ? newForm.apiKeyProxyUrl : newForm.proxyUrl) || ''} onChange={(e) => updateNewForm(isOpenAI ? { apiKeyProxyUrl: e.target.value } : { proxyUrl: e.target.value })} placeholder="direct / http://..." />
          </Field>
          <Field label="模型前缀">
            <Input value={newForm.prefix || ''} onChange={(e) => updateNewForm({ prefix: e.target.value })} placeholder="teamA" />
          </Field>
          <Field label="优先级">
            <Input type="number" value={newForm.priority || ''} onChange={(e) => updateNewForm({ priority: e.target.value })} placeholder="0" />
          </Field>
          <Field label="Models JSON">
            <Input value={newForm.modelsText || ''} onChange={(e) => updateNewForm({ modelsText: e.target.value })} placeholder='[{"name":"...","alias":"..."}]' />
          </Field>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => fetchItems(false)} disabled={loading} className="mr-auto shadow-sm h-9">
            <RefreshCcw className={cn("h-4 w-4 mr-2", loading && "animate-spin")} />
            刷新
          </Button>

          <Button variant="secondary" size="sm" className="shadow-sm h-9" onClick={() => {
            if (isOpenAI && !newForm.name?.trim()) { toast.error('测试前需填提供商名称'); return }
            onProbeForm?.(
              newForm.apiKey?.trim() || '',
              newForm.baseUrl?.trim() || '',
              newForm.name?.trim() || provider,
              newForm.modelsUrl?.trim() || ''
            )
          }} title="测试当前填写的凭证">
            <Search className="h-4 w-4 mr-1.5" />探测模型
          </Button>

          <Button onClick={handleAdd} size="sm" className="shadow-sm bg-indigo-600 hover:bg-indigo-700 text-white h-9">
            <Plus className="h-4 w-4 mr-1.5"/>添加 / 更新
          </Button>
        </div>
      </div>

      <Dialog
        open={editingItem !== null}
        onOpenChange={(open) => {
          if (!open) setEditingItem(null)
        }}
      >
        <DialogContent className="max-w-4xl w-[96vw] max-h-[92vh] overflow-y-auto z-[100] gap-0 p-0 sm:max-w-4xl">
          {editingItem && (
            <>
              <div className="px-6 pt-6 pb-4 border-b">
                <DialogHeader className="space-y-2 text-left">
                  <DialogTitle>{isOpenAI ? '编辑 OpenAI 兼容渠道' : '编辑凭证'}</DialogTitle>
                  <DialogDescription className="sr-only">
                    {isOpenAI ? '配置连接、从上游同步模型列表，并控制对用户模型页的展示。' : '编辑渠道凭证与高级选项。'}
                  </DialogDescription>
                </DialogHeader>
              </div>
              <div className="px-6 py-4 min-h-0">
                {isOpenAI ? (
                  <OpenAiEditDialogBody editForm={editForm} updateEditForm={updateEditForm} persistEdit={persistEdit} />
                ) : (
                  <ProviderEditFields providerKind={providerKind} form={editForm} onChange={updateEditForm} />
                )}
              </div>
              <DialogFooter className="gap-2 sm:gap-0 px-6 py-4 border-t bg-muted/20">
                <Button type="button" variant="outline" onClick={() => setEditingItem(null)}>
                  取消
                </Button>
                <Button type="button" onClick={() => void persistEdit(undefined, true)}>
                  保存配置
                </Button>
              </DialogFooter>
            </>
          )}
        </DialogContent>
      </Dialog>

      <div className="border rounded-md overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow>
              {isOpenAI && <TableHead className="w-40">提供商</TableHead>}
              <TableHead>Base URL / 代理网关地址</TableHead>
              <TableHead>凭证内容</TableHead>
              <TableHead className="w-32">路由</TableHead>
              <TableHead className="w-32">近期请求</TableHead>
              <TableHead className="w-36 text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={isOpenAI ? 6 : 5} className="h-24 text-center text-muted-foreground">
                  <Loader2 className="mx-auto h-5 w-5 animate-spin" />
                </TableCell>
              </TableRow>
            ) : items.length === 0 ? (
              <TableRow>
                <TableCell colSpan={isOpenAI ? 6 : 5} className="h-24 text-center text-muted-foreground">暂无凭证</TableCell>
              </TableRow>
            ) : (
              items.map((item) => (
                <TableRow key={item._id}>
                  {isOpenAI && <TableCell className="font-medium text-sm">{item.name}</TableCell>}
                  <TableCell>
                    <div className="space-y-1">
                      {item.baseUrl ? (
                        <span className="flex items-center gap-1.5 text-xs text-muted-foreground bg-muted/50 w-fit max-w-[340px] px-2 py-1 rounded truncate" title={item.baseUrl}>
                          <Globe className="h-3 w-3 flex-shrink-0"/> {item.baseUrl}
                        </span>
                      ) : (
                        <span className="text-xs text-muted-foreground">官方默认网关</span>
                      )}
                      {(item.proxyUrl || item.apiKeyProxyUrl) && (
                        <span className="block text-[11px] text-muted-foreground truncate" title={item.proxyUrl || item.apiKeyProxyUrl}>proxy: {item.proxyUrl || item.apiKeyProxyUrl}</span>
                      )}
                    </div>
                  </TableCell>
                  <TableCell className="font-mono text-xs truncate max-w-[220px]" title={item.apiKey}>{item.apiKey || '<空>'}</TableCell>
                  <TableCell>
                    <div className="space-y-1 text-xs text-muted-foreground">
                      <div>priority: {item.priority ?? 0}</div>
                      <div className="truncate" title={item.prefix}>prefix: {item.prefix || '无'}</div>
                    </div>
                  </TableCell>
                  <TableCell>
                    {item.usage ? (
                      <span className="inline-flex items-center gap-1.5 rounded px-2 py-1 text-xs bg-muted/50 text-muted-foreground">
                        <Activity className="h-3 w-3" />
                        {item.usage.success || 0}/{item.usage.failed || 0}
                      </span>
                    ) : (
                      <span className="text-xs text-muted-foreground">无数据</span>
                    )}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-1">
                      <Button variant="ghost" size="icon" className="text-blue-500 hover:text-blue-600 hover:bg-blue-50" onClick={() => handleOpenModelsDialog(item)} title="探测可用模型">
                        <Search className="h-4 w-4" />
                      </Button>
                      <Button variant="ghost" size="icon" onClick={() => startEdit(item)} title="编辑配置">
                        <Pencil className="h-4 w-4" />
                      </Button>
                      <Button type="button" variant="dangerIcon" onClick={() => handleDelete(item)} title="删除凭证" aria-label="删除凭证">
                        <Trash2 />
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
}

function ProviderEditFields({ providerKind, form, onChange }: { providerKind: ProviderKind; form: ProviderStructuredForm; onChange: (patch: ProviderStructuredForm) => void }) {
  const isOpenAI = providerKind === 'openai'
  return (
    <div className="space-y-4">
      <div className={cn("grid gap-4", isOpenAI ? "md:grid-cols-3" : "md:grid-cols-2")}>
        {isOpenAI && (
          <Field label="提供商名称">
            <Input value={form.name || ''} onChange={(e) => onChange({ name: e.target.value })} />
          </Field>
        )}
        <Field label="API Key">
          <Input value={form.apiKey || ''} onChange={(e) => onChange({ apiKey: e.target.value })} />
        </Field>
        <Field label="Base URL">
          <Input value={form.baseUrl || ''} onChange={(e) => onChange({ baseUrl: e.target.value })} />
        </Field>
      </div>
      <div className="grid gap-4 md:grid-cols-4">
        <Field label={isOpenAI ? 'Key Proxy URL' : 'Proxy URL'}>
          <Input value={(isOpenAI ? form.apiKeyProxyUrl : form.proxyUrl) || ''} onChange={(e) => onChange(isOpenAI ? { apiKeyProxyUrl: e.target.value } : { proxyUrl: e.target.value })} />
        </Field>
        <Field label="模型前缀">
          <Input value={form.prefix || ''} onChange={(e) => onChange({ prefix: e.target.value })} />
        </Field>
        <Field label="优先级">
          <Input type="number" value={form.priority || ''} onChange={(e) => onChange({ priority: e.target.value })} />
        </Field>
        <div className="flex items-end gap-4">
          {isOpenAI && <SwitchField label="禁用" checked={!!form.disabled} onCheckedChange={(v) => onChange({ disabled: v })} />}
          {providerKind === 'codex' && <SwitchField label="WebSocket" checked={!!form.websockets} onCheckedChange={(v) => onChange({ websockets: v })} />}
          {providerKind === 'claude' && <SwitchField label="CCH Signing" checked={!!form.experimentalCCHSigning} onCheckedChange={(v) => onChange({ experimentalCCHSigning: v })} />}
        </div>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        <Field label="Headers JSON">
          <Textarea className="font-mono text-xs h-24" value={form.headersText || ''} onChange={(e) => onChange({ headersText: e.target.value })} />
        </Field>
        <Field label="Models JSON">
          <Textarea className="font-mono text-xs h-24" value={form.modelsText || ''} onChange={(e) => onChange({ modelsText: e.target.value })} />
        </Field>
        {!isOpenAI && (
          <Field label="Excluded Models">
            <Textarea className="font-mono text-xs h-20" value={form.excludedModelsText || ''} onChange={(e) => onChange({ excludedModelsText: e.target.value })} />
          </Field>
        )}
        <Field label={isOpenAI ? '高级 JSON (Provider)' : '高级 JSON (Credential)'}>
          <Textarea className="font-mono text-xs h-32" value={form.advancedText || ''} onChange={(e) => onChange({ advancedText: e.target.value })} />
        </Field>
      </div>
    </div>
  )
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="space-y-1.5">
      <label className="text-xs font-semibold text-gray-500 dark:text-dark-300 ml-1">{label}</label>
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
