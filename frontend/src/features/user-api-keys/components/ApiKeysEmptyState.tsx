import { useState, type ReactNode } from 'react'
import {
  KeyRound,
  BookOpen,
  Copy,
  Check,
  ShieldCheck,
  Layers,
  Code2,
  Terminal,
} from 'lucide-react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'
import { docsPath } from '@/shared/routes/docs'

interface Props {
  actionNode?: ReactNode
}

type SnippetTab = 'curl' | 'python' | 'node' | 'claude'

export function ApiKeysEmptyState({ actionNode }: Props) {
  const [activeTab, setActiveTab] = useState<SnippetTab>('curl')
  const [copiedSnippet, setCopiedSnippet] = useState(false)

  const origin = typeof window !== 'undefined' ? window.location.origin : ''
  const openaiEndpoint = `${origin}/v1`

  const snippets: Record<SnippetTab, { label: string; code: string }> = {
    curl: {
      label: 'cURL',
      code: `curl ${openaiEndpoint}/chat/completions \\
  -H "Authorization: Bearer agw-your-api-key" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'`,
    },
    python: {
      label: 'Python',
      code: `from openai import OpenAI

client = OpenAI(
    base_url="${openaiEndpoint}",
    api_key="agw-your-api-key",
)

response = client.chat.completions.create(
    model="gpt-4o",
    messages=[{"role": "user", "content": "Hello!"}],
)
print(response.choices[0].message.content)`,
    },
    node: {
      label: 'Node.js',
      code: `import OpenAI from "openai";

const client = new OpenAI({
  baseURL: "${openaiEndpoint}",
  apiKey: "agw-your-api-key",
});

const response = await client.chat.completions.create({
  model: "gpt-4o",
  messages: [{ role: "user", content: "Hello!" }],
});
console.log(response.choices[0].message.content);`,
    },
    claude: {
      label: 'Claude Code',
      code: `# 终端环境变量配置 (Claude 原生协议)
export ANTHROPIC_BASE_URL="${origin}"
export ANTHROPIC_AUTH_TOKEN="agw-your-api-key"

# 启动 Claude Code
claude`,
    },
  }

  const handleCopyCode = () => {
    void navigator.clipboard.writeText(snippets[activeTab].code)
    setCopiedSnippet(true)
    setTimeout(() => setCopiedSnippet(false), 2000)
    toast.success('已复制示例代码到剪贴板')
  }

  return (
    <div className="space-y-6">
      {/* Main Activation Card */}
      <div className="rounded-2xl border border-border bg-card p-6 sm:p-8 lg:p-10 shadow-sm">
        <div className="grid grid-cols-1 lg:grid-cols-12 gap-8 lg:gap-10 items-start">
          {/* Left Column: Guidance & Steps */}
          <div className="lg:col-span-6 space-y-6">
            <div className="space-y-2">
              <div className="inline-flex items-center gap-2 rounded-full border border-primary/20 bg-primary/10 px-3 py-1 text-xs font-semibold text-primary">
                <KeyRound className="h-3.5 w-3.5" />
                <span>新手快速接入</span>
              </div>
              <h2 className="text-xl sm:text-2xl font-bold tracking-tight text-foreground">
                开启您的 AI-GateWay 之旅
              </h2>
              <p className="text-sm leading-relaxed text-muted-foreground">
                创建第一把密钥后，即可通过网关统一调度多家上游大模型，按真实 Token 精算扣费。只需三步：
              </p>
            </div>

            {/* 3 Step List */}
            <div className="space-y-4">
              <div className="flex items-start gap-3.5">
                <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-primary/10 text-xs font-bold text-primary">
                  1
                </div>
                <div className="space-y-0.5 pt-0.5">
                  <h3 className="text-sm font-semibold text-foreground">创建 API 密钥凭证</h3>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    点击下方按钮生成以 <code className="font-mono text-primary font-semibold">agw-</code> 开头的密钥，可设置过期时间、绑定专用分组或限制额度。
                  </p>
                </div>
              </div>

              <div className="flex items-start gap-3.5">
                <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-primary/10 text-xs font-bold text-primary">
                  2
                </div>
                <div className="space-y-0.5 pt-0.5">
                  <h3 className="text-sm font-semibold text-foreground">配置客户端 Base URL</h3>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    在您的应用、Cursor、Claude Code 或 SDK 中将 API 地址填为当前网关端点：
                    <code className="ml-1 rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-foreground">
                      {openaiEndpoint}
                    </code>
                  </p>
                </div>
              </div>

              <div className="flex items-start gap-3.5">
                <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-primary/10 text-xs font-bold text-primary">
                  3
                </div>
                <div className="space-y-0.5 pt-0.5">
                  <h3 className="text-sm font-semibold text-foreground">立即发起调用</h3>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    支持 OpenAI 格式与 Anthropic 原生请求，账本按 Hold → Settle → Release 精准扣费。
                  </p>
                </div>
              </div>
            </div>

            {/* Action Buttons */}
            <div className="flex flex-wrap items-center gap-3 pt-2">
              {actionNode}
              <Link
                to={docsPath('quickstart')}
                className="inline-flex items-center gap-2 rounded-xl border border-border bg-card px-4 py-2.5 text-sm font-medium text-foreground hover:bg-muted transition-colors shadow-2xs"
              >
                <BookOpen className="h-4 w-4 text-muted-foreground" />
                <span>查看接入文档</span>
              </Link>
            </div>
          </div>

          {/* Right Column: Interactive Code Snippet Box */}
          <div className="lg:col-span-6">
            <div className="rounded-xl border border-border bg-muted/40 overflow-hidden shadow-2xs">
              {/* Snippet Header & Tabs */}
              <div className="flex items-center justify-between border-b border-border bg-muted/70 px-3 py-2">
                <div className="flex items-center gap-1">
                  {(['curl', 'python', 'node', 'claude'] as SnippetTab[]).map((tab) => (
                    <button
                      key={tab}
                      type="button"
                      onClick={() => setActiveTab(tab)}
                      className={`rounded-lg px-2.5 py-1 text-xs font-medium transition-colors ${
                        activeTab === tab
                          ? 'bg-card text-foreground shadow-2xs'
                          : 'text-muted-foreground hover:text-foreground'
                      }`}
                    >
                      {snippets[tab].label}
                    </button>
                  ))}
                </div>

                <button
                  type="button"
                  onClick={handleCopyCode}
                  className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium text-muted-foreground hover:text-foreground hover:bg-card/70 transition-colors"
                  title="复制示例代码"
                >
                  {copiedSnippet ? (
                    <>
                      <Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
                      <span className="text-emerald-600 dark:text-emerald-400">已复制</span>
                    </>
                  ) : (
                    <>
                      <Copy className="h-3.5 w-3.5" />
                      <span>复制</span>
                    </>
                  )}
                </button>
              </div>

              {/* Code Pre Block */}
              <div className="p-4 overflow-x-auto bg-card/60 dark:bg-dark-950/40">
                <pre className="font-mono text-xs leading-relaxed text-foreground select-all">
                  <code>{snippets[activeTab].code}</code>
                </pre>
              </div>

              {/* Quick Hint */}
              <div className="border-t border-border bg-muted/30 px-4 py-2.5 flex items-center justify-between text-[11px] text-muted-foreground">
                <span className="flex items-center gap-1.5">
                  <Terminal className="h-3 w-3" />
                  <span>替换代码中的 agw-your-api-key 即可在终端直接执行</span>
                </span>
                <Link to={docsPath('openai')} className="text-primary hover:underline font-medium">
                  参数说明
                </Link>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Value & Architectural Highlights */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div className="rounded-xl border border-border bg-card p-4 space-y-1.5 shadow-2xs">
          <div className="flex items-center gap-2">
            <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <Code2 className="h-4 w-4" />
            </span>
            <h4 className="text-sm font-semibold text-foreground">多协议无缝兼容</h4>
          </div>
          <p className="text-xs text-muted-foreground leading-relaxed">
            对齐 Chat Completions、Responses 与 Anthropic Messages 三套入口，主流 Agent 与客户端即插即用。
          </p>
        </div>

        <div className="rounded-xl border border-border bg-card p-4 space-y-1.5 shadow-2xs">
          <div className="flex items-center gap-2">
            <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
              <ShieldCheck className="h-4 w-4" />
            </span>
            <h4 className="text-sm font-semibold text-foreground">细粒度额度与时窗</h4>
          </div>
          <p className="text-xs text-muted-foreground leading-relaxed">
            支持设置单 Key 消费总额上限与 5h/1d/7d/30d 滚动时间窗口限额，防止并发意外透支。
          </p>
        </div>

        <div className="rounded-xl border border-border bg-card p-4 space-y-1.5 shadow-2xs">
          <div className="flex items-center gap-2">
            <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-violet-500/10 text-violet-600 dark:text-violet-400">
              <Layers className="h-4 w-4" />
            </span>
            <h4 className="text-sm font-semibold text-foreground">分组与订阅隔离</h4>
          </div>
          <p className="text-xs text-muted-foreground leading-relaxed">
            支持绑定订阅套餐专享分组或特定模型通道，实现多项目、开发测试与生产环境的配额隔离。
          </p>
        </div>
      </div>
    </div>
  )
}
