import { lazy, Suspense } from 'react'
import { Network, Monitor } from 'lucide-react'
import { useSearchParams, Navigate } from 'react-router-dom'
import { cn } from '@/shared/utils/utils'
import { adminCredentialsTab } from '@/shared/routes/admin'

const AdminProxyProvidersPage = lazy(() => import('./AdminProxyProvidersPage'))
const AdminProxyAmpcodePage = lazy(() => import('./AdminProxyAmpcodePage'))

const tabs = [
  {
    id: 'providers',
    label: 'API 密钥池',
    tag: '多渠道负载',
    icon: Network,
    description: '管理各大模型服务商的 API 密钥池、自定义网关地址与故障转移策略。',
  },
  {
    id: 'ampcode',
    label: 'Ampcode 专线',
    tag: '专用网关',
    icon: Monitor,
    description: '配置 Ampcode 专用上游渠道映射、SDK 密钥路由与模型重映射规则。',
  },
]

export default function AdminProxyChannelsPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const rawTab = searchParams.get('tab')

  // Smooth redirects for legacy tab parameters to credentials page
  if (rawTab === 'oauth') {
    return <Navigate to={adminCredentialsTab('oauth')} replace />
  }
  if (rawTab === 'credentials') {
    return <Navigate to={adminCredentialsTab('sessions')} replace />
  }

  const initialTab = rawTab || 'providers'
  const activeTab = tabs.some(t => t.id === initialTab) ? initialTab : 'providers'

  const handleTabChange = (value: string) => {
    setSearchParams({ tab: value }, { replace: true })
  }

  const ActiveComponent =
    activeTab === 'ampcode' ? AdminProxyAmpcodePage : AdminProxyProvidersPage

  return (
    <div className="mx-auto max-w-7xl space-y-6">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div className="space-y-1">
          <div className="flex items-center gap-2.5 flex-wrap">
            <div className="p-2 rounded-xl bg-primary/10 text-primary border border-primary/20 shadow-xs">
              <Network className="h-5 w-5" />
            </div>
            <h1 className="text-xl sm:text-2xl font-bold tracking-tight text-foreground">
              渠道管理
            </h1>
            <span className="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
              多账号故障转移
            </span>
            <span className="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-blue-500/10 text-blue-600 dark:text-blue-400 border border-blue-500/20">
              自动负载均衡
            </span>
          </div>
          <p className="text-xs sm:text-sm text-muted-foreground">
            统一调度 OpenAI、Claude、Gemini 等各大主流大模型服务的接入点、API 密钥池与专用网关。
          </p>
        </div>
      </div>

      <div className="flex flex-col gap-6">
        {/* Navigation Tabs */}
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 w-full">
          {tabs.map((tab) => {
            const isActive = activeTab === tab.id
            const Icon = tab.icon

            return (
              <button
                key={tab.id}
                onClick={() => handleTabChange(tab.id)}
                className={cn(
                  "text-left p-3.5 sm:p-4 rounded-xl transition-all border flex items-start gap-3.5 cursor-pointer group relative overflow-hidden",
                  isActive 
                    ? "bg-card border-primary/60 ring-1 ring-primary/25 text-foreground shadow-sm" 
                    : "bg-card/60 border-border/70 hover:border-border hover:bg-card text-muted-foreground hover:text-foreground"
                )}
              >
                <div className={cn(
                  "p-2 rounded-lg transition-colors shrink-0 mt-0.5",
                  isActive 
                    ? "bg-primary text-primary-foreground shadow-xs" 
                    : "bg-muted text-muted-foreground group-hover:text-foreground group-hover:bg-muted/80"
                )}>
                  <Icon className="h-4 w-4" />
                </div>
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex items-center gap-2">
                    <span className={cn(
                      "font-semibold text-sm",
                      isActive ? "text-foreground" : "text-foreground/90 group-hover:text-foreground"
                    )}>
                      {tab.label}
                    </span>
                    <span className={cn(
                      "text-[10px] px-1.5 py-0.2 rounded font-medium",
                      isActive
                        ? "bg-primary/10 text-primary border border-primary/20"
                        : "bg-muted text-muted-foreground"
                    )}>
                      {tab.tag}
                    </span>
                  </div>
                  <p className="text-xs leading-relaxed text-muted-foreground line-clamp-1">
                    {tab.description}
                  </p>
                </div>
                {isActive && (
                  <div className="w-1.5 h-1.5 rounded-full bg-primary shrink-0 self-center" />
                )}
              </button>
            )
          })}
        </div>

        <div key={activeTab} className="w-full">
          <Suspense fallback={<TabFallback />}>
            <ActiveComponent />
          </Suspense>
        </div>
      </div>
    </div>
  )
}

function TabFallback() {
  return (
    <div className="space-y-4 py-6" role="status" aria-label="加载中">
      <div className="h-12 rounded-xl bg-muted/60 animate-pulse" />
      <div className="h-36 rounded-xl bg-muted/40 animate-pulse" />
      <div className="h-48 rounded-xl bg-muted/30 animate-pulse" />
    </div>
  )
}
