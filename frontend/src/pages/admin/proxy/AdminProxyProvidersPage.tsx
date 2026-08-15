import { useState } from 'react'
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

  const handleOpenModelsDialog = async (item: BaseChannelItem) => {
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
  }

  const handleProbeForm = (apiKey: string, baseUrl: string, name: string, modelsUrl?: string) => {
    setModelsDialogItem({ _id: 'test', providerKind: activeTab, index: -1, apiKey, baseUrl, modelsUrl, originalPayload: {}, name })
  }

  const handleSaveConfiguredModels = async (models: ModelInfo[]) => {
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
  }

  const handleSaveModelsUrl = async (modelsUrl: string) => {
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
  }

  const dialogProviderKind = modelsDialogItem?.providerKind ?? activeTab

  return (
    <div className="space-y-6 max-w-6xl">
      <Card>
        <CardHeader>
          <CardTitle>多渠道接口池</CardTitle>
          <CardDescription>配置不同模型渠道的 API Keys 和自定义 Base URL（支持配置多个，自动负载均衡）。对于不支持或者自定义的协议，请使用 OpenAI 兼容格式。</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="w-full mb-6">
            <div className="bg-slate-100/80 dark:bg-dark-800/60 rounded-xl p-1 border border-slate-200/50 dark:border-dark-700/50">
              <div className="relative flex w-full">
                {/* Sliding Capsule Background */}
                <div
                  className="absolute inset-y-0 rounded-lg bg-white dark:bg-dark-700 shadow-sm border border-slate-200/50 dark:border-dark-600/50 transition-all duration-300 ease-out"
                  style={{ width: `${100 / providerTabs.length}%`, transform: `translateX(${activeIndex * 100}%)` }}
                />

                {providerTabs.map((tab) => (
                  <button
                    key={tab.id}
                    className={cn(
                      "relative z-10 flex-1 flex items-center justify-center py-2 text-sm font-medium transition-colors duration-300 rounded-lg cursor-pointer",
                      activeTab === tab.id
                        ? "text-slate-900 dark:text-white"
                        : "text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-300"
                    )}
                    onClick={() => setActiveTab(tab.id)}
                  >
                    {tab.label}
                  </button>
                ))}
              </div>
            </div>
          </div>

          <div key={activeTab} className="animate-in fade-in duration-300" style={{ willChange: 'opacity' }}>
            <ProviderTab
              providerKind={activeTab}
              endpoint={PROVIDER_ENDPOINTS[activeTab]}
              refreshSignal={providerRefreshSignal}
              onOpenModelsDialog={handleOpenModelsDialog}
              onProbeForm={handleProbeForm}
            />
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
