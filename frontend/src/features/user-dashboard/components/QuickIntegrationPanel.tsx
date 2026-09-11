import { useState } from 'react'
import { Link } from 'react-router-dom'
import { Terminal, Code2, MessageSquare, Copy, Check, ArrowRight, Key } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { toast } from 'sonner'
import type { IntegrationTab, QuickIntegrationPanelProps } from '../types'
import { anthropicBaseUrl, openaiBaseUrl } from '@/pages/docs/guide'
import { userRoutes } from '@/shared/routes/user'
import { docsPath } from '@/shared/routes/docs'

const integrationTabs: Array<{ id: IntegrationTab; label: string; Icon: LucideIcon }> = [
  { id: 'openai', label: 'OpenAI 兼容', Icon: Code2 },
  { id: 'anthropic', label: 'Anthropic 原生', Icon: MessageSquare },
  { id: 'amp', label: 'Amp', Icon: Terminal },
]

function CopyableField({ label, value, hint }: { label: string; value: string; hint?: string }) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(value)
      setCopied(true)
      toast.success('已复制到剪贴板')
      setTimeout(() => setCopied(false), 2000)
    } catch {
      toast.error('复制失败，请手动选择复制')
    }
  }

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between text-[11px] font-semibold text-muted-foreground">
        <span>{label}</span>
        {hint && <span className="text-[10px] font-normal text-muted-foreground/80">{hint}</span>}
      </div>
      <div className="group relative flex items-center justify-between rounded-[8px] border border-border bg-muted/30 px-3 py-2 text-xs font-mono text-foreground transition-colors hover:bg-muted/50">
        <span className="truncate select-all mr-2">{value}</span>
        <button
          type="button"
          onClick={handleCopy}
          aria-label={`复制 ${label}`}
          className="shrink-0 rounded-[6px] p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          {copied ? <Check className="size-3.5 text-emerald-600 dark:text-emerald-400" /> : <Copy className="size-3.5" />}
        </button>
      </div>
    </div>
  )
}

export function QuickIntegrationPanel({
  apiKeyCount,
  integrationTab,
  onIntegrationTabChange,
}: QuickIntegrationPanelProps) {
  const origin = window.location.origin
  const description = integrationTab === 'openai'
    ? '配置 OpenAI 兼容客户端（如 Cursor、Cline、aider）连接到 AI-GateWay 代理池：'
    : integrationTab === 'anthropic'
      ? '配置 Anthropic 原生客户端（如 Claude Code）连接到 AI-GateWay 代理池：'
      : '配置 Amp CLI 或编辑器扩展使用 AI-GateWay 的 Amp 路由：'

  return (
    <div className="overflow-hidden rounded-[13px] border border-border bg-card shadow-[0_1px_2px_rgb(0_0_0/0.07)] flex flex-col justify-between">
      {/* Panel Header */}
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <div className="flex items-center gap-2">
          <span className="size-1.5 rounded-[2px] bg-primary" aria-hidden />
          <div>
            <h3 className="text-xs font-semibold text-foreground">快速接入</h3>
            <p className="text-[11px] text-muted-foreground">标准客户端与 IDE 插件中转配置</p>
          </div>
        </div>
        <Link
          to={docsPath('quickstart')}
          className="inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
        >
          接入文档
          <ArrowRight className="size-3" />
        </Link>
      </div>

      <div className="p-4 flex-1 flex flex-col justify-between space-y-4">
        {/* Protocol Switcher */}
        <div className="flex flex-wrap gap-1 p-0.5 bg-muted rounded-[8px] w-fit max-w-full">
          {integrationTabs.map(tab => {
            const Icon = tab.Icon
            const isActive = integrationTab === tab.id
            return (
              <button
                key={tab.id}
                type="button"
                onClick={() => onIntegrationTabChange(tab.id)}
                className={`inline-flex items-center gap-1.5 px-3 py-1 rounded-[6px] text-xs font-medium transition-all ${
                  isActive
                    ? 'bg-card text-foreground shadow-[0_1px_2px_rgb(0_0_0/0.07)]'
                    : 'text-muted-foreground hover:text-foreground'
                }`}
              >
                <Icon className="size-3.5" />
                {tab.label}
              </button>
            )
          })}
        </div>

        <p className="text-xs leading-relaxed text-muted-foreground">
          {description}
        </p>

        {/* Code Snippets */}
        <div className="space-y-3">
          {integrationTab === 'amp' ? (
            <>
              <CopyableField
                label="Amp 环境变量"
                value={`AMP_URL="${origin}"\nAMP_API_KEY="<YOUR_API_KEY>"`}
              />
              <CopyableField
                label="VS Code Settings"
                value={`{\n  "amp.url": "${origin}"\n}`}
              />
            </>
          ) : (
            <>
              <CopyableField
                label="Base URL 请求地址"
                value={integrationTab === 'openai' ? openaiBaseUrl(origin) : anthropicBaseUrl(origin)}
              />
              <div className="space-y-1.5">
                <span className="text-[11px] font-semibold text-muted-foreground">身份认证标头</span>
                <div className="rounded-[8px] border border-border bg-muted/30 px-3 py-2 text-xs font-mono text-foreground select-all">
                  Authorization: Bearer <span className="text-emerald-600 dark:text-emerald-400">{"<YOUR_API_KEY>"}</span>
                </div>
              </div>
              {integrationTab === 'anthropic' && (
                <div className="space-y-1.5">
                  <span className="text-[11px] font-semibold text-muted-foreground">必须标头</span>
                  <div className="rounded-[8px] border border-border bg-muted/30 px-3 py-2 text-xs font-mono text-foreground select-all">
                    anthropic-version: <span className="text-primary font-medium">2023-06-01</span>
                  </div>
                </div>
              )}
            </>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-border/50 pt-3 text-xs text-muted-foreground">
          <span className="inline-flex items-center gap-1.5">
            <Key className="size-3.5 text-muted-foreground" />
            已分配 API Keys
            <span className="ml-1 rounded-md border border-border bg-muted px-2 py-0.5 font-semibold tabular-nums text-foreground">
              {apiKeyCount}
            </span>
          </span>
          <Link
            to={userRoutes.keys}
            className="text-xs font-medium text-primary hover:underline"
          >
            管理密钥 →
          </Link>
        </div>
      </div>
    </div>
  )
}
