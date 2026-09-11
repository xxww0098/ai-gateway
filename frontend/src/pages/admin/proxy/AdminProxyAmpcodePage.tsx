import { useCallback, useEffect, useState } from 'react'
import { Globe, KeyRound, GitFork, Loader2 } from 'lucide-react'
import { toast } from 'sonner'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/shared/components/ui/tabs'
import { Badge } from '@/shared/components/ui/badge'
import { fetchProviderConfig, updateProviderConfig, deleteProviderConfig, apiCallRequest } from '@/features/admin-proxy/api'
import { testAmpcodeUpstream, type AmpcodeUpstreamTestResult } from '@/features/admin-proxy/ampcodeUpstreamTest'
import {
  buildAmpModelMappingsPutPayload,
  buildAmpUpstreamAPIKeysDeletePayload,
  buildAmpUpstreamAPIKeysPutPayload,
  extractAmpcodeConfig,
  normalizeAmpModelMappings,
  normalizeAmpUpstreamAPIKeyEntries,
  type AmpModelMapping,
  type AmpUpstreamAPIKeyEntry,
} from '@/features/admin-proxy/ampcodeConfig'

import { AmpcodeHeader } from '@/features/admin-proxy/components/ampcode/AmpcodeHeader'
import { AmpcodeConnectionCard } from '@/features/admin-proxy/components/ampcode/AmpcodeConnectionCard'
import { AmpcodeKeyRoutingCard } from '@/features/admin-proxy/components/ampcode/AmpcodeKeyRoutingCard'
import { AmpcodeKeyMappingDialog } from '@/features/admin-proxy/components/ampcode/AmpcodeKeyMappingDialog'
import { AmpcodeModelMappingsCard } from '@/features/admin-proxy/components/ampcode/AmpcodeModelMappingsCard'
import { AmpcodeJsonBackupDialog } from '@/features/admin-proxy/components/ampcode/AmpcodeJsonBackupDialog'

export default function AdminProxyAmpcodePage() {
  const [loading, setLoading] = useState(true)
  const [rawConfig, setRawConfig] = useState<Record<string, unknown>>({})

  // Upstream Connection State
  const [upstreamUrl, setUpstreamUrl] = useState('')
  const [savedUrl, setSavedUrl] = useState('')
  const [upstreamApiKey, setUpstreamApiKey] = useState('')
  const [savedApiKey, setSavedApiKey] = useState('')
  const [savingConnection, setSavingConnection] = useState(false)

  // Testing State
  const [testingUpstream, setTestingUpstream] = useState(false)
  const [upstreamTestResult, setUpstreamTestResult] = useState<AmpcodeUpstreamTestResult | null>(null)

  // Key Routing State
  const [upstreamKeyEntries, setUpstreamKeyEntries] = useState<AmpUpstreamAPIKeyEntry[]>([])
  const [deletingKey, setDeletingKey] = useState<string | null>(null)
  const [keyDialogOpen, setKeyDialogOpen] = useState(false)
  const [editingKeyEntry, setEditingKeyEntry] = useState<AmpUpstreamAPIKeyEntry | null>(null)
  const [savingKeyEntry, setSavingKeyEntry] = useState(false)

  // Model Mappings State
  const [forceModelMappings, setForceModelMappings] = useState(false)
  const [togglingForce, setTogglingForce] = useState(false)
  const [modelMappings, setModelMappings] = useState<AmpModelMapping[]>([])
  const [savedMappings, setSavedMappings] = useState<AmpModelMapping[]>([])
  const [savingMappings, setSavingMappings] = useState(false)

  // Backup Dialog
  const [jsonDialogOpen, setJsonDialogOpen] = useState(false)

  // Load all configurations
  const fetchAll = useCallback(async () => {
    setLoading(true)
    try {
      const [data, mappingRes, upstreamKeysRes] = await Promise.all([
        fetchProviderConfig<Record<string, unknown>>('/ampcode'),
        fetchProviderConfig<Record<string, unknown>>('/ampcode/model-mappings'),
        fetchProviderConfig<Record<string, unknown>>('/ampcode/upstream-api-keys'),
      ])

      const ampConfig = extractAmpcodeConfig(data)
      setRawConfig(data)

      const urlVal = ampConfig['upstream-url'] ?? ampConfig.upstream_url
      const keyVal = ampConfig['upstream-api-key'] ?? ampConfig.upstream_api_key
      const forceVal = ampConfig['force-model-mappings'] ?? ampConfig.force_model_mappings

      const resolvedUrl = typeof urlVal === 'string' ? urlVal : ''
      const resolvedKey = typeof keyVal === 'string' ? keyVal : ''
      const resolvedForce = typeof forceVal === 'boolean' ? forceVal : false

      setUpstreamUrl(resolvedUrl)
      setSavedUrl(resolvedUrl)
      setUpstreamApiKey(resolvedKey)
      setSavedApiKey(resolvedKey)
      setForceModelMappings(resolvedForce)

      const mm = mappingRes['model-mappings'] || mappingRes.mappings || ampConfig['model-mappings'] || []
      const normalizedMappings = normalizeAmpModelMappings(mm)
      setModelMappings(normalizedMappings)
      setSavedMappings(normalizedMappings)

      const rawKeys = (upstreamKeysRes['upstream-api-keys'] || []) as AmpUpstreamAPIKeyEntry[]
      setUpstreamKeyEntries(normalizeAmpUpstreamAPIKeyEntries(rawKeys))
    } catch (e: unknown) {
      toast.error(`读取数据失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void fetchAll()
  }, [fetchAll])

  // Save Connection Config
  const handleSaveConnection = async () => {
    setSavingConnection(true)
    try {
      const trimmedUrl = upstreamUrl.trim()
      const trimmedKey = upstreamApiKey.trim()

      await updateProviderConfig('/ampcode', {
        'upstream-url': trimmedUrl,
        'upstream-api-key': trimmedKey,
      })

      setSavedUrl(trimmedUrl)
      setSavedApiKey(trimmedKey)
      toast.success('上游服务连接配置已更新')
    } catch (e: unknown) {
      toast.error(`保存失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setSavingConnection(false)
    }
  }

  const handleDiscardConnection = () => {
    setUpstreamUrl(savedUrl)
    setUpstreamApiKey(savedApiKey)
    toast.info('已还原未保存的连接配置修改')
  }

  // Test Connection
  const handleTestConnection = async (allowFallback: boolean) => {
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
      const result = await testAmpcodeUpstream(
        {
          upstreamUrl,
          upstreamApiKey,
          allowFallback,
        },
        apiCallRequest,
      )
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

  // Toggle Force Model Mappings
  const handleToggleForceMappings = async (enabled: boolean) => {
    setTogglingForce(true)
    const prev = forceModelMappings
    setForceModelMappings(enabled)

    try {
      await updateProviderConfig('/ampcode/force-model-mappings', { value: enabled })
      toast.success(enabled ? '已开启强制模型重定向' : '已切换为故障旁路重定向')
    } catch (e: unknown) {
      setForceModelMappings(prev) // rollback
      toast.error(`更新策略失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setTogglingForce(false)
    }
  }

  // Save Model Mappings
  const handleSaveMappings = async () => {
    setSavingMappings(true)
    try {
      const payload = buildAmpModelMappingsPutPayload(modelMappings)
      await updateProviderConfig('/ampcode/model-mappings', payload)
      const cleaned = normalizeAmpModelMappings(modelMappings).filter(entry => entry.from && entry.to)
      setModelMappings(cleaned)
      setSavedMappings(cleaned)
      toast.success('模型映射规则已成功保存')
    } catch (e: unknown) {
      toast.error(`保存失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setSavingMappings(false)
    }
  }

  const handleDiscardMappings = () => {
    setModelMappings(savedMappings)
    toast.info('已还原未保存的映射修改')
  }

  // Key Routing Handlers
  const handleOpenAddKeyDialog = () => {
    setEditingKeyEntry(null)
    setKeyDialogOpen(true)
  }

  const handleOpenEditKeyDialog = (entry: AmpUpstreamAPIKeyEntry) => {
    setEditingKeyEntry(entry)
    setKeyDialogOpen(true)
  }

  const handleSaveKeyEntry = async (entry: AmpUpstreamAPIKeyEntry) => {
    setSavingKeyEntry(true)
    try {
      const filtered = upstreamKeyEntries.filter(
        item => item['upstream-api-key'] !== entry['upstream-api-key'],
      )
      const next = normalizeAmpUpstreamAPIKeyEntries([...filtered, entry])

      await updateProviderConfig(
        '/ampcode/upstream-api-keys',
        buildAmpUpstreamAPIKeysPutPayload(next),
      )
      setUpstreamKeyEntries(next)
      toast.success('SDK 密钥路由已更新')
    } catch (e: unknown) {
      toast.error(`保存失败: ${e instanceof Error ? e.message : String(e)}`)
      throw e
    } finally {
      setSavingKeyEntry(false)
    }
  }

  const handleDeleteKeyEntry = async (upstreamKey: string) => {
    setDeletingKey(upstreamKey)
    try {
      await deleteProviderConfig(
        '/ampcode/upstream-api-keys',
        buildAmpUpstreamAPIKeysDeletePayload([upstreamKey]),
      )
      setUpstreamKeyEntries(prev => prev.filter(item => item['upstream-api-key'] !== upstreamKey))
      toast.success('密钥映射已删除')
    } catch (e: unknown) {
      toast.error(`删除失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setDeletingKey(null)
    }
  }

  // Compute summary stats
  const regexMappingsCount = modelMappings.filter(m => m.regex).length
  const totalClientKeysCount = upstreamKeyEntries.reduce(
    (acc, cur) => acc + cur['api-keys'].length,
    0,
  )

  if (loading) {
    return (
      <div className="flex flex-col items-center justify-center py-24 gap-3 text-muted-foreground">
        <Loader2 className="h-8 w-8 animate-spin text-primary" />
        <p className="text-sm font-medium">正在读取 Ampcode 专线配置...</p>
      </div>
    )
  }

  return (
    <div className="space-y-6 max-w-5xl">
      {/* Overview & KPI Header */}
      <AmpcodeHeader
        loading={loading}
        onRefresh={fetchAll}
        onOpenJsonDialog={() => setJsonDialogOpen(true)}
        upstreamConfigured={!!savedUrl}
        upstreamTestResult={upstreamTestResult}
        forceModelMappings={forceModelMappings}
        modelMappingsCount={modelMappings.length}
        regexMappingsCount={regexMappingsCount}
        upstreamKeyEntriesCount={upstreamKeyEntries.length}
        totalClientKeysCount={totalClientKeysCount}
      />

      {/* Structured Functional Tabs */}
      <Tabs defaultValue="connection" className="space-y-4">
        <TabsList className="grid w-full grid-cols-3 h-10 p-1 bg-muted/70 rounded-xl">
          <TabsTrigger value="connection" className="text-xs sm:text-sm gap-2 rounded-lg cursor-pointer">
            <Globe className="h-4 w-4" />
            <span>专线上游与诊断</span>
          </TabsTrigger>

          <TabsTrigger value="keys" className="text-xs sm:text-sm gap-2 rounded-lg cursor-pointer">
            <KeyRound className="h-4 w-4" />
            <span>SDK 密钥路由</span>
            {upstreamKeyEntries.length > 0 && (
              <Badge variant="secondary" className="px-1.5 py-0 text-[10px] h-4.5 rounded-full font-mono">
                {upstreamKeyEntries.length}
              </Badge>
            )}
          </TabsTrigger>

          <TabsTrigger value="mappings" className="text-xs sm:text-sm gap-2 rounded-lg cursor-pointer">
            <GitFork className="h-4 w-4" />
            <span>模型重定向调度</span>
            {modelMappings.length > 0 && (
              <Badge variant="secondary" className="px-1.5 py-0 text-[10px] h-4.5 rounded-full font-mono">
                {modelMappings.length}
              </Badge>
            )}
          </TabsTrigger>
        </TabsList>

        {/* Tab 1: Upstream Connection & Diagnostics */}
        <TabsContent value="connection" className="mt-0 space-y-4 focus-visible:outline-none">
          <AmpcodeConnectionCard
            upstreamUrl={upstreamUrl}
            setUpstreamUrl={setUpstreamUrl}
            upstreamApiKey={upstreamApiKey}
            setUpstreamApiKey={setUpstreamApiKey}
            savedUrl={savedUrl}
            savedApiKey={savedApiKey}
            onSaveConnection={handleSaveConnection}
            onDiscardConnection={handleDiscardConnection}
            saving={savingConnection}
            testing={testingUpstream}
            onTestConnection={handleTestConnection}
            testResult={upstreamTestResult}
          />
        </TabsContent>

        {/* Tab 2: SDK Key Routing */}
        <TabsContent value="keys" className="mt-0 space-y-4 focus-visible:outline-none">
          <AmpcodeKeyRoutingCard
            entries={upstreamKeyEntries}
            onOpenAddDialog={handleOpenAddKeyDialog}
            onOpenEditDialog={handleOpenEditKeyDialog}
            onDeleteEntry={handleDeleteKeyEntry}
            deletingKey={deletingKey}
          />
        </TabsContent>

        {/* Tab 3: Model Remappings & Simulator */}
        <TabsContent value="mappings" className="mt-0 space-y-4 focus-visible:outline-none">
          <AmpcodeModelMappingsCard
            forceModelMappings={forceModelMappings}
            onToggleForceMappings={handleToggleForceMappings}
            togglingForce={togglingForce}
            modelMappings={modelMappings}
            setModelMappings={setModelMappings}
            savedMappings={savedMappings}
            onSaveMappings={handleSaveMappings}
            onDiscardMappings={handleDiscardMappings}
            savingMappings={savingMappings}
          />
        </TabsContent>
      </Tabs>

      {/* Modals & Dialogs */}
      <AmpcodeKeyMappingDialog
        open={keyDialogOpen}
        onOpenChange={setKeyDialogOpen}
        initialEntry={editingKeyEntry}
        existingKeys={upstreamKeyEntries.map(e => e['upstream-api-key'])}
        onSave={handleSaveKeyEntry}
        saving={savingKeyEntry}
      />

      <AmpcodeJsonBackupDialog
        open={jsonDialogOpen}
        onOpenChange={setJsonDialogOpen}
        rawConfig={rawConfig}
        onConfigImported={fetchAll}
      />
    </div>
  )
}
