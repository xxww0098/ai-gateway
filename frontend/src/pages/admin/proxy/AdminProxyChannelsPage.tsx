import { Network, KeyRound, Shield, Monitor } from 'lucide-react'
import { useSearchParams } from 'react-router-dom'
import { cn } from '@/shared/utils/utils'

// Import existing page components
import AdminProxyProvidersPage from './AdminProxyProvidersPage'
import AdminProxyOAuthPage from './AdminProxyOAuthPage'
import AdminProxyAuthFilesPage from './AdminProxyAuthFilesPage'
import AdminProxyAmpcodePage from './AdminProxyAmpcodePage'

const tabs = [
  { id: 'providers', label: 'API 密钥池', icon: Network, description: '管理上游各个模型服务的固定 API key 凭证，适用于常规 API 接入。' },
  { id: 'oauth', label: 'OAuth 登录', icon: KeyRound, description: '通过浏览器执行 OAuth 登录流程，获取并自动刷新上游 Token。' },
  { id: 'credentials', label: '凭证会话', icon: Shield, description: '查看网关系统当前持有的所有底层认证文件和活跃凭证状态。' },
  { id: 'ampcode', label: 'Ampcode', icon: Monitor, description: '配置 Ampcode 上游连接渠道、模型黑白名单与渠道映射规则。' },
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
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">渠道与凭证管理</h2>
        <p className="text-gray-500 dark:text-dark-300 mt-2 text-sm max-w-3xl">
          配置并管理通向不同 AI 供应商的连接。您可以通过静态密钥、OAuth 认证或者 Ampcode 插件等方式接入上游算力，并在此集中管理各类认证凭证。
        </p>
      </div>

      <div className="flex flex-col gap-6">
        {/* Topbar Navigation */}
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 w-full">
          {tabs.map((tab) => {
            const isActive = activeTab === tab.id
            const Icon = tab.icon

            return (
              <button
                key={tab.id}
                onClick={() => handleTabChange(tab.id)}
                className={cn(
                  "group text-left p-3.5 rounded-xl transition-all duration-200 border relative overflow-hidden flex flex-col gap-1.5",
                  isActive 
                    ? "bg-white dark:bg-dark-800 border-indigo-500/30 dark:border-indigo-400/30 shadow-sm ring-1 ring-indigo-500/10 dark:ring-indigo-400/10" 
                    : "bg-white/50 dark:bg-dark-800/50 border-gray-200/60 dark:border-dark-700/60 hover:bg-white dark:hover:bg-dark-800 hover:border-gray-300 dark:hover:border-dark-600 hover:shadow-sm"
                )}
              >
                {isActive && (
                  <div className="absolute top-0 left-0 right-0 h-1 bg-indigo-500 dark:bg-indigo-400 rounded-t-xl" />
                )}
                <div className="flex items-center justify-between w-full">
                  <div className="flex items-center gap-2.5">
                    <div className={cn(
                      "p-1.5 rounded-lg transition-colors",
                      isActive 
                        ? "bg-indigo-50 dark:bg-indigo-500/10 text-indigo-600 dark:text-indigo-400" 
                        : "bg-gray-100 dark:bg-dark-700 text-gray-500 dark:text-dark-300 group-hover:bg-gray-200 dark:group-hover:bg-dark-600"
                    )}>
                      <Icon className="h-3.5 w-3.5" />
                    </div>
                    <span className={cn(
                      "font-semibold text-sm",
                      isActive ? "text-indigo-900 dark:text-indigo-200" : "text-gray-700 dark:text-gray-300 group-hover:text-gray-900 dark:group-hover:text-gray-100"
                    )}>
                      {tab.label}
                    </span>
                  </div>
                </div>
                
                <p className={cn(
                  "text-xs leading-snug line-clamp-2",
                  isActive ? "text-gray-600 dark:text-dark-200" : "text-gray-500 dark:text-dark-400"
                )}>
                  {tab.description}
                </p>
              </button>
            )
          })}
        </div>

        {/* Selected Content */}
        <div className="w-full bg-white dark:bg-dark-800 rounded-2xl border border-gray-200/60 dark:border-dark-700/60 shadow-sm overflow-hidden p-6 text-left">
          <div key={activeTab} className="animate-in fade-in duration-300" style={{ willChange: 'opacity' }}>
            <ActiveComponent />
          </div>
        </div>
      </div>
    </div>
  )
}
