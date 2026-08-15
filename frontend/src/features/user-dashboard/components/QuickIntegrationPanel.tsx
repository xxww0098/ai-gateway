import { Zap, Archive, Code2, MessageSquare, Terminal } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import type { IntegrationTab, QuickIntegrationPanelProps } from '../types'

const integrationTabs: Array<{ id: IntegrationTab; label: string; Icon: LucideIcon }> = [
  { id: 'openai', label: 'OpenAI 兼容', Icon: Code2 },
  { id: 'anthropic', label: 'Anthropic 原生', Icon: MessageSquare },
  { id: 'amp', label: 'Amp', Icon: Terminal },
]

export function QuickIntegrationPanel({
  apiKeyCount,
  integrationTab,
  onIntegrationTabChange,
}: QuickIntegrationPanelProps) {
  const origin = window.location.origin
  const description = integrationTab === 'openai'
    ? '配置 OpenAI 兼容客户端（如 Cursor、Cline、aider）连接到 CPA Gateway 代理池：'
    : integrationTab === 'anthropic'
      ? '配置 Anthropic 原生客户端（如 Claude Code）连接到 CPA Gateway 代理池：'
      : '配置 Amp CLI 或编辑器扩展使用 CPA Gateway 的 Amp 路由：'

  return (
    <div className="glass-card overflow-hidden flex flex-col relative border-primary-500/20">
      <div className="absolute inset-0 bg-gradient-to-br from-primary-500/5 to-blue-500/5 z-0 pointer-events-none"></div>
      <div className="px-6 py-5 border-b border-primary-500/10 flex items-center gap-2 bg-primary-50/50 dark:bg-primary-900/10 z-10">
        <Zap className="w-5 h-5 text-emerald-500" />
        <h3 className="text-sm font-bold uppercase tracking-wider text-gray-900 dark:text-white">
          快速集成
        </h3>
      </div>
      <div className="p-6 flex-1 z-10">
        {/* Protocol Tabs */}
        <div className="flex flex-wrap gap-1 p-1 mb-5 bg-gray-100 dark:bg-dark-800 rounded-xl w-fit max-w-full">
          {integrationTabs.map(tab => {
            const Icon = tab.Icon
            return (
              <button
                key={tab.id}
                onClick={() => onIntegrationTabChange(tab.id)}
                className={`inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-semibold transition-all ${
                  integrationTab === tab.id
                    ? 'bg-white dark:bg-dark-700 text-gray-900 dark:text-white shadow-sm'
                    : 'text-gray-500 dark:text-dark-400 hover:text-gray-700 dark:hover:text-dark-200'
                }`}
              >
                <Icon className="h-3.5 w-3.5" />
                {tab.label}
              </button>
            )
          })}
        </div>

        <p className="text-sm text-gray-600 dark:text-gray-300 mb-6">
          {description}
        </p>
        <div className="space-y-6">
          {integrationTab === 'amp' ? (
            <>
              <div className="space-y-2">
                <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase tracking-wider">Amp 环境变量</label>
                <div className="whitespace-pre-wrap bg-white dark:bg-dark-900/80 border border-border p-3 rounded-xl font-mono text-sm shadow-sm select-all cursor-text text-primary-600 dark:text-primary-400">
                  {`AMP_URL="${origin}"\nAMP_API_KEY="<YOUR_API_KEY>"`}
                </div>
              </div>
              <div className="space-y-2">
                <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase tracking-wider">VS Code Settings</label>
                <div className="whitespace-pre-wrap bg-white dark:bg-dark-900/80 border border-border p-3 rounded-xl font-mono text-sm shadow-sm select-all cursor-text text-gray-500 dark:text-dark-300">
                  {`{\n  "amp.url": "${origin}"\n}`}
                </div>
              </div>
            </>
          ) : (
            <>
              <div className="space-y-2">
                <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase tracking-wider">Base URL 请求地址</label>
                <div className="flex items-center justify-between bg-white dark:bg-dark-900/80 border border-border p-3 rounded-xl font-mono text-sm shadow-sm select-all cursor-text text-primary-600 dark:text-primary-400">
                  {integrationTab === 'openai'
                    ? `${origin}/v1`
                    : `${origin}/v1beta`}
                </div>
              </div>
              <div className="space-y-2">
                <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase tracking-wider">身份认证标头</label>
                <div className="bg-white dark:bg-dark-900/80 border border-border p-3 rounded-xl font-mono text-sm shadow-sm overflow-x-auto text-gray-500 dark:text-dark-300">
                  {integrationTab === 'openai' ? (
                    <>Authorization: Bearer <span className="text-emerald-500">{"<YOUR_API_KEY>"}</span></>
                  ) : (
                    <>x-api-key: <span className="text-emerald-500">{"<YOUR_API_KEY>"}</span></>
                  )}
                </div>
              </div>
              {integrationTab === 'anthropic' && (
                <div className="space-y-2">
                  <label className="text-xs font-semibold text-gray-500 dark:text-dark-400 uppercase tracking-wider">必须标头</label>
                  <div className="bg-white dark:bg-dark-900/80 border border-border p-3 rounded-xl font-mono text-sm shadow-sm overflow-x-auto text-gray-500 dark:text-dark-300">
                    anthropic-version: <span className="text-blue-500">2023-06-01</span>
                  </div>
                </div>
              )}
            </>
          )}
          <div className="pt-4 border-t border-border flex items-center justify-between text-sm text-gray-600 dark:text-gray-400">
            <span className="flex items-center gap-2">
              <Archive className="w-4 h-4" />
              当前拥有的 API Keys
            </span>
            <span className="font-bold text-gray-900 dark:text-white bg-gray-100 dark:bg-dark-800 px-3 py-1 rounded-full border border-border">
              {apiKeyCount}
            </span>
          </div>
        </div>
      </div>
    </div>
  )
}
