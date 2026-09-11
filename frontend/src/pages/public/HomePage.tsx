import { Link } from 'react-router-dom'
import { useAuthStore } from '@/features/auth/auth_store'
import { ThemeToggleButton } from '@/shared/components/ThemeToggleButton'
import { docsRoutes } from '@/shared/routes/docs'
import { Monitor, CreditCard, Shield, ChevronRight, Play, Check } from 'lucide-react'

export default function Home() {
  const token = useAuthStore(s => s.token)
  const isAuthenticated = !!token

  const dashboardPath = '/dashboard'

  return (
    <div className="relative flex min-h-screen flex-col overflow-hidden bg-background">
      <div className="pointer-events-none absolute inset-0 overflow-hidden">
        <div className="absolute -right-40 -top-40 h-96 w-96 rounded-full bg-primary/15 blur-3xl" />
        <div className="absolute -bottom-40 -left-40 h-96 w-96 rounded-full bg-primary/10 blur-3xl" />
      </div>

      <header className="relative z-20 px-6 py-4">
        <nav className="mx-auto flex max-w-6xl items-center justify-between">
          <div className="flex items-center gap-3">
            <img src="/icon.svg" alt="AI-GateWay" className="h-10 w-10 shrink-0 rounded-xl" />
            <span className="text-xl font-bold text-foreground">AI-GateWay</span>
          </div>

          <div className="flex items-center gap-4">
            <ThemeToggleButton />
            <Link
              to={docsRoutes.root}
              className="hidden text-sm font-medium text-muted-foreground transition-colors hover:text-foreground sm:inline"
            >
              接入指南
            </Link>
            {isAuthenticated ? (
              <Link to={dashboardPath} className="btn btn-primary btn-sm rounded-full px-5">
                控制台
              </Link>
            ) : (
              <div className="flex items-center gap-3">
                <Link to="/login" className="text-sm font-medium text-muted-foreground transition-colors hover:text-foreground">登录</Link>
                <Link to="/register" className="btn btn-primary btn-sm rounded-full px-5">免费注册</Link>
              </div>
            )}
          </div>
        </nav>
      </header>

      <main className="relative z-10 flex-1 px-6 pb-20 pt-20 sm:pt-28">
        <div className="mx-auto max-w-6xl">
          <div className="flex flex-col items-center justify-between gap-12 lg:flex-row lg:gap-16">
            <div className="flex-1 space-y-8 text-center lg:text-left">
              <div className="space-y-5">
                <h1 className="text-4xl font-extrabold tracking-[-0.03em] text-foreground sm:text-5xl lg:text-[3.4rem] lg:leading-[1.08]">
                  一把密钥，接通多家上游大模型
                </h1>
                <p className="mx-auto max-w-xl text-base leading-relaxed text-muted-foreground sm:text-lg lg:mx-0">
                  AI-GateWay 是自部署的多租户 LLM 中转：一张余额、一套 agw- 密钥走
                  OpenAI 与 Anthropic 兼容接口，费用按真实 Token 逐笔精算。
                </p>
              </div>

              <div className="flex flex-col items-center justify-center gap-4 sm:flex-row lg:justify-start">
                <Link to={isAuthenticated ? dashboardPath : '/login'} className="btn btn-primary btn-lg w-full px-8 py-3.5 text-base sm:w-auto shadow-sm hover:shadow-md">
                  {isAuthenticated ? '进入控制台' : '立即开始接入'}
                  <ChevronRight className="ml-1 h-5 w-5" />
                </Link>
                <Link to={docsRoutes.root} className="btn btn-secondary btn-lg w-full px-7 py-3.5 text-base sm:w-auto">
                  查看接入文档
                </Link>
              </div>

              <ul className="flex flex-wrap items-center justify-center gap-x-5 gap-y-2 text-xs font-medium text-muted-foreground lg:justify-start">
                <li className="inline-flex items-center gap-1.5">
                  <Check className="h-3.5 w-3.5 text-primary" />
                  agw- 统一密钥
                </li>
                <li className="inline-flex items-center gap-1.5">
                  <Check className="h-3.5 w-3.5 text-primary" />
                  OpenAI / Anthropic 兼容
                </li>
                <li className="inline-flex items-center gap-1.5">
                  <Check className="h-3.5 w-3.5 text-primary" />
                  Hold → Settle → Release 逐笔对账
                </li>
              </ul>
            </div>

            <div className="flex flex-1 justify-center lg:justify-end w-full">
              <div className="relative w-full max-w-lg">
                <div className="relative z-10 w-full overflow-hidden rounded-2xl border border-primary/20 bg-[#0d1117] shadow-[0_20px_50px_rgba(0,0,0,0.45)]">
                  <div className="flex items-center border-b border-white/10 bg-[#161b22] px-4 py-3">
                    <div className="flex gap-2">
                      <div className="h-3 w-3 rounded-full bg-red-500/80" />
                      <div className="h-3 w-3 rounded-full bg-amber-500/80" />
                      <div className="h-3 w-3 rounded-full bg-green-500/80" />
                    </div>
                    <div className="flex-1 text-center font-mono text-xs text-gray-400">ai-gateway ~ bash</div>
                  </div>
                  <div className="min-w-0 break-words p-5 font-mono text-sm leading-relaxed">
                    <div className="flex items-center gap-2 text-primary-400">
                      <span className="text-green-400">$</span>
                      <span>curl -X POST /v1/chat/completions \</span>
                    </div>
                    <div className="pl-4 text-gray-300">
                      -H "Authorization: Bearer agw-..." \
                    </div>
                    <div className="pl-4 text-gray-300">
                      -H "Content-Type: application/json" \
                    </div>
                    <div className="pl-4 text-gray-300">
                      -d '{'{"model":"claude-3-5-sonnet-20241022","messages":[{...}]}'}'
                    </div>

                    <div className="mt-4 flex items-center gap-2 text-gray-500">
                      <Play className="h-3 w-3 text-amber-400" />
                      <span className="text-xs text-amber-300/80">Routing to upstream pool...</span>
                    </div>

                    <div className="mt-3 text-green-400 font-bold">HTTP/1.1 200 OK</div>
                    <div className="text-gray-300">{'{'}</div>
                    <div className="pl-4 text-blue-300">"id": "chatcmpl-123",</div>
                    <div className="pl-4 text-blue-300">"choices": [{'{'} "message": {'{'} "content": "Hello!" {'}'} {'}'}],</div>
                    <div className="pl-4 text-amber-300 font-semibold">"usage": {'{'} "prompt_tokens": 12, "completion_tokens": 5 {'}'}</div>
                    <div className="text-gray-300">{'}'}</div>

                    <div className="mt-4 flex items-center">
                      <span className="text-green-400">$</span>
                      <span className="ml-2 h-4 w-2 bg-gray-400 animate-pulse" />
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div className="mt-20 grid gap-6 md:grid-cols-3">
            <div className="rounded-2xl border border-border/80 bg-card p-8 shadow-xs transition-all duration-200 hover:border-primary/40 hover:shadow-sm">
              <div className="mb-6 flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
                <Monitor className="h-6 w-6" />
              </div>
              <h3 className="mb-3 text-xl font-bold tracking-tight text-foreground">统一代理路由</h3>
              <p className="leading-relaxed text-sm text-muted-foreground">
                SDK 级别的协议转换。原生支持 OpenAI、Anthropic 和 Google Gemini 接口，一致的输入输出格式，无缝衔接主流框架。
              </p>
            </div>

            <div className="rounded-2xl border border-border/80 bg-card p-8 shadow-xs transition-all duration-200 hover:border-primary/40 hover:shadow-sm">
              <div className="mb-6 flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
                <Shield className="h-6 w-6" />
              </div>
              <h3 className="mb-3 text-xl font-bold tracking-tight text-foreground">高可用会话池</h3>
              <p className="leading-relaxed text-sm text-muted-foreground">
                不再受限于单一密钥。支持多账户轮询、并发控制、错误重试和自动封禁，保证企业级业务的高可用性。
              </p>
            </div>

            <div className="rounded-2xl border border-border/80 bg-card p-8 shadow-xs transition-all duration-200 hover:border-primary/40 hover:shadow-sm">
              <div className="mb-6 flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10 text-primary">
                <CreditCard className="h-6 w-6" />
              </div>
              <h3 className="mb-3 text-xl font-bold tracking-tight text-foreground">精细化多租户计费</h3>
              <p className="leading-relaxed text-sm text-muted-foreground">
                按 Model ID 自定义定价，配合用户的余额抵扣策略，实时拦截与限流，轻松落地 AI 商业化服务。
              </p>
            </div>
          </div>
        </div>
      </main>

      <footer className="relative z-10 border-t border-border px-6 py-8">
        <div className="mx-auto flex max-w-6xl items-center justify-between">
          <p className="text-sm text-muted-foreground">
            &copy; {new Date().getFullYear()} AI-GateWay
          </p>
        </div>
      </footer>
    </div>
  )
}
