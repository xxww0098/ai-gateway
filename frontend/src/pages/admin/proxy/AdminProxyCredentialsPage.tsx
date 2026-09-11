import { lazy, Suspense } from 'react'
import { ShieldCheck, KeyRound } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'
import { cn } from '@/shared/utils/utils'

const AdminProxyAuthFilesPage = lazy(() => import('./AdminProxyAuthFilesPage'))
const AdminProxyOAuthPage = lazy(() => import('./AdminProxyOAuthPage'))

const tabs = [
  {
    id: 'sessions',
    label: '凭证资产',
    icon: ShieldCheck,
    description: '管理底层持久化的凭证文件、OAuth 活跃会话、配额与健康状态。',
  },
  {
    id: 'oauth',
    label: 'OAuth 接入',
    icon: KeyRound,
    description: '通过 OAuth 授权连接上游服务（Gemini/Claude/Codex等），支持设备码与自动刷新。',
  },
]

export default function AdminProxyCredentialsPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const rawTab = searchParams.get('tab')
  // Map legacy 'credentials' query param to 'sessions'
  const initialTab = rawTab === 'credentials' ? 'sessions' : rawTab || 'sessions'
  const activeTab = tabs.some(t => t.id === initialTab) ? initialTab : 'sessions'

  const handleTabChange = (value: string) => {
    setSearchParams({ tab: value }, { replace: true })
  }

  const ActiveComponent =
    activeTab === 'oauth' ? AdminProxyOAuthPage : AdminProxyAuthFilesPage

  return (
    <div className="mx-auto max-w-7xl space-y-6">
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
                  "text-left p-3.5 rounded-xl transition-all border flex flex-col gap-1.5 cursor-pointer",
                  isActive 
                    ? "bg-card border-primary ring-1 ring-primary/20 text-foreground shadow-xs" 
                    : "bg-card/50 border-border hover:border-gray-300 dark:hover:border-dark-600 text-muted-foreground hover:text-foreground"
                )}
              >
                <div className="flex items-center gap-2">
                  <div className={cn(
                    "p-1.5 rounded-lg transition-colors",
                    isActive 
                      ? "bg-primary/10 text-primary" 
                      : "bg-muted text-muted-foreground"
                  )}>
                    <Icon className="h-4 w-4" />
                  </div>
                  <span className="font-semibold text-sm text-foreground">
                    {tab.label}
                  </span>
                </div>
                
                <p className="text-xs leading-relaxed text-muted-foreground line-clamp-2">
                  {tab.description}
                </p>
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
    <div className="space-y-3 py-6" role="status" aria-label="加载中">
      <div className="h-10 rounded-xl bg-muted animate-pulse" />
      <div className="h-10 rounded-xl bg-muted/70 animate-pulse" />
      <div className="h-32 rounded-xl bg-muted/50 animate-pulse" />
    </div>
  )
}
