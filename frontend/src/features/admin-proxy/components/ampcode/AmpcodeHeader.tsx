import { RefreshCw, Code2, Server, ShieldCheck, Zap, GitFork, KeyRound } from 'lucide-react'
import { Button } from '@/shared/components/ui/button'
import { Badge } from '@/shared/components/ui/badge'
import type { AmpcodeUpstreamTestResult } from '../../ampcodeUpstreamTest'

interface AmpcodeHeaderProps {
  loading: boolean
  onRefresh: () => void
  onOpenJsonDialog: () => void
  upstreamConfigured: boolean
  upstreamTestResult: AmpcodeUpstreamTestResult | null
  forceModelMappings: boolean
  modelMappingsCount: number
  regexMappingsCount: number
  upstreamKeyEntriesCount: number
  totalClientKeysCount: number
}

export function AmpcodeHeader({
  loading,
  onRefresh,
  onOpenJsonDialog,
  upstreamConfigured,
  upstreamTestResult,
  forceModelMappings,
  modelMappingsCount,
  regexMappingsCount,
  upstreamKeyEntriesCount,
  totalClientKeysCount,
}: AmpcodeHeaderProps) {
  const getStatusBadge = () => {
    if (!upstreamConfigured) {
      return <Badge variant="secondary">未配置上游</Badge>
    }
    if (!upstreamTestResult) {
      return <Badge variant="outline">就绪 (待测试)</Badge>
    }
    if (upstreamTestResult.status === 'connected') {
      return <Badge variant="success">服务连通正常</Badge>
    }
    if (upstreamTestResult.status === 'reachable') {
      return <Badge variant="warning">上游网络可达</Badge>
    }
    return <Badge variant="destructive">连接异常</Badge>
  }

  return (
    <div className="space-y-4">
      {/* Top Hero Banner */}
      <div className="relative overflow-hidden rounded-2xl border border-border/80 bg-gradient-to-r from-card via-card to-primary/5 p-5 sm:p-6 shadow-xs">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
          <div className="flex items-start gap-4">
            <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-primary/10 text-primary border border-primary/20 shadow-xs">
              <Server className="h-6 w-6" />
            </div>
            <div className="space-y-1">
              <div className="flex items-center gap-2.5 flex-wrap">
                <h2 className="text-lg sm:text-xl font-bold tracking-tight text-foreground">
                  Ampcode 专线控制面板
                </h2>
                {getStatusBadge()}
                {upstreamTestResult?.elapsedMs !== undefined && (
                  <span className="font-mono text-xs px-2 py-0.5 rounded-md bg-muted text-muted-foreground border border-border/60">
                    {upstreamTestResult.elapsedMs} ms
                  </span>
                )}
              </div>
              <p className="text-xs sm:text-sm text-muted-foreground max-w-2xl leading-relaxed">
                专用于对接下级 Ampcode 控制面与中转网络。支持网络延时与连通性自测、SDK 客户端 Key 动态分发路由及模型重定向策略。
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2 self-start sm:self-center shrink-0">
            <Button
              variant="outline"
              size="sm"
              onClick={onRefresh}
              disabled={loading}
              className="h-8.5 text-xs gap-1.5 cursor-pointer shadow-2xs"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
              刷新
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={onOpenJsonDialog}
              className="h-8.5 text-xs gap-1.5 cursor-pointer shadow-2xs"
            >
              <Code2 className="h-3.5 w-3.5 text-muted-foreground" />
              原始配置 (JSON)
            </Button>
          </div>
        </div>
      </div>

      {/* KPI Stats Bar */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        {/* Card 1: Upstream Link */}
        <div className="rounded-xl border border-border/70 bg-card/80 p-3.5 shadow-2xs space-y-1">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium">专线上游状态</span>
            <Zap className="h-3.5 w-3.5 text-primary" />
          </div>
          <div className="text-base sm:text-lg font-bold text-foreground">
            {upstreamConfigured ? (
              upstreamTestResult?.status === 'connected' ? (
                <span className="text-emerald-600 dark:text-emerald-400">活跃</span>
              ) : (
                <span>已配置</span>
              )
            ) : (
              <span className="text-muted-foreground">未配置</span>
            )}
          </div>
          <div className="text-[11px] text-muted-foreground truncate">
            {upstreamTestResult?.statusCode ? `HTTP ${upstreamTestResult.statusCode}` : '未执行即时诊断'}
          </div>
        </div>

        {/* Card 2: Force Mapping Switch */}
        <div className="rounded-xl border border-border/70 bg-card/80 p-3.5 shadow-2xs space-y-1">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium">调度策略</span>
            <ShieldCheck className="h-3.5 w-3.5 text-primary" />
          </div>
          <div className="text-base sm:text-lg font-bold text-foreground">
            {forceModelMappings ? (
              <span className="text-amber-600 dark:text-amber-400">强制重映射</span>
            ) : (
              <span>故障旁路</span>
            )}
          </div>
          <div className="text-[11px] text-muted-foreground truncate">
            {forceModelMappings ? '始终优先使用重定向规则' : '原渠道故障时兜底转发'}
          </div>
        </div>

        {/* Card 3: Model Rules */}
        <div className="rounded-xl border border-border/70 bg-card/80 p-3.5 shadow-2xs space-y-1">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium">模型重定向规则</span>
            <GitFork className="h-3.5 w-3.5 text-primary" />
          </div>
          <div className="text-base sm:text-lg font-bold text-foreground">
            {modelMappingsCount} <span className="text-xs font-normal text-muted-foreground">条</span>
          </div>
          <div className="text-[11px] text-muted-foreground truncate">
            {regexMappingsCount > 0 ? `含 ${regexMappingsCount} 条正则规则` : '全为精确匹配'}
          </div>
        </div>

        {/* Card 4: SDK Key Routes */}
        <div className="rounded-xl border border-border/70 bg-card/80 p-3.5 shadow-2xs space-y-1">
          <div className="flex items-center justify-between text-muted-foreground">
            <span className="text-xs font-medium">SDK 密钥路由</span>
            <KeyRound className="h-3.5 w-3.5 text-primary" />
          </div>
          <div className="text-base sm:text-lg font-bold text-foreground">
            {upstreamKeyEntriesCount} <span className="text-xs font-normal text-muted-foreground">个上游 Key</span>
          </div>
          <div className="text-[11px] text-muted-foreground truncate">
            已绑定 {totalClientKeysCount} 个客户端 Key
          </div>
        </div>
      </div>
    </div>
  )
}
