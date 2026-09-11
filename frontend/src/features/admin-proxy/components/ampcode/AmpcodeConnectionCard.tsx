import { useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card'
import { Input } from '@/shared/components/ui/input'
import { Label } from '@/shared/components/ui/label'
import { Button } from '@/shared/components/ui/button'
import { Badge } from '@/shared/components/ui/badge'
import {
  Globe,
  KeyRound,
  Eye,
  EyeOff,
  Copy,
  Check,
  PlugZap,
  Loader2,
  CheckCircle2,
  AlertCircle,
  XCircle,
  RotateCcw,
  Save,
  ChevronDown,
  ChevronUp,
  Sparkles,
} from 'lucide-react'
import { toast } from 'sonner'
import { copyTextToClipboard } from '@/shared/utils/clipboard'
import { validateUpstreamUrl } from '../../ampcodeConfig'
import type { AmpcodeUpstreamTestResult } from '../../ampcodeUpstreamTest'
import { cn } from '@/shared/utils/utils'

interface AmpcodeConnectionCardProps {
  upstreamUrl: string
  setUpstreamUrl: (url: string) => void
  upstreamApiKey: string
  setUpstreamApiKey: (key: string) => void
  savedUrl: string
  savedApiKey: string
  onSaveConnection: () => Promise<void>
  onDiscardConnection: () => void
  saving: boolean
  testing: boolean
  onTestConnection: (allowFallback: boolean) => Promise<void>
  testResult: AmpcodeUpstreamTestResult | null
}

export function AmpcodeConnectionCard({
  upstreamUrl,
  setUpstreamUrl,
  upstreamApiKey,
  setUpstreamApiKey,
  savedUrl,
  savedApiKey,
  onSaveConnection,
  onDiscardConnection,
  saving,
  testing,
  onTestConnection,
  testResult,
}: AmpcodeConnectionCardProps) {
  const [showApiKey, setShowApiKey] = useState(false)
  const [copiedKey, setCopiedKey] = useState(false)
  const [copiedEndpoint, setCopiedEndpoint] = useState(false)
  const [showDetails, setShowDetails] = useState(false)
  const [allowFallback, setAllowFallback] = useState(true)

  const isDirty = upstreamUrl !== savedUrl || upstreamApiKey !== savedApiKey
  const urlValidation = validateUpstreamUrl(upstreamUrl)

  const handleCopyApiKey = async () => {
    if (!upstreamApiKey) return
    await copyTextToClipboard(upstreamApiKey)
    setCopiedKey(true)
    toast.success('上游 API Key 已复制到剪贴板')
    setTimeout(() => setCopiedKey(false), 2000)
  }

  const handleCopyEndpoint = async (url: string) => {
    if (!url) return
    await copyTextToClipboard(url)
    setCopiedEndpoint(true)
    toast.success('探测端点已复制')
    setTimeout(() => setCopiedEndpoint(false), 2000)
  }

  const testResultMeta = (() => {
    if (!testResult) return null
    if (testResult.status === 'connected') {
      return {
        label: '连接正常',
        icon: CheckCircle2,
        badgeVariant: 'success' as const,
        cardClass: 'border-emerald-200 bg-emerald-50/50 text-emerald-950 dark:border-emerald-800/40 dark:bg-emerald-950/20 dark:text-emerald-200',
      }
    }
    if (testResult.status === 'reachable') {
      return {
        label: '网络可达 (非标准响应)',
        icon: AlertCircle,
        badgeVariant: 'warning' as const,
        cardClass: 'border-amber-200 bg-amber-50/50 text-amber-950 dark:border-amber-800/40 dark:bg-amber-950/20 dark:text-amber-200',
      }
    }
    return {
      label: '连接失败',
      icon: XCircle,
      badgeVariant: 'destructive' as const,
      cardClass: 'border-red-200 bg-red-50/50 text-red-950 dark:border-red-800/40 dark:bg-red-950/20 dark:text-red-200',
    }
  })()
  const ResultIcon = testResultMeta?.icon

  return (
    <div className="space-y-6">
      {/* Configuration Card */}
      <Card className="border-border/80 shadow-2xs">
        <CardHeader className="pb-4">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2">
            <CardTitle className="text-base sm:text-lg font-semibold flex items-center gap-2">
              <Globe className="h-5 w-5 text-primary" />
              <span>专线上游服务连接</span>
            </CardTitle>
            {isDirty ? (
              <Badge variant="warning" className="w-fit">
                ● 存在未保存的修改
              </Badge>
            ) : (
              <Badge variant="outline" className="w-fit text-emerald-600 dark:text-emerald-400 border-emerald-500/30">
                ✓ 配置已与服务器同步
              </Badge>
            )}
          </div>
          <CardDescription className="text-xs sm:text-sm">
            设置下级 Ampcode 服务的网关 URL 与访问凭据。连接保存后，专线流量将自动定向至此节点。
          </CardDescription>
        </CardHeader>

        <CardContent className="space-y-5">
          <div className="flex flex-col gap-4 p-4 rounded-xl border border-border/70 bg-muted/20">
            {/* Upstream URL Input */}
            <div className="grid gap-1.5">
              <div className="flex items-center justify-between">
                <Label className="text-xs font-semibold text-foreground/90 flex items-center gap-1.5">
                  <span>Upstream Gateway URL (上游地址)</span>
                  <span className="text-destructive">*</span>
                </Label>
                {urlValidation.suggestedUrl && (
                  <button
                    type="button"
                    onClick={() => setUpstreamUrl(urlValidation.suggestedUrl!)}
                    className="inline-flex items-center gap-1 text-[11px] text-primary hover:underline cursor-pointer font-medium"
                  >
                    <Sparkles className="h-3 w-3" />
                    建议补全协议: {urlValidation.suggestedUrl}
                  </button>
                )}
              </div>
              <div className="relative">
                <Input
                  className={cn(
                    'bg-background shadow-2xs font-mono text-xs sm:text-sm h-9.5 pr-8',
                    !urlValidation.valid && upstreamUrl ? 'border-destructive focus-visible:ring-destructive' : '',
                  )}
                  value={upstreamUrl}
                  onChange={e => setUpstreamUrl(e.target.value)}
                  placeholder="https://ampcode.com 或 https://your-gateway.internal"
                />
              </div>
              {!urlValidation.valid && upstreamUrl && (
                <p className="text-[11px] text-destructive flex items-center gap-1 mt-0.5">
                  <AlertCircle className="h-3 w-3" />
                  {urlValidation.message}
                </p>
              )}
            </div>

            {/* Upstream API Key Input */}
            <div className="grid gap-1.5">
              <div className="flex items-center justify-between">
                <Label className="text-xs font-semibold text-foreground/90 flex items-center gap-1.5">
                  <KeyRound className="h-3.5 w-3.5 text-primary" />
                  <span>Upstream API Key (专线访问密钥)</span>
                  <span className="text-destructive">*</span>
                </Label>
                <span className="text-[11px] text-muted-foreground">
                  通常以 <code className="font-mono text-foreground/80">sgamp_user_...</code> 开头
                </span>
              </div>
              <div className="relative flex items-center">
                <Input
                  type={showApiKey ? 'text' : 'password'}
                  className="bg-background shadow-2xs font-mono text-xs sm:text-sm h-9.5 pr-20"
                  value={upstreamApiKey}
                  onChange={e => setUpstreamApiKey(e.target.value)}
                  placeholder="sgamp_user_xxxxxxxxxxxxxx"
                />
                <div className="absolute right-1.5 flex items-center gap-1">
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 text-muted-foreground hover:text-foreground"
                    onClick={() => setShowApiKey(!showApiKey)}
                    title={showApiKey ? '隐藏密钥' : '显示密钥'}
                  >
                    {showApiKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 text-muted-foreground hover:text-foreground"
                    onClick={handleCopyApiKey}
                    disabled={!upstreamApiKey}
                    title="复制密钥"
                  >
                    {copiedKey ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                  </Button>
                </div>
              </div>
            </div>

            {/* Save & Reset Actions Bar */}
            <div className="flex flex-wrap items-center justify-between gap-3 pt-2 border-t border-border/60">
              <div className="text-xs text-muted-foreground">
                {isDirty ? '修改尚未保存至持久化存储' : '当前设置与后台数据库保持一致'}
              </div>
              <div className="flex items-center gap-2">
                {isDirty && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8.5 px-3 text-xs gap-1.5 cursor-pointer shadow-2xs"
                    onClick={onDiscardConnection}
                    disabled={saving}
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                    放弃修改
                  </Button>
                )}
                <Button
                  size="sm"
                  className="h-8.5 px-3.5 text-xs gap-1.5 cursor-pointer shadow-2xs"
                  onClick={onSaveConnection}
                  disabled={saving || !isDirty}
                >
                  {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
                  保存连接配置
                </Button>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Connectivity & Diagnostic Card */}
      <Card className="border-border/80 shadow-2xs">
        <CardHeader className="pb-3">
          <CardTitle className="text-base font-semibold flex items-center gap-2">
            <PlugZap className="h-4.5 w-4.5 text-primary" />
            <span>连通性与时延即时诊断</span>
          </CardTitle>
          <CardDescription className="text-xs sm:text-sm">
            使用输入框中的配置发起只读网络探测，验证网关地址解析、控制面可用性及认证有效性。
          </CardDescription>
        </CardHeader>

        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 p-3.5 rounded-xl border border-border/70 bg-muted/20">
            <div className="flex items-center gap-2">
              <input
                id="allow-fallback"
                type="checkbox"
                checked={allowFallback}
                onChange={e => setAllowFallback(e.target.checked)}
                className="rounded border-border text-primary focus:ring-primary h-4 w-4 cursor-pointer"
              />
              <label htmlFor="allow-fallback" className="text-xs text-foreground/90 cursor-pointer select-none">
                若控制面 (/api/user) 404 时自动回退探测模型列表 (/v1/models)
              </label>
            </div>

            <Button
              variant="outline"
              size="sm"
              className="h-8.5 px-3.5 text-xs gap-1.5 cursor-pointer shadow-2xs"
              onClick={() => onTestConnection(allowFallback)}
              disabled={testing || !upstreamUrl.trim() || !upstreamApiKey.trim()}
            >
              {testing ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  <span>探测中...</span>
                </>
              ) : (
                <>
                  <PlugZap className="h-3.5 w-3.5 text-primary" />
                  <span>发起连通性测试</span>
                </>
              )}
            </Button>
          </div>

          {/* Diagnostic Result Block */}
          {testResult && testResultMeta && (
            <div className={cn('rounded-xl border p-4 text-xs sm:text-sm shadow-2xs space-y-3', testResultMeta.cardClass)}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2 flex-wrap">
                  <Badge variant={testResultMeta.badgeVariant} className="gap-1.5 px-2.5 py-1">
                    {ResultIcon && <ResultIcon className="h-4 w-4" />}
                    <span>{testResultMeta.label}</span>
                  </Badge>

                  {typeof testResult.statusCode === 'number' && (
                    <span className="font-mono text-xs px-2 py-0.5 rounded-md bg-background/80 border border-current/20 font-semibold">
                      HTTP {testResult.statusCode}
                    </span>
                  )}

                  {typeof testResult.elapsedMs === 'number' && (
                    <span
                      className={cn(
                        'font-mono text-xs px-2 py-0.5 rounded-md border font-semibold',
                        testResult.isLatencyHealthy
                          ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 border-emerald-500/30'
                          : 'bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-500/30',
                      )}
                    >
                      {testResult.elapsedMs} ms
                    </span>
                  )}

                  {testResult.diagnosticStep && (
                    <span className="font-mono text-[11px] px-2 py-0.5 rounded bg-muted/60 text-muted-foreground border border-border/50">
                      阶段: {testResult.diagnosticStep}
                    </span>
                  )}
                </div>

                <div className="text-[11px] text-muted-foreground font-mono">
                  {new Date(testResult.checkedAt).toLocaleTimeString()}
                </div>
              </div>

              <p className="text-xs leading-relaxed font-medium">{testResult.message}</p>

              {testResult.endpoint && (
                <div className="flex items-center justify-between gap-2 p-2 rounded-lg bg-background/60 border border-current/15">
                  <div className="flex items-center gap-1.5 overflow-hidden">
                    <span className="text-[11px] text-muted-foreground shrink-0">探测目标:</span>
                    <span className="font-mono text-[11px] truncate select-all">{testResult.endpoint}</span>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
                    onClick={() => handleCopyEndpoint(testResult.endpoint)}
                    title="复制端点"
                  >
                    {copiedEndpoint ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                  </Button>
                </div>
              )}

              {/* Collapsible Detailed Response Preview */}
              {testResult.bodyPreview && (
                <div className="space-y-1.5">
                  <button
                    type="button"
                    onClick={() => setShowDetails(!showDetails)}
                    className="flex items-center gap-1 text-[11px] font-semibold text-foreground/80 hover:text-foreground cursor-pointer"
                  >
                    {showDetails ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
                    <span>{showDetails ? '收起响应详情' : '展开响应详情与预览'}</span>
                  </button>

                  {showDetails && (
                    <div className="rounded-lg bg-background/90 p-3 border border-border/70 font-mono text-[11px] space-y-2 overflow-x-auto">
                      <div>
                        <span className="text-muted-foreground">Response Body Preview:</span>
                        <pre className="mt-1 whitespace-pre-wrap break-all text-foreground/90 max-h-48 overflow-y-auto">
                          {testResult.bodyPreview}
                        </pre>
                      </div>

                      {testResult.headersPreview && Object.keys(testResult.headersPreview).length > 0 && (
                        <div className="pt-2 border-t border-border/50">
                          <span className="text-muted-foreground">Response Headers:</span>
                          <div className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 mt-1 text-[10px]">
                            {Object.entries(testResult.headersPreview).map(([k, v]) => (
                              <div key={k} className="contents">
                                <span className="text-muted-foreground font-semibold">{k}:</span>
                                <span className="truncate">{v}</span>
                              </div>
                            ))}
                          </div>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
