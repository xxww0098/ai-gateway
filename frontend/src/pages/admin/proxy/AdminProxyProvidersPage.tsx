import { useState, useCallback } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card'
import { ProviderTab, type BaseChannelItem } from '@/features/admin-proxy/components/ProviderTab'
import { ProviderModelsDialog } from '@/features/admin-proxy/components/ProviderModelsDialog'
import { cn } from '@/shared/utils/utils'
import { fetchProviderConfig, updateProviderConfig } from '@/features/admin-proxy/api'
import { apiClient } from '@/shared/api/client'
import {
  buildProviderModelsArray,
  normalizeProviderItems,
  PROVIDER_ENDPOINTS,
  providerLabel,
  type ProviderKind,
} from '@/features/admin-proxy/providerConfig'
import { AuthProviderBrandIcon } from '@/features/admin-proxy/components/AuthProviderBrandIcon'
import type { ModelInfo } from '@/features/pricing/model_prices'

function recordString(value: unknown, key: string): string {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return ''
  const item = (value as Record<string, unknown>)[key]
  return typeof item === 'string' ? item : ''
}

function itemModelsUrl(item: BaseChannelItem | null): string {
  if (!item) return ''
  return item.modelsUrl ||
    recordString(item.originalPayload, 'models-url') ||
    recordString(item.originalPayload, 'model-url') ||
    recordString(item.originalPayload, 'models_url')
}

async function fetchPersistedModelsUrl(channelKey: string): Promise<string> {
  const key = channelKey.trim()
  if (!key) return ''
  const params = new URLSearchParams({ channel_key: key })
  const response = await apiClient.get<{
    models_url?: string
  }>(`/admin/model-catalog/models-url?${params.toString()}`)
  return String(response?.models_url || '').trim()
}

export default function AdminProxyProvidersPage() {
  const [activeTab, setActiveTab] = useState<ProviderKind>('openai')
  const [visitedTabs, setVisitedTabs] = useState<Set<ProviderKind>>(() => new Set<ProviderKind>(['openai']))
  const [modelsDialogItem, setModelsDialogItem] = useState<BaseChannelItem | null>(null)
  const [providerRefreshSignal, setProviderRefreshSignal] = useState(0)

  const providerTabs: Array<{ id: ProviderKind; label: string }> = [
    { id: 'openai', label: 'OpenAI (兼容)' },
    { id: 'claude', label: 'Claude' },
    { id: 'gemini', label: 'Gemini' },
    { id: 'codex', label: 'Codex' },
    { id: 'vertex', label: 'Vertex' },
  ];

  const activeIndex = providerTabs.findIndex(t => t.id === activeTab)

  const handleTabChange = useCallback((tabId: ProviderKind) => {
    setActiveTab(tabId)
    setVisitedTabs((prev) => {
      if (prev.has(tabId)) return prev
      const next = new Set(prev)
      next.add(tabId)
      return next
    })
  }, [])

  const handleOpenModelsDialog = useCallback(async (item: BaseChannelItem) => {
    let nextItem = item
    try {
      const latest = await fetchProviderConfig(PROVIDER_ENDPOINTS[item.providerKind])
      const fresh = normalizeProviderItems(item.providerKind, latest).find((candidate) =>
        candidate.index === item.index &&
        candidate.keyIndex === item.keyIndex &&
        candidate.providerKind === item.providerKind
      )
      if (fresh) {
        nextItem = fresh
      }
    } catch {
      // Keep the current row data if the refresh fails; probing will still report the exact attempted URL.
    }

    if (nextItem.providerKind === 'openai' && nextItem.index >= 0) {
      try {
        nextItem = {
          ...nextItem,
          modelsUrl: await fetchPersistedModelsUrl(nextItem.name || ''),
        }
      } catch {
        nextItem = {
          ...nextItem,
          modelsUrl: itemModelsUrl(nextItem),
        }
      }
    }

    setModelsDialogItem(nextItem)
  }, [])

  const handleProbeForm = useCallback((apiKey: string, baseUrl: string, name: string, modelsUrl?: string, providerKind?: ProviderKind) => {
    setModelsDialogItem({
      _id: 'test',
      providerKind: providerKind || activeTab,
      index: -1,
      apiKey,
      baseUrl,
      modelsUrl,
      originalPayload: {},
      name,
    })
  }, [activeTab])

  const handleSaveConfiguredModels = useCallback(async (models: ModelInfo[]) => {
    if (!modelsDialogItem || modelsDialogItem.index < 0) {
      throw new Error('请先保存供应商后再写入模型配置')
    }

    const providerKind = modelsDialogItem.providerKind
    const endpoint = PROVIDER_ENDPOINTS[providerKind]
    const latest = await fetchProviderConfig(endpoint)
    const updatedArray = buildProviderModelsArray(providerKind, latest, modelsDialogItem, models)
    await updateProviderConfig(endpoint, updatedArray)

    const nextModels = models
      .map((model) => {
        const name = String(model.name || '').trim()
        if (!name) return null
        const alias = String(model.alias || '').trim()
        return alias && alias !== name ? { name, alias } : { name }
      })
      .filter(Boolean)

    setModelsDialogItem((current) => current ? {
      ...current,
      originalPayload: {
        ...current.originalPayload,
        models: nextModels,
      },
      models: nextModels,
    } : current)
    setProviderRefreshSignal((value) => value + 1)

    if (providerKind === 'openai' && modelsDialogItem?.name && nextModels.length > 0) {
      const modelIds = nextModels.map((m) => (m && typeof m === 'object' && 'name' in m ? String((m as { name: string }).name).trim() : '')).filter(Boolean)
      if (modelIds.length > 0) {
        try {
          await apiClient.post('/admin/model-catalog/ensure-openai-channel', { channel_key: String(modelsDialogItem.name).trim(), model_ids: modelIds })
        } catch {
          /* 登记失败不影响渠道保存；可在渠道列表中开关区域重试 */
        }
      }
    }
  }, [modelsDialogItem])

  const handleSaveModelsUrl = useCallback(async (modelsUrl: string) => {
    if (!modelsDialogItem || modelsDialogItem.index < 0) {
      throw new Error('请先保存供应商后再保存模型列表 URL')
    }
    if (modelsDialogItem.providerKind !== 'openai') {
      throw new Error('模型列表 URL 仅适用于 OpenAI 兼容渠道')
    }

    const channelKey = String(modelsDialogItem.name || '').trim()
    if (!channelKey) {
      throw new Error('渠道名称不能为空')
    }
    await apiClient.put('/admin/model-catalog/models-url', { channel_key: channelKey, models_url: modelsUrl })

    setModelsDialogItem((current) => current ? {
      ...current,
      modelsUrl,
    } : current)
  }, [modelsDialogItem])

  const dialogProviderKind = modelsDialogItem?.providerKind ?? activeTab

  return (
    <div className="space-y-6 max-w-6xl">
      <Card className="border-border/80 shadow-xs">
        <CardHeader className="pb-4">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2">
            <div>
              <CardTitle className="text-base sm:text-lg font-semibold flex items-center gap-2">
                <span>多渠道接口池</span>
                <span className="text-xs font-normal text-muted-foreground py-0.5 px-2 bg-muted rounded-md">
                  {providerTabs.find(t => t.id === activeTab)?.label}
                </span>
              </CardTitle>
              <CardDescription className="text-xs sm:text-sm mt-1">
                配置不同模型渠道的 API Keys 与自定义 Base URL（支持配置多个凭证，请求将自动故障转移与负载均衡）。
              </CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          <div className="w-full mb-6">
            <div className="bg-muted/70 dark:bg-muted/40 rounded-xl p-1 border border-border/60">
              <div className="relative flex w-full">
                {/* Sliding Capsule Background */}
                <div
                  className="absolute inset-y-0 rounded-lg bg-card shadow-xs border border-border/80 transition-all duration-300 ease-out"
                  style={{ width: `${100 / providerTabs.length}%`, transform: `translateX(${activeIndex * 100}%)` }}
                />

                {providerTabs.map((tab) => (
                  <button
                    key={tab.id}
                    type="button"
                    className={cn(
                      "relative z-10 flex-1 flex items-center justify-center gap-1.5 sm:gap-2 py-2 px-1 sm:px-3 text-xs sm:text-sm font-medium transition-colors duration-200 rounded-lg cursor-pointer select-none",
                      activeTab === tab.id
                        ? "text-foreground font-semibold"
                        : "text-muted-foreground hover:text-foreground"
                    )}
                    onClick={() => handleTabChange(tab.id)}
                  >
                    <AuthProviderBrandIcon
                      provider={tab.id === 'vertex' ? 'google' : tab.id}
                      size={15}
                      className={cn(
                        "transition-transform shrink-0",
                        activeTab === tab.id ? "scale-105" : "opacity-70"
                      )}
                    />
                    <span className="truncate">{tab.label}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>

          <div>
            {providerTabs.map((tab) => {
              if (!visitedTabs.has(tab.id)) return null
              const isActive = activeTab === tab.id
              return (
                <div
                  key={tab.id}
                  className={cn(!isActive && 'hidden')}
                  role="tabpanel"
                  aria-hidden={!isActive}
                >
                  <ProviderTab
                    providerKind={tab.id}
                    endpoint={PROVIDER_ENDPOINTS[tab.id]}
                    refreshSignal={providerRefreshSignal}
                    onOpenModelsDialog={handleOpenModelsDialog}
                    onProbeForm={handleProbeForm}
                  />
                </div>
              )
            })}
          </div>
        </CardContent>
      </Card>

      <ProviderModelsDialog
        open={!!modelsDialogItem}
        onOpenChange={(open) => !open && setModelsDialogItem(null)}
        provider={modelsDialogItem?.name || providerLabel(dialogProviderKind)}
        providerKind={dialogProviderKind}
        baseUrl={modelsDialogItem?.baseUrl || ''}
        modelsUrl={dialogProviderKind === 'openai' ? (modelsDialogItem?.modelsUrl ?? '') : itemModelsUrl(modelsDialogItem)}
        apiKey={modelsDialogItem?.apiKey || ''}
        configuredModels={modelsDialogItem?.originalPayload?.models}
        onSaveConfiguredModels={modelsDialogItem && modelsDialogItem.index >= 0 ? handleSaveConfiguredModels : undefined}
        onSaveModelsUrl={modelsDialogItem && modelsDialogItem.index >= 0 && dialogProviderKind === 'openai' ? handleSaveModelsUrl : undefined}
      />
    </div>
  )
}
