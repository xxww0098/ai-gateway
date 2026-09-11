import { useState, useMemo } from 'react'
import { Check, Copy, Cpu, ExternalLink } from 'lucide-react'
import { toast } from 'sonner'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/shared/components/ui/dialog'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/shared/components/ui/tabs'
import type { ModelCatalogItem } from '@/features/pricing/model_catalog'
import { getProviderDisplayName, getModelProviderKey } from '@/features/pricing/model_catalog'
import { getProviderBrandIcon } from '@/features/pricing/modelCatalogUtils'
import { Link } from 'react-router-dom'
import { userRoutes } from '@/shared/routes/user'

interface ModelQuickCodeDialogProps {
  model: ModelCatalogItem | null
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function ModelQuickCodeDialog({
  model,
  open,
  onOpenChange,
}: ModelQuickCodeDialogProps) {
  const [copiedTab, setCopiedTab] = useState<string | null>(null)

  const origin = typeof window !== 'undefined' ? window.location.origin : 'https://api.ai-gateway.com'
  const baseUrl = `${origin}/v1`

  const provider = model ? getModelProviderKey(model) : 'other'
  const providerLabel = model ? getProviderDisplayName(provider) : ''
  const BrandIcon = model ? getProviderBrandIcon(provider) : null

  const codeSnippets = useMemo(() => {
    if (!model) return { curl: '', python: '', node: '', cli: '' }
    const modelId = model.id

    const curl = `curl ${baseUrl}/chat/completions \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer agw-YOUR_API_KEY" \\
  -d '{
    "model": "${modelId}",
    "messages": [
      {
        "role": "user",
        "content": "Hello! Please introduce yourself."
      }
    ],
    "temperature": 0.7
  }'`

    const python = `from openai import OpenAI

# 初始化客户端，指向 AI-GateWay 网关端点
client = OpenAI(
    api_key="agw-YOUR_API_KEY",
    base_url="${baseUrl}",
)

response = client.chat.completions.create(
    model="${modelId}",
    messages=[
        {"role": "user", "content": "Hello! Please introduce yourself."}
    ],
    temperature=0.7,
)

print(response.choices[0].message.content)`

    const node = `import OpenAI from 'openai';

// 初始化客户端，指向 AI-GateWay 网关端点
const client = new OpenAI({
  apiKey: 'agw-YOUR_API_KEY',
  baseURL: '${baseUrl}',
});

async function main() {
  const completion = await client.chat.completions.create({
    model: '${modelId}',
    messages: [
      { role: 'user', content: 'Hello! Please introduce yourself.' },
    ],
  });

  console.log(completion.choices[0].message.content);
}

main();`

    const cli = `# 环境变量配置（兼容大多数 AI 开发工具）
export OPENAI_BASE_URL="${baseUrl}"
export OPENAI_API_KEY="agw-YOUR_API_KEY"

# Claude Code / Cursor / Cline 配置提示：
# 在终端或设置面板将 Base URL 指向 ${baseUrl}
# 模型名称填写：${modelId}`

    return { curl, python, node, cli }
  }, [model, baseUrl])

  const copyCode = (tab: string, code: string) => {
    navigator.clipboard.writeText(code)
    setCopiedTab(tab)
    toast.success('调用代码已复制到剪贴板')
    setTimeout(() => setCopiedTab(null), 2000)
  }

  if (!model) return null

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl sm:max-w-3xl border-border bg-card p-6 shadow-2xl">
        <DialogHeader className="space-y-2 pb-2 border-b border-border/60">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border bg-muted/60">
              {BrandIcon ? (
                <BrandIcon size={18} className="shrink-0" />
              ) : (
                <Cpu className="h-4 w-4 text-primary" />
              )}
            </div>
            <div className="min-w-0 flex-1">
              <DialogTitle className="flex items-center gap-2 text-base font-bold text-foreground">
                <span className="truncate">{model.display_name || model.id}</span>
                <span className="rounded-md bg-muted px-2 py-0.5 text-xs font-semibold text-muted-foreground">
                  {providerLabel}
                </span>
              </DialogTitle>
              <DialogDescription className="font-mono text-xs text-muted-foreground truncate mt-0.5">
                {model.id}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="space-y-4 pt-2">
          {/* Quick Info Bar */}
          <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl bg-muted/40 px-3.5 py-2.5 border border-border/60 text-xs">
            <div className="flex items-center gap-3 text-muted-foreground">
              <span>端点协议：<strong className="text-foreground font-medium">OpenAI 兼容 (/v1)</strong></span>
              {model.context_length && (
                <span>上下文：<strong className="text-foreground font-medium tabular-nums">{Math.round(model.context_length).toLocaleString()} tokens</strong></span>
              )}
            </div>
            <Link
              to={userRoutes.keys}
              className="inline-flex items-center gap-1 font-semibold text-primary hover:underline"
            >
              获取 agw- 密钥
              <ExternalLink className="h-3 w-3" />
            </Link>
          </div>

          {/* Code Snippet Tabs */}
          <Tabs defaultValue="curl" className="w-full">
            <div className="flex items-center justify-between mb-2">
              <TabsList className="h-8 bg-muted/60 p-0.5">
                <TabsTrigger value="curl" className="h-7 text-xs px-3">
                  cURL
                </TabsTrigger>
                <TabsTrigger value="python" className="h-7 text-xs px-3">
                  Python
                </TabsTrigger>
                <TabsTrigger value="node" className="h-7 text-xs px-3">
                  Node.js
                </TabsTrigger>
                <TabsTrigger value="cli" className="h-7 text-xs px-3">
                  终端 / CLI
                </TabsTrigger>
              </TabsList>
            </div>

            {(['curl', 'python', 'node', 'cli'] as const).map((tab) => (
              <TabsContent key={tab} value={tab} className="relative mt-0 focus-visible:outline-none">
                <div className="relative rounded-xl border border-border/80 bg-zinc-950 p-4 text-zinc-100 dark:bg-black font-mono text-xs leading-relaxed overflow-x-auto shadow-inner">
                  <button
                    type="button"
                    onClick={() => copyCode(tab, codeSnippets[tab])}
                    className="absolute right-3 top-3 inline-flex items-center gap-1.5 rounded-lg border border-zinc-700 bg-zinc-800/80 px-2.5 py-1 text-[11px] font-medium text-zinc-300 backdrop-blur transition hover:bg-zinc-700 hover:text-zinc-100 active:scale-95 shadow-sm"
                  >
                    {copiedTab === tab ? (
                      <>
                        <Check className="h-3.5 w-3.5 text-emerald-400" />
                        <span>已复制</span>
                      </>
                    ) : (
                      <>
                        <Copy className="h-3.5 w-3.5" />
                        <span>复制代码</span>
                      </>
                    )}
                  </button>
                  <pre className="pr-20 whitespace-pre">{codeSnippets[tab]}</pre>
                </div>
              </TabsContent>
            ))}
          </Tabs>

          <p className="text-[11px] text-muted-foreground leading-relaxed">
            提示：所有请求头必须携带以 <code className="rounded bg-muted px-1.5 py-0.5 font-mono font-medium text-foreground">agw-</code> 开头的 API 密钥。模型计费将按输入/输出/缓存/推理真实 Token 精算扣除。
          </p>
        </div>
      </DialogContent>
    </Dialog>
  )
}
