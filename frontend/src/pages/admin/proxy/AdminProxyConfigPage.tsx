import { useEffect, useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card'
import { Switch } from '@/shared/components/ui/switch'
import { Label } from '@/shared/components/ui/label'
import { Loader2 } from 'lucide-react'
import { toast } from 'sonner'
import { fetchMgmtApi } from '@/features/admin-proxy/api'
import { apiClient } from '@/shared/api/client'
import { Input } from '@/shared/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/components/ui/select'
import {
  ROUTING_STRATEGY_OPTIONS,
  buildSDKExtraConfigPatch,
  configValue,
  normalizeDisableImageGeneration,
  type DisableImageGenerationMode,
  type SDKExtraConfigForm,
} from '@/features/admin-proxy/sdkConfig'

interface ConfigSettingProps {
  title: string
  description: string
  loading: boolean
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  value: any
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  onChange: (val: any) => void
  type?: 'boolean' | 'number' | 'select' | 'string'
  options?: { label: string; value: string }[]
  max?: number
  /** string 类型失焦时回调，用于自动落库 */
  onStringBlur?: (val: string) => void
}

function ConfigSetting({ title, description, loading, value, onChange, type = 'boolean', options = [], max, onStringBlur }: ConfigSettingProps) {
  return (
    <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 p-4 rounded-lg border bg-card">
      <div className="space-y-0.5">
        <Label className="text-base">{title}</Label>
        <p className="text-sm text-muted-foreground">{description}</p>
      </div>
      <div>
        {loading ? (
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
        ) : type === 'boolean' ? (
          <Switch checked={value as boolean} onCheckedChange={onChange} />
        ) : type === 'number' ? (
          <Input 
            type="number" 
            className="w-32" 
            value={value as number} 
            onChange={(e) => onChange(e.target.value)} 
            max={max}
            onBlur={(e) => onChange(Number(e.target.value))}
          />
        ) : type === 'select' ? (
          <Select value={value as string} onValueChange={onChange}>
            <SelectTrigger className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {options.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <Input
            className="w-64"
            value={value as string}
            onChange={(e) => onChange(e.target.value)}
            onBlur={(e) => onStringBlur?.(e.target.value)}
          />
        )}
      </div>
    </div>
  )
}

export default function AdminProxyConfigPage() {
  const [loading, setLoading] = useState(true)
  
  // State for different settings
  const [debug, setDebug] = useState(false)
  const [requestLog, setRequestLog] = useState(false)
  const [loggingToFile, setLoggingToFile] = useState(false)
  const [wsAuth, setWsAuth] = useState(true)
  const [forceModelPrefix, setForceModelPrefix] = useState(false)
  
  const [routingStrategy, setRoutingStrategy] = useState('round-robin')
  const [requestRetry, setRequestRetry] = useState(3)
  const [maxRetryInterval, setMaxRetryInterval] = useState(30)
  const [maxRetryCredentials, setMaxRetryCredentials] = useState(0)
  const [usageStatsEnabled, setUsageStatsEnabled] = useState(false)
  const [redisRetention, setRedisRetention] = useState(60)
  const [disableImageGeneration, setDisableImageGeneration] = useState<DisableImageGenerationMode>('off')
  const [sessionAffinity, setSessionAffinity] = useState(false)
  const [sessionAffinityTtl, setSessionAffinityTtl] = useState('1h')
  const [proxyUrl, setProxyUrl] = useState('')
  const [logsMaxSize, setLogsMaxSize] = useState(100)

  const [togglesLoading, setTogglesLoading] = useState<Record<string, boolean>>({})

  const fetchAllConfig = async () => {
    setLoading(true)
    try {
      const globalConfig = await fetchMgmtApi('/config')
      
      const debugData = (await fetchMgmtApi('/debug')) as Record<string, unknown>
      setDebug(Boolean(debugData.debug ?? debugData.value ?? false))

      const routingData = (await fetchMgmtApi('/routing/strategy')) as Record<string, unknown>
      setRoutingStrategy(String(routingData.strategy ?? routingData['routing-strategy'] ?? routingData.routingStrategy ?? 'round-robin'))

      const forcePrefixData = (await fetchMgmtApi('/force-model-prefix')) as Record<string, unknown>
      setForceModelPrefix(Boolean(forcePrefixData['force-model-prefix'] ?? forcePrefixData.forceModelPrefix ?? false))

      const sizeData = (await fetchMgmtApi('/logs-max-total-size-mb')) as Record<string, unknown>
      setLogsMaxSize(Number(sizeData['logs-max-total-size-mb'] ?? sizeData.logsMaxTotalSizeMb ?? 100))

      const config = globalConfig as Record<string, unknown>
      const routing = (config.routing && typeof config.routing === 'object') ? config.routing as Record<string, unknown> : {}
      setProxyUrl(configValue(config, 'proxy-url', ''))
      setRequestRetry(configValue(config, 'request-retry', 3))
      setMaxRetryInterval(configValue(config, 'max-retry-interval', 30))
      setMaxRetryCredentials(configValue(config, 'max-retry-credentials', 0))
      setUsageStatsEnabled(configValue(config, 'usage-statistics-enabled', false))
      setRedisRetention(configValue(config, 'redis-usage-queue-retention-seconds', 60))
      setDisableImageGeneration(normalizeDisableImageGeneration(configValue(config, 'disable-image-generation', false)))
      setSessionAffinity(Boolean(routing['session-affinity']))
      setSessionAffinityTtl(typeof routing['session-affinity-ttl'] === 'string' ? routing['session-affinity-ttl'] : '1h')
      setRequestLog(configValue(config, 'request-log', false))
      setLoggingToFile(configValue(config, 'logging-to-file', false))
      setWsAuth(configValue(config, 'ws-auth', true))
      
    } catch (e: unknown) {
      toast.error(`读取配置失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void fetchAllConfig()
    }, 0)
    return () => window.clearTimeout(timer)
  }, [])

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const updateSetting = async (key: string, endpoint: string, val: any, updateStateFn: (v: any) => void) => {
    setTogglesLoading(prev => ({ ...prev, [key]: true }))
    try {
      const method = (val === '' && endpoint === '/proxy-url') ? 'DELETE' : 'PUT'
      const body = method === 'DELETE' ? undefined : JSON.stringify({ value: val })
      
      await fetchMgmtApi(endpoint, {
        method,
        body
      })
      updateStateFn(val)
      toast.success('已自动保存')
    } catch (e: unknown) {
      toast.error(`更新失败: ${e instanceof Error ? e.message : String(e)}`)
      fetchAllConfig() // rollback on fail
    } finally {
      setTogglesLoading(prev => ({ ...prev, [key]: false }))
    }
  }

  const updateSDKExtraSetting = async (key: string, patchForm: SDKExtraConfigForm, updateStateFn: () => void) => {
    setTogglesLoading(prev => ({ ...prev, [key]: true }))
    try {
      await apiClient.patch('/admin/sdk-config', buildSDKExtraConfigPatch(patchForm))
      updateStateFn()
      toast.success('已自动保存')
    } catch (e: unknown) {
      toast.error(`更新失败: ${e instanceof Error ? e.message : String(e)}`)
      fetchAllConfig()
    } finally {
      setTogglesLoading(prev => ({ ...prev, [key]: false }))
    }
  }

  return (
    <div className="space-y-6 max-w-4xl">
      <div className="grid gap-6">
        <Card>
          <CardHeader>
            <CardTitle>全局基础设置</CardTitle>
            <CardDescription>控制日志行为及调试模式。修改后立即同步至网关（自动保存）。</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <ConfigSetting 
              title="Debug / 调试模式" 
              description="开启后将在控制台输出更多底层网络请求信息。" 
              loading={loading || togglesLoading['debug']} 
              value={debug} 
              onChange={(val: boolean) => updateSetting('debug', '/debug', val, setDebug)} 
            />
            <ConfigSetting 
              title="请求日志记录" 
              description="是否在内存和日志页中记录所有通过的经过网关的完整请求与报错。" 
              loading={loading || togglesLoading['reqLog']} 
              value={requestLog} 
              onChange={(val: boolean) => updateSetting('reqLog', '/request-log', val, setRequestLog)} 
            />
            <ConfigSetting 
              title="写回磁盘 (Log to File)" 
              description="开启后会将请求记录同步输出至文件中 (logs/)" 
              loading={loading || togglesLoading['logToFile']} 
              value={loggingToFile} 
              onChange={(val: boolean) => updateSetting('logToFile', '/logging-to-file', val, setLoggingToFile)} 
            />
            <ConfigSetting 
              title="日志文件总容量阈值 (MB)" 
              description="限制日志文件夹所占用的磁盘大小上限，超出时会自动轮转清理。" 
              type="number"
              loading={loading || togglesLoading['logsSize']} 
              value={logsMaxSize} 
              onChange={(val) => typeof val === 'number' && updateSetting('logsSize', '/logs-max-total-size-mb', val, setLogsMaxSize)} 
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>网络请求行为</CardTitle>
            <CardDescription>请求重试、上游网络代理及端点行为配置。代理 URL 在输入框失焦时自动保存。</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <ConfigSetting 
              title="请求重试次数" 
              description="向上游发送网络请求失败后的自动重试补偿次数。" 
              type="number"
              max={10}
              loading={loading || togglesLoading['retry']} 
              value={requestRetry} 
              onChange={(val) => typeof val === 'number' && updateSetting('retry', '/request-retry', val, setRequestRetry)} 
            />
            <ConfigSetting
              title="最大重试等待时间 (秒)"
              description="冷却凭证恢复前最多等待多久再进行重试。"
              type="number"
              loading={loading || togglesLoading['maxRetryInterval']}
              value={maxRetryInterval}
              onChange={(val) => typeof val === 'number' && updateSetting('maxRetryInterval', '/max-retry-interval', val, setMaxRetryInterval)}
            />
            <ConfigSetting
              title="单请求最多尝试凭证数"
              description="限制一次失败请求最多切换多少个凭证；0 表示沿用 SDK 默认行为。"
              type="number"
              loading={loading || togglesLoading['maxRetryCredentials']}
              value={maxRetryCredentials}
              onChange={(val) => typeof val === 'number' && updateSDKExtraSetting('maxRetryCredentials', { maxRetryCredentials: val }, () => setMaxRetryCredentials(val))}
            />
            <ConfigSetting 
              title="全局代理层 URL" 
              description="让网关通过指定的 HTTP/SOCKS5 代理收发国外请求，留空表示直连。(例: http://127.0.0.1:7890)" 
              type="string"
              loading={loading || togglesLoading['proxyUrl']} 
              value={proxyUrl} 
              onChange={(val) => {
                if (typeof val === 'string') {
                  setProxyUrl(val)
                }
              }}
              onStringBlur={(v) => void updateSetting('proxyUrl', '/proxy-url', v, setProxyUrl)}
            />
          </CardContent>
        </Card>
        
        <Card>
          <CardHeader>
            <CardTitle>负载与安全模型</CardTitle>
            <CardDescription>账号池路由算法及强制模型前缀策略。</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <ConfigSetting 
              title="路由池策略" 
              description="管理含有多个上游 API 渠道时的调用分布。" 
              type="select"
              options={[...ROUTING_STRATEGY_OPTIONS]}
              loading={loading || togglesLoading['routingStrategy']} 
              value={routingStrategy} 
              onChange={(val: string) => updateSetting('routingStrategy', '/routing/strategy', val, setRoutingStrategy)} 
            />
            <ConfigSetting
              title="会话粘滞路由"
              description="启用后同一会话尽量固定到同一凭证，凭证不可用时仍会自动故障转移。"
              loading={loading || togglesLoading['sessionAffinity']}
              value={sessionAffinity}
              onChange={(val: boolean) => updateSDKExtraSetting('sessionAffinity', { routing: { sessionAffinity: val } }, () => setSessionAffinity(val))}
            />
            <ConfigSetting
              title="会话粘滞 TTL"
              description="会话到凭证绑定的保留时间，例如 30m、1h、2h30m。"
              type="string"
              loading={loading || togglesLoading['sessionAffinityTtl']}
              value={sessionAffinityTtl}
              onChange={(val: string) => setSessionAffinityTtl(val)}
              onStringBlur={(v) => {
                const t = v.trim()
                void updateSDKExtraSetting(
                  'sessionAffinityTtl',
                  { routing: { sessionAffinityTtl: t } },
                  () => setSessionAffinityTtl(t),
                )
              }}
            />
            <ConfigSetting 
              title="WebSocket 鉴权" 
              description="是否对 WebSocket (如 Claude Sonnet / Vertex) 进行流式鉴权拦截。" 
              loading={loading || togglesLoading['wsAuth']} 
              value={wsAuth} 
              onChange={(val: boolean) => updateSetting('wsAuth', '/ws-auth', val, setWsAuth)} 
            />
            <ConfigSetting 
              title="强制附着模型前缀" 
              description="强行让下游调用时以特定的渠道名前缀限定上层请求归属。可能影响多渠道并发的兼容性。" 
              loading={loading || togglesLoading['forcePrefix']} 
              value={forceModelPrefix} 
              onChange={(val: boolean) => updateSetting('forcePrefix', '/force-model-prefix', val, setForceModelPrefix)} 
            />
            <ConfigSetting
              title="SDK 用量统计"
              description="启用 SDK 内存和 Redis 队列层面的用量聚合；CPA 计费用量仍由本项目数据库记录。"
              loading={loading || togglesLoading['usageStats']}
              value={usageStatsEnabled}
              onChange={(val: boolean) => updateSetting('usageStats', '/usage-statistics-enabled', val, setUsageStatsEnabled)}
            />
            <ConfigSetting
              title="Redis 用量队列保留秒数"
              description="控制 SDK Redis RESP 用量队列在内存中的保留时间，范围 1-3600 秒。"
              type="number"
              max={3600}
              loading={loading || togglesLoading['redisRetention']}
              value={redisRetention}
              onChange={(val) => typeof val === 'number' && updateSDKExtraSetting('redisRetention', { redisUsageQueueRetentionSeconds: val }, () => setRedisRetention(Math.min(3600, Math.max(1, Math.trunc(val)))))}
            />
            <ConfigSetting
              title="图片生成开关"
              description="off 为启用；all 禁用全部图片生成；chat 仅禁用聊天/响应接口内的图片生成注入。"
              type="select"
              options={[
                { label: '启用图片生成', value: 'off' },
                { label: '完全禁用', value: 'all' },
                { label: '仅禁用聊天注入', value: 'chat' },
              ]}
              loading={loading || togglesLoading['disableImageGeneration']}
              value={disableImageGeneration}
              onChange={(val: DisableImageGenerationMode) => updateSDKExtraSetting('disableImageGeneration', { disableImageGeneration: val }, () => setDisableImageGeneration(val))}
            />
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
