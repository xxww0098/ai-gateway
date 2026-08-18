import { Network, KeyRound, Shield, Monitor } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'
import { cn } from '@/shared/utils/utils'

// Import existing page components
import AdminProxyProvidersPage from './AdminProxyProvidersPage'
import AdminProxyOAuthPage from './AdminProxyOAuthPage'
import AdminProxyAuthFilesPage from './AdminProxyAuthFilesPage'
import AdminProxyAmpcodePage from './AdminProxyAmpcodePage'

const tabs = [
  { id: 'providers', label: 'API 密钥池', icon: Network, description: '管理上游模型服务的 API 密钥池与负载均衡策略。' },
  { id: 'oauth', label: 'OAuth 登录', icon: KeyRound, description: '通过 OAuth 授权连接上游服务，支持自动刷新凭证。' },
  { id: 'credentials', label: '凭证会话', icon: Shield, description: '管理底层持久化的凭证文件、OAuth 活跃会话与状态。' },
  { id: 'ampcode', label: 'Ampcode', icon: Monitor, description: '配置 Ampcode 专用上游渠道映射与模型访问规则。' },
]

export default function AdminProxyChannelsPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const initialTab = searchParams.get('tab') || 'providers'
  // Ensure valid initial tab
  const activeTab = tabs.some(t => t.id === initialTab) ? initialTab : 'providers'

  const handleTabChange = (value: string) => {
    setSearchParams({ tab: value }, { replace: true })
  }

  // Active Component
  const ActiveComponent =
    activeTab === 'providers' ? AdminProxyProvidersPage :
    activeTab === 'oauth' ? AdminProxyOAuthPage :
    activeTab === 'credentials' ? AdminProxyAuthFilesPage :
    activeTab === 'ampcode' ? AdminProxyAmpcodePage :
    AdminProxyProvidersPage

  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500 max-w-7xl mx-auto">
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">渠道管理</h2>
        <p className="text-gray-500 dark:text-dark-300 mt-2 text-sm max-w-3xl">
          配置并管理上游 AI 提供商的连接渠道，支持 API 密钥池、OAuth 会话与 Ampcode 插件代理。
        </p>
      </div>

      <div className="flex flex-col gap-6">
        {/* Navigation Tabs */}
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3 w-full">
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
                    ? "bg-card border-primary ring-1 ring-primary/20 text-foreground" 
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

        {/* Tab Content */}
        <div key={activeTab} className="w-full">
          <ActiveComponent />
        </div>
      </div>
    </div>
  )
}
