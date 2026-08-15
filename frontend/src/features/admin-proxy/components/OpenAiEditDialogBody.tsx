import { useCallback, useEffect, useState, type ReactNode } from 'react'
import { toast } from 'sonner'
import { Loader2, Layers, ChevronDown, ChevronRight } from 'lucide-react'
import { Button } from '@/shared/components/ui/button'
import { Input } from '@/shared/components/ui/input'
import { Textarea } from '@/shared/components/ui/textarea'
import { Switch } from '@/shared/components/ui/switch'
import { fetchApi } from '@/shared/api/client'
import { modelsApi, type ModelInfo } from '@/features/pricing/model_prices'
import type { ProviderStructuredForm } from '../providerConfig'
import { cn } from '@/shared/utils/utils'

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="space-y-1.5 min-w-0">
      <label className="text-xs font-semibold text-muted-foreground ml-0.5">{label}</label>
      {children}
    </div>
  )
}

type Props = {
  editForm: ProviderStructuredForm
  updateEditForm: (patch: ProviderStructuredForm) => void
  /** 写入 SDK；`closeDialog=false` 时保存后仍留在弹窗内（用于先写入模型列表再调展示开关） */
  persistEdit: (formOverride?: ProviderStructuredForm, closeDialog?: boolean) => Promise<void>
}

export function OpenAiEditDialogBody({ editForm, updateEditForm, persistEdit }: Props) {
  const [probeModels, setProbeModels] = useState<ModelInfo[]>([])
  const [probeLoading, setProbeLoading] = useState(false)
  const [catalogVisible, setCatalogVisible] = useState<Record<string, boolean>>({})
  const [catLoading, setCatLoading] = useState(false)
  const [bulkVisibilityLoading, setBulkVisibilityLoading] = useState(false)
  const [showAdvanced, setShowAdvanced] = useState(false)

  const channelKey = (editForm.name || '').trim()

  const loadCatalog = useCallback(async () => {
    if (!channelKey) {
      setCatalogVisible({})
      return
    }
    setCatLoading(true)
    try {
      const pageSize = 100
      const rows: Array<{ model_id: string; visible: boolean }> = []
      for (let page = 1; page <= 50; page++) {
        const params = new URLSearchParams({
          channel_key: channelKey,
          page: String(page),
          page_size: String(pageSize),
        })
        const res = (await fetchApi(`/admin/model-catalog/entries?${params.toString()}`)) as {
          data?: { items?: Array<{ model_id: string; visible: boolean }>; total?: number }
        }
        const d = res?.data
        const batch = Array.isArray(d?.items) ? d.items : []
        rows.push(...batch)
        if (batch.length < pageSize) break
        if (typeof d?.total === 'number' && rows.length >= d.total) break
      }
      const next: Record<string, boolean> = {}
      for (const r of rows) {
        if (r.model_id) next[r.model_id] = !!r.visible
      }
      setCatalogVisible(next)
    } catch {
      setCatalogVisible({})
    } finally {
      setCatLoading(false)
    }
  }, [channelKey])

  useEffect(() => {
    void loadCatalog()
  }, [loadCatalog])

  const handleFetchUpstream = async () => {
    const base = (editForm.baseUrl || '').trim()
    const key = (editForm.apiKey || '').trim()
    if (!base || !key) {
      toast.error('请先填写 Base URL 与 API Key')
      return
    }
    setProbeLoading(true)
    setProbeModels([])
    try {
      const list = await modelsApi.fetchOpenAIModelsViaApiCall(base, key, {}, editForm.modelsUrl)
      setProbeModels(list)
      if (list.length === 0) {
        toast.success('上游未返回模型')
        return
      }

      const modelsJson = JSON.stringify(
        list.map((m) => {
          const name = String(m.name || '').trim()
          const alias = String(m.alias || '').trim()
          return alias && alias !== name ? { name, alias } : { name }
        }),
        null,
        2
      )
      const nextForm: ProviderStructuredForm = { ...editForm, modelsText: modelsJson }
      updateEditForm({ modelsText: modelsJson })

      if (!channelKey) {
        toast.success(`已获取 ${list.length} 个模型`)
        toast.message('请填写「提供商名称（渠道标识）」后，点击底部「保存配置」将列表写入网关。')
        return
      }

      await persistEdit(nextForm, false)
      await fetchApi('/admin/model-catalog/ensure-openai-channel', {
        method: 'POST',
        body: JSON.stringify({
          channel_key: channelKey,
          model_ids: list.map((m) => String(m.name || '').trim()).filter(Boolean),
        }),
      })
      await loadCatalog()
      toast.success(`已获取并同步 ${list.length} 个模型，可在下方设置「对用户展示」`)
    } catch (e: unknown) {
      toast.error(e instanceof Error ? e.message : '获取或同步失败')
    } finally {
      setProbeLoading(false)
    }
  }

  const toggleUserVisible = async (modelId: string, v: boolean) => {
    if (!channelKey) {
      toast.error('请先填写提供商名称')
      return
    }
    try {
      await fetchApi('/admin/model-catalog/openai-visibility', {
        method: 'POST',
        body: JSON.stringify({ channel_key: channelKey, model_id: modelId, visible: v }),
      })
      setCatalogVisible((s) => ({ ...s, [modelId]: v }))
    } catch (e: unknown) {
      toast.error(e instanceof Error ? e.message : '更新展示状态失败')
      await loadCatalog()
    }
  }

  const modelIdsForSwitches = probeModels.length > 0
    ? probeModels.map((m) => String(m.name || '').trim()).filter(Boolean)
    : (() => {
        try {
          const raw = JSON.parse(editForm.modelsText || '[]') as unknown
          if (!Array.isArray(raw)) return []
          const ids: string[] = []
          for (const m of raw) {
            if (typeof m === 'string' && m.trim()) ids.push(m.trim())
            else if (m && typeof m === 'object' && 'name' in m) ids.push(String((m as { name: string }).name).trim())
          }
          return [...new Set(ids.filter(Boolean))]
        } catch {
          return []
        }
      })()
  const visibleCount = modelIdsForSwitches.filter((mid) => !!catalogVisible[mid]).length
  const allVisible = modelIdsForSwitches.length > 0 && visibleCount === modelIdsForSwitches.length
  const allHidden = modelIdsForSwitches.length > 0 && visibleCount === 0

  const setAllUserVisible = async (visible: boolean) => {
    if (!channelKey) {
      toast.error('请先填写提供商名称')
      return
    }
    if (modelIdsForSwitches.length === 0) return

    setBulkVisibilityLoading(true)
    try {
      await fetchApi('/admin/model-catalog/ensure-openai-channel', {
        method: 'POST',
        body: JSON.stringify({ channel_key: channelKey, model_ids: modelIdsForSwitches }),
      })
      await Promise.all(
        modelIdsForSwitches.map((modelId) =>
          fetchApi('/admin/model-catalog/openai-visibility', {
            method: 'POST',
            body: JSON.stringify({ channel_key: channelKey, model_id: modelId, visible }),
          })
        )
      )
      setCatalogVisible((current) => {
        const next = { ...current }
        for (const modelId of modelIdsForSwitches) {
          next[modelId] = visible
        }
        return next
      })
      toast.success(visible ? `已全选展示 ${modelIdsForSwitches.length} 个模型` : `已全部隐藏 ${modelIdsForSwitches.length} 个模型`)
    } catch (e: unknown) {
      toast.error(e instanceof Error ? e.message : '批量更新展示状态失败')
      await loadCatalog()
    } finally {
      setBulkVisibilityLoading(false)
    }
  }

  return (
    <div className="space-y-5 min-w-0">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <Field label="提供商名称（渠道标识）">
          <Input
            className="font-medium"
            value={editForm.name || ''}
            onChange={(e) => updateEditForm({ name: e.target.value })}
            placeholder="如 MINIMAX"
          />
        </Field>
        <Field label="Base URL">
          <Input
            className="font-mono text-xs"
            value={editForm.baseUrl || ''}
            onChange={(e) => updateEditForm({ baseUrl: e.target.value })}
            placeholder="https://..."
          />
        </Field>
        <Field label="Key Proxy URL">
          <Input
            value={editForm.apiKeyProxyUrl || ''}
            onChange={(e) => updateEditForm({ apiKeyProxyUrl: e.target.value })}
            placeholder="可选"
          />
        </Field>
      </div>

      <Field label="API Key">
        <Textarea
          className="font-mono text-xs min-h-[72px] max-h-40 resize-y break-all"
          value={editForm.apiKey || ''}
          onChange={(e) => updateEditForm({ apiKey: e.target.value })}
          placeholder="sk-..."
          spellCheck={false}
        />
      </Field>

      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="模型前缀">
          <Input value={editForm.prefix || ''} onChange={(e) => updateEditForm({ prefix: e.target.value })} />
        </Field>
        <Field label="优先级">
          <Input type="number" value={editForm.priority || ''} onChange={(e) => updateEditForm({ priority: e.target.value })} />
        </Field>
      </div>

      <div className="rounded-xl border border-border bg-muted/20 p-4 space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <Layers className="h-4 w-4 text-violet-600" />
          <span className="text-sm font-medium">上游模型与用户模型页</span>
        </div>
        <p className="text-xs text-muted-foreground leading-relaxed">
          点击「从上游获取模型列表」后会自动把列表写入网关配置（已填写渠道标识时）；随后在下方打开「对用户展示」的模型会出现在用户「模型」页（还须出现在网关
          GET /v1/models 且未被计费拉黑）。
        </p>
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="secondary" size="sm" disabled={probeLoading} onClick={() => void handleFetchUpstream()}>
            {probeLoading ? <Loader2 className="h-4 w-4 animate-spin mr-2" /> : null}
            从上游获取模型列表
          </Button>
          {modelIdsForSwitches.length > 0 && (
            <>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={!channelKey || catLoading || bulkVisibilityLoading || allVisible}
                onClick={() => void setAllUserVisible(true)}
              >
                {bulkVisibilityLoading ? <Loader2 className="h-4 w-4 animate-spin mr-2" /> : null}
                全部展示
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={!channelKey || catLoading || bulkVisibilityLoading || allHidden}
                onClick={() => void setAllUserVisible(false)}
              >
                全部隐藏
              </Button>
              <span className="inline-flex items-center text-xs text-muted-foreground">
                已展示 {visibleCount}/{modelIdsForSwitches.length}
              </span>
            </>
          )}
        </div>

        {modelIdsForSwitches.length === 0 ? (
          <p className="text-xs text-muted-foreground">获取模型或配置 Models 后，将在此显示开关。</p>
        ) : (
          <div className="max-h-56 overflow-y-auto rounded-lg border border-border bg-background p-2 space-y-1.5">
            {(catLoading || bulkVisibilityLoading) && (
              <p className="text-xs text-muted-foreground px-1">
                {bulkVisibilityLoading ? '批量更新展示状态中…' : '同步展示状态中…'}
              </p>
            )}
            {modelIdsForSwitches.map((mid) => (
              <label
                key={mid}
                className="flex items-center justify-between gap-3 rounded-md px-2 py-1.5 hover:bg-muted/50 text-xs"
              >
                <span className="font-mono break-all min-w-0 flex-1" title={mid}>
                  {mid}
                </span>
                <span className="flex items-center gap-1.5 shrink-0 text-muted-foreground">
                  对用户展示
                  <Switch
                    checked={!!catalogVisible[mid]}
                    disabled={!channelKey || bulkVisibilityLoading}
                    onCheckedChange={(v) => void toggleUserVisible(mid, v)}
                  />
                </span>
              </label>
            ))}
          </div>
        )}
      </div>

      <button
        type="button"
        className="flex w-full items-center justify-between rounded-lg border border-dashed px-3 py-2 text-xs font-medium text-muted-foreground hover:bg-muted/40"
        onClick={() => setShowAdvanced((v) => !v)}
      >
        <span>高级选项（Headers / Models JSON / 原始 Provider JSON）</span>
        {showAdvanced ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
      </button>

      {showAdvanced && (
        <div className={cn('space-y-3 rounded-lg border p-3 bg-muted/10')}>
          <Field label="Headers JSON">
            <Textarea
              className="font-mono text-xs min-h-[88px] break-all"
              value={editForm.headersText || ''}
              onChange={(e) => updateEditForm({ headersText: e.target.value })}
            />
          </Field>
          <Field label="Models JSON（获取上游列表时会自动更新，也可手改）">
            <Textarea
              className="font-mono text-xs min-h-[100px] break-all"
              value={editForm.modelsText || ''}
              onChange={(e) => updateEditForm({ modelsText: e.target.value })}
            />
          </Field>
          <Field label="高级 JSON（Provider 原始对象）">
            <Textarea
              className="font-mono text-xs min-h-[120px] break-all"
              value={editForm.advancedText || ''}
              onChange={(e) => updateEditForm({ advancedText: e.target.value })}
            />
          </Field>
        </div>
      )}
    </div>
  )
}
