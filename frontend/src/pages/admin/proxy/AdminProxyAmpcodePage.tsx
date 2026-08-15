import { useCallback, useEffect, useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card'
import { Switch } from '@/shared/components/ui/switch'
import { Label } from '@/shared/components/ui/label'
import { Button } from '@/shared/components/ui/button'
import { AlertCircle, CheckCircle2, Loader2, PlugZap, XCircle, Plus, Trash2 } from 'lucide-react'
import { toast } from 'sonner'
import { fetchProviderConfig, updateProviderConfig, deleteProviderConfig } from '@/features/admin-proxy/api'
import { apiCallRequest } from '@/features/admin-proxy/api'
import { Input } from '@/shared/components/ui/input'
import { Textarea } from '@/shared/components/ui/textarea'
import { testAmpcodeUpstream, type AmpcodeUpstreamTestResult } from '@/features/admin-proxy/ampcodeUpstreamTest'
import {
  buildAmpModelMappingsPutPayload,
  buildAmpUpstreamAPIKeysDeletePayload,
  buildAmpUpstreamAPIKeysPutPayload,
  extractAmpcodeConfig,
  normalizeAmpModelMappings,
  normalizeAmpUpstreamAPIKeyEntries,
  parseAmpUpstreamAPIKeyForm,
  type AmpModelMapping,
  type AmpUpstreamAPIKeyEntry,
} from '@/features/admin-proxy/ampcodeConfig'
import { cn } from '@/shared/utils/utils'

export default function AdminProxyAmpcodePage() {
  const [loading, setLoading] = useState(true)
  
  const [upstreamUrl, setUpstreamUrl] = useState('')
  const [upstreamApiKey, setUpstreamApiKey] = useState('')
  const [upstreamKeyEntries, setUpstreamKeyEntries] = useState<AmpUpstreamAPIKeyEntry[]>([])
  const [newMappedUpstreamKey, setNewMappedUpstreamKey] = useState('')
  const [newMappedClientKeys, setNewMappedClientKeys] = useState('')
  const [testingUpstream, setTestingUpstream] = useState(false)
  const [upstreamTestResult, setUpstreamTestResult] = useState<AmpcodeUpstreamTestResult | null>(null)
  const [forceModelMappings, setForceModelMappings] = useState(false)
  const [modelMappings, setModelMappings] = useState<AmpModelMapping[]>([])

  const [togglesLoading, setTogglesLoading] = useState<Record<string, boolean>>({})
  const [urlFormHydrated, setUrlFormHydrated] = useState(false)

  const fetchAll = useCallback(async () => {
    setLoading(true)
    try {
      const data = await fetchProviderConfig<Record<string, unknown>>('/ampcode')
      const ampConfig = extractAmpcodeConfig(data)
      const upstreamUrlValue = ampConfig['upstream-url'] ?? ampConfig.upstream_url
      const upstreamApiKeyValue = ampConfig['upstream-api-key'] ?? ampConfig.upstream_api_key
      const forceModelMappingsValue = ampConfig['force-model-mappings'] ?? ampConfig.force_model_mappings
      setUpstreamUrl(typeof upstreamUrlValue === 'string' ? upstreamUrlValue : '')
      setUpstreamApiKey(typeof upstreamApiKeyValue === 'string' ? upstreamApiKeyValue : '')
      setForceModelMappings(typeof forceModelMappingsValue === 'boolean' ? forceModelMappingsValue : false)

      const mappingRes = await fetchProviderConfig<Record<string, unknown>>('/ampcode/model-mappings')
      const mm = mappingRes['model-mappings'] || mappingRes.mappings || ampConfig['model-mappings'] || []
      setModelMappings(normalizeAmpModelMappings(mm))

      const upstreamKeysRes = await fetchProviderConfig<Record<string, unknown>>('/ampcode/upstream-api-keys')
      setUpstreamKeyEntries(normalizeAmpUpstreamAPIKeyEntries((upstreamKeysRes['upstream-api-keys'] || []) as AmpUpstreamAPIKeyEntry[]))
    } catch (e: unknown) {
      toast.error(`读取数据失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setLoading(false)
      setUrlFormHydrated(true)
    }
  }, [])

  useEffect(() => {
    const timer = window.setTimeout(() => void fetchAll(), 0)
    return () => window.clearTimeout(timer)
  }, [fetchAll])

  const updateSetting = useCallback(
    async (
      key: string,
      endpoint: string,
      val: string | boolean,
      isDelete = false,
      options?: { silent?: boolean },
    ) => {
      setTogglesLoading(prev => ({ ...prev, [key]: true }))
      try {
        if (isDelete) {
          await deleteProviderConfig(endpoint)
        } else {
          await updateProviderConfig(endpoint, { value: val })
        }
        if (!options?.silent) {
          toast.success('配置已更新')
        }
      } catch (e: unknown) {
        toast.error(`更新失败: ${e instanceof Error ? e.message : String(e)}`)
        void fetchAll() // rollback
      } finally {
        setTogglesLoading(prev => ({ ...prev, [key]: false }))
      }
    },
    [fetchAll],
  )

  useEffect(() => {
    if (!urlFormHydrated) return
    const id = window.setTimeout(() => {
      void updateSetting('url', '/ampcode/upstream-url', upstreamUrl, !upstreamUrl, { silent: true })
    }, 600)
    return () => window.clearTimeout(id)
  }, [upstreamUrl, urlFormHydrated, updateSetting])

  useEffect(() => {
    if (!urlFormHydrated) return
    const id = window.setTimeout(() => {
      void updateSetting('key', '/ampcode/upstream-api-key', upstreamApiKey, !upstreamApiKey, { silent: true })
    }, 600)
    return () => window.clearTimeout(id)
  }, [upstreamApiKey, urlFormHydrated, updateSetting])

  const handleSaveMappings = async () => {
    setTogglesLoading(prev => ({ ...prev, mappings: true }))
    try {
      await updateProviderConfig('/ampcode/model-mappings', buildAmpModelMappingsPutPayload(modelMappings))
      setModelMappings(prev => normalizeAmpModelMappings(prev).filter(entry => entry.from && entry.to))
      toast.success('模型映射已更新')
    } catch (e: unknown) {
      toast.error(`保存失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setTogglesLoading(prev => ({ ...prev, mappings: false }))
    }
  }

  const updateModelMapping = (index: number, patch: Partial<AmpModelMapping>) => {
    setModelMappings(prev => prev.map((entry, i) => i === index ? { ...entry, ...patch } : entry))
  }

  const addModelMapping = () => {
    setModelMappings(prev => [...prev, { from: '', to: '', regex: false }])
  }

  const deleteModelMapping = (index: number) => {
    setModelMappings(prev => prev.filter((_, i) => i !== index))
  }

  const handleAddUpstreamKeyMapping = async () => {
    try {
      const entry = parseAmpUpstreamAPIKeyForm({
        upstreamApiKey: newMappedUpstreamKey,
        apiKeysText: newMappedClientKeys,
      })
      if (!entry['upstream-api-key']) {
        toast.error('上游 API Key 不能为空')
        return
      }
      const next = normalizeAmpUpstreamAPIKeyEntries([
        ...upstreamKeyEntries.filter(item => item['upstream-api-key'] !== entry['upstream-api-key']),
        entry,
      ])
      await updateAmpUpstreamKeyEntries(next)
      setNewMappedUpstreamKey('')
      setNewMappedClientKeys('')
    } catch (e: unknown) {
      toast.error(`保存失败: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  const updateAmpUpstreamKeyEntries = async (entries: AmpUpstreamAPIKeyEntry[]) => {
    setTogglesLoading(prev => ({ ...prev, upstreamKeyEntries: true }))
    try {
      await updateProviderConfig('/ampcode/upstream-api-keys', buildAmpUpstreamAPIKeysPutPayload(entries))
      setUpstreamKeyEntries(entries)
      toast.success('上游 Key 映射已更新')
    } catch (e: unknown) {
      toast.error(`更新失败: ${e instanceof Error ? e.message : String(e)}`)
      fetchAll()
    } finally {
      setTogglesLoading(prev => ({ ...prev, upstreamKeyEntries: false }))
    }
  }

  const handleDeleteUpstreamKeyMapping = async (upstreamKey: string) => {
    setTogglesLoading(prev => ({ ...prev, upstreamKeyEntries: true }))
    try {
      await deleteProviderConfig('/ampcode/upstream-api-keys', buildAmpUpstreamAPIKeysDeletePayload([upstreamKey]))
      setUpstreamKeyEntries(prev => prev.filter(item => item['upstream-api-key'] !== upstreamKey))
      toast.success('映射已删除')
    } catch (e: unknown) {
      toast.error(`删除失败: ${e instanceof Error ? e.message : String(e)}`)
      fetchAll()
    } finally {
      setTogglesLoading(prev => ({ ...prev, upstreamKeyEntries: false }))
    }
  }

  const handleTestUpstream = async () => {
    if (!upstreamUrl.trim()) {
      toast.error('请先填写上游地址')
      return
    }
    if (!upstreamApiKey.trim()) {
      toast.error('请先填写上游 API Key')
      return
    }

    setTestingUpstream(true)
    setUpstreamTestResult(null)
    try {
      const result = await testAmpcodeUpstream({ upstreamUrl, upstreamApiKey }, apiCallRequest)
      setUpstreamTestResult(result)

      if (result.status === 'connected') {
        toast.success(result.message)
      } else if (result.status === 'reachable') {
        toast(result.message)
      } else {
        toast.error(result.message)
      }
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e)
      toast.error(`测试失败: ${message}`)
      setUpstreamTestResult({
        status: 'failed',
        message,
        endpoint: upstreamUrl,
        checkedAt: new Date().toISOString(),
      })
    } finally {
      setTestingUpstream(false)
    }
  }

  const testResultMeta = (() => {
    if (!upstreamTestResult) return null
    if (upstreamTestResult.status === 'connected') {
      return {
        label: '连接正常',
        icon: CheckCircle2,
        className: 'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-800/50 dark:bg-emerald-950/20 dark:text-emerald-300',
      }
    }
    if (upstreamTestResult.status === 'reachable') {
      return {
        label: '网络可达',
        icon: AlertCircle,
        className: 'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-800/50 dark:bg-amber-950/20 dark:text-amber-300',
      }
    }
    return {
      label: '连接失败',
      icon: XCircle,
      className: 'border-red-200 bg-red-50 text-red-700 dark:border-red-800/50 dark:bg-red-950/20 dark:text-red-300',
    }
  })()
  const TestResultIcon = testResultMeta?.icon

  return (
    <div className="space-y-6 max-w-4xl">
      <div className="grid gap-6">
        <Card>
          <CardHeader>
            <CardTitle>连接渠道与凭证</CardTitle>
            <CardDescription>用于对接下级 Ampcode 网关或中转程序的 URL 和 Key。<br />
            <span className="text-amber-500 font-medium">注意：如果您想配置 OpenAI、Claude、Gemini 等官方渠道或其他兼容的第三方网关，请前往「上游认证」页面。</span></CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex flex-col gap-4 p-4 rounded-lg border bg-card">
              <div className="grid gap-2">
                <div className="flex items-center justify-between gap-2">
                  <Label className="mb-0">Upstream URL (上游地址)</Label>
                  {togglesLoading['url'] ? (
                    <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      保存中
                    </span>
                  ) : null}
                </div>
                <Input
                  value={upstreamUrl}
                  onChange={e => {
                    setUpstreamUrl(e.target.value)
                    setUpstreamTestResult(null)
                  }}
                  placeholder="https://ampcode.com"
                />
              </div>

              <div className="grid gap-2 mt-2">
                <div className="flex items-center justify-between gap-2">
                  <Label className="mb-0">Upstream API Key (上游密钥)</Label>
                  {togglesLoading['key'] ? (
                    <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      保存中
                    </span>
                  ) : null}
                </div>
                <Input
                  type="password"
                  value={upstreamApiKey}
                  onChange={e => {
                    setUpstreamApiKey(e.target.value)
                    setUpstreamTestResult(null)
                  }}
                  placeholder="sgamp_user_..."
                />
              </div>

              <div className="flex flex-wrap items-center gap-3 pt-2">
                <Button
                  variant="outline"
                  onClick={handleTestUpstream}
                  disabled={testingUpstream || loading || !upstreamUrl.trim() || !upstreamApiKey.trim()}
                >
                  {testingUpstream ? (
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  ) : (
                    <PlugZap className="mr-2 h-4 w-4" />
                  )}
                  测试连接
                </Button>
                <span className="text-xs text-muted-foreground">使用当前输入值发起只读探测，不会保存配置。</span>
              </div>

              {upstreamTestResult && testResultMeta && (
                <div className={cn('rounded-md border px-3 py-2 text-sm', testResultMeta.className)}>
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    <span className="inline-flex items-center gap-1.5 font-medium">
                      {TestResultIcon && <TestResultIcon className="h-4 w-4" />}
                      {testResultMeta.label}
                    </span>
                    {typeof upstreamTestResult.statusCode === 'number' && (
                      <span className="font-mono text-xs">HTTP {upstreamTestResult.statusCode}</span>
                    )}
                    {typeof upstreamTestResult.elapsedMs === 'number' && (
                      <span className="font-mono text-xs">{upstreamTestResult.elapsedMs}ms</span>
                    )}
                  </div>
                  <p className="mt-1">{upstreamTestResult.message}</p>
                  {upstreamTestResult.endpoint && (
                    <p className="mt-1 break-all font-mono text-xs opacity-80">{upstreamTestResult.endpoint}</p>
                  )}
                  {upstreamTestResult.bodyPreview && upstreamTestResult.status !== 'connected' && (
                    <p className="mt-2 break-words rounded bg-white/60 px-2 py-1 font-mono text-xs opacity-90 dark:bg-black/20">
                      {upstreamTestResult.bodyPreview}
                    </p>
                  )}
                </div>
              )}
            </div>

            <div className="flex flex-col gap-4 p-4 rounded-lg border bg-card">
              <div>
                <Label>Upstream API Keys 映射</Label>
                <p className="text-xs text-muted-foreground mt-1">按 SDK 顶层 api-keys 将 Amp 请求路由到不同上游 API Key。CPA 用户 API Key 会先被网关改写为内部 Key，这里的映射仅适用于直接使用 SDK api-keys 的场景。</p>
              </div>
              <div className="flex flex-col gap-3">
                <Input
                  type="password"
                  value={newMappedUpstreamKey}
                  onChange={e => setNewMappedUpstreamKey(e.target.value)}
                  placeholder="上游 API Key"
                />
                <Textarea
                  className="min-h-[4.5rem] w-full font-mono text-xs"
                  value={newMappedClientKeys}
                  onChange={e => setNewMappedClientKeys(e.target.value)}
                  placeholder="客户端 API Keys，换行或逗号分隔"
                />
                <Button
                  className="w-fit"
                  onClick={handleAddUpstreamKeyMapping}
                  disabled={loading || togglesLoading['upstreamKeyEntries']}
                >
                  <Plus className="mr-2 h-4 w-4" />
                  添加映射
                </Button>
              </div>
              <div className="rounded-md border overflow-hidden">
                {upstreamKeyEntries.length === 0 ? (
                  <div className="px-3 py-6 text-center text-sm text-muted-foreground">暂无多上游 Key 映射</div>
                ) : (
                  <div className="divide-y">
                    {upstreamKeyEntries.map(entry => (
                      <div key={entry['upstream-api-key']} className="grid gap-3 md:grid-cols-[1fr_1fr_auto] items-center px-3 py-2">
                        <span className="font-mono text-xs truncate" title={entry['upstream-api-key']}>{entry['upstream-api-key']}</span>
                        <span className="text-xs text-muted-foreground truncate" title={entry['api-keys'].join(', ')}>
                          {entry['api-keys'].length ? entry['api-keys'].join(', ') : '未绑定客户端 Key'}
                        </span>
                        <Button type="button" variant="dangerIcon" onClick={() => handleDeleteUpstreamKeyMapping(entry['upstream-api-key'])} title="删除" aria-label="删除">
                          <Trash2 />
                        </Button>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>底层调度参数</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 p-4 rounded-lg border bg-card">
              <div className="space-y-0.5">
                <Label className="text-base">强制开启模型映射</Label>
                <p className="text-sm text-muted-foreground">开启后，Amp 请求会优先应用下方映射，即使 from 模型本身已有可用渠道。</p>
              </div>
              <div>
                {loading || togglesLoading['forceMap'] ? (
                  <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                ) : (
                  <Switch 
                     checked={forceModelMappings} 
                     onCheckedChange={(v) => {
                       setForceModelMappings(v)
                       updateSetting('forceMap', '/ampcode/force-model-mappings', v)
                     }} 
                  />
                )}
              </div>
            </div>

            <div className="p-4 rounded-lg border bg-card space-y-4">
               <div>
                  <Label>Model Mappings (模型映射)</Label>
                  <p className="text-xs text-muted-foreground mb-2">当 Amp 请求的 from 模型不可用时，转发到 to 模型。SDK 保存格式为 <code>{`[{ "from": "...", "to": "..." }]`}</code>。</p>
               </div>
               <div className="space-y-3">
                 {modelMappings.length === 0 ? (
                   <div className="rounded-md border border-dashed px-3 py-6 text-center text-sm text-muted-foreground">暂无模型映射</div>
                 ) : (
                   modelMappings.map((mapping, index) => (
                     <div key={index} className="grid gap-3 md:grid-cols-[1fr_1fr_auto_auto] md:items-center">
                       <Input
                         value={mapping.from}
                         onChange={e => updateModelMapping(index, { from: e.target.value })}
                         placeholder="from: claude-opus-4-5-20251101"
                       />
                       <Input
                         value={mapping.to}
                         onChange={e => updateModelMapping(index, { to: e.target.value })}
                         placeholder="to: gemini-claude-sonnet-4-5"
                       />
                       <div className="flex items-center gap-2 rounded-md border px-3 py-2">
                         <Switch checked={!!mapping.regex} onCheckedChange={v => updateModelMapping(index, { regex: v })} />
                         <span className="text-xs text-muted-foreground">Regex</span>
                       </div>
                       <Button type="button" variant="dangerIcon" onClick={() => deleteModelMapping(index)} title="删除映射" aria-label="删除映射">
                         <Trash2 />
                       </Button>
                     </div>
                   ))
                 )}
               </div>
               <div className="flex flex-wrap gap-2">
                 <Button variant="outline" onClick={addModelMapping} disabled={loading}>
                   <Plus className="mr-2 h-4 w-4" />
                   添加映射
                 </Button>
                 <Button onClick={() => void handleSaveMappings()} disabled={loading || togglesLoading['mappings']}>
                   保存映射
                 </Button>
               </div>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
