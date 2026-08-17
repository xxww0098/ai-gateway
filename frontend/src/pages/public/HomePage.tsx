import { useEffect } from 'react'
import { Link } from 'react-router-dom'
import { useAuthStore } from '@/features/auth/auth_store'
import { Monitor, CreditCard, Shield, ChevronRight, Activity, Play } from 'lucide-react'

export default function Home() {
  const token = useAuthStore(s => s.token)
  const isAuthenticated = !!token

  useEffect(() => {
    const isDark = document.documentElement.classList.contains('dark') || window.matchMedia('(prefers-color-scheme: dark)').matches
    if (isDark) {
      document.documentElement.classList.add('dark')
    }
  }, [])
  
  const dashboardPath = '/dashboard'

  return (
    <div className="relative flex min-h-screen flex-col overflow-hidden bg-gradient-to-br from-gray-50 via-primary-50/30 to-gray-100 dark:from-dark-950 dark:via-dark-900 dark:to-dark-950">
      {/* Background Decorations */}
      <div className="pointer-events-none absolute inset-0 overflow-hidden">
        <div className="absolute -right-40 -top-40 h-96 w-96 rounded-full bg-primary-400/20 blur-3xl animate-pulse-slow"></div>
        <div className="absolute -bottom-40 -left-40 h-96 w-96 rounded-full bg-primary-500/15 blur-3xl animate-pulse-slow" style={{ animationDelay: '1s' }}></div>
        <div className="absolute left-1/3 top-1/4 h-72 w-72 rounded-full bg-primary-300/10 blur-3xl"></div>
        <div className="absolute inset-0 bg-[linear-gradient(rgba(20,184,166,0.03)_1px,transparent_1px),linear-gradient(90deg,rgba(20,184,166,0.03)_1px,transparent_1px)] bg-[size:64px_64px]"></div>
      </div>

      {/* Header */}
      <header className="relative z-20 px-6 py-4">
        <nav className="mx-auto flex max-w-6xl items-center justify-between">
          <div className="flex items-center gap-3">
            <img src="/icon.svg" alt="AI-GateWay" className="h-10 w-10 rounded-xl shrink-0" />
            <span className="text-xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-gray-900 to-gray-600 dark:from-white dark:to-gray-300">
              AI-GateWay
            </span>
          </div>

          <div className="flex items-center gap-4">
            {isAuthenticated ? (
              <Link 
                to={dashboardPath}
                className="btn btn-primary btn-sm px-5 rounded-full"
              >
                控制台
              </Link>
            ) : (
              <div className="flex items-center gap-3">
                <Link to="/login" className="text-sm font-medium text-gray-600 hover:text-gray-900 dark:text-gray-300 dark:hover:text-white transition-colors">登录</Link>
                <Link to="/register" className="btn btn-primary btn-sm rounded-full px-5">免费注册</Link>
              </div>
            )}
          </div>
        </nav>
      </header>

      {/* Hero Section */}
      <main className="relative z-10 flex-1 px-6 pt-24 pb-16">
        <div className="mx-auto max-w-6xl">
          <div className="flex flex-col items-center justify-between gap-12 lg:flex-row lg:gap-16">
            
            {/* Left: Text Content */}
            <div className="flex-1 text-center lg:text-left space-y-8 animate-in fade-in slide-in-from-bottom-8 duration-700" style={{ willChange: 'transform, opacity' }}>
              <div className="inline-flex items-center gap-2 rounded-full border border-primary-200 bg-primary-50/50 px-4 py-1.5 text-sm font-medium text-primary-700 dark:border-primary-800/50 dark:bg-primary-900/20 dark:text-primary-400">
                <Activity className="h-4 w-4" />
                AI-GateWay 全新管理层
              </div>
              
              <h1 className="text-5xl font-extrabold tracking-tight text-gray-900 dark:text-white md:text-6xl lg:text-7xl leading-tight">
                企业级 AI 接口
                <br />
                <span className="text-gradient">统一网关与计费</span>
              </h1>
              
              <p className="text-lg text-gray-600 dark:text-gray-400 max-w-xl mx-auto lg:mx-0 leading-relaxed">
                兼容 OpenAI, Claude, Gemini 的高性能 AI 代理网关。提供组织级密钥管理、精确到 Token 的实时计费与高并发会话池调度。
              </p>

              <div className="flex flex-col sm:flex-row items-center justify-center lg:justify-start gap-4">
                <Link to={isAuthenticated ? dashboardPath : '/login'} className="btn btn-primary btn-lg w-full sm:w-auto px-8 py-4 text-base shadow-glow">
                  {isAuthenticated ? '进入控制台' : '立即开始接入'}
                  <ChevronRight className="ml-1 h-5 w-5" />
                </Link>
              </div>
            </div>

            {/* Right: Premium Terminal Animation Mockup */}
            <div className="flex flex-1 justify-center lg:justify-end animate-in fade-in slide-in-from-right-8 duration-1000" style={{ willChange: 'transform, opacity' }}>
              <div className="relative w-full max-w-md perspective-1000">
                <div className="relative z-10 w-full bg-[#0d1117] rounded-2xl shadow-[0_20px_50px_rgba(0,0,0,0.5)] border border-white/10 overflow-hidden transform transition-transform hover:scale-[1.02] duration-500">
                  {/* Mac style header */}
                  <div className="flex items-center px-4 py-3 bg-[#161b22] border-b border-white/5">
                    <div className="flex gap-2">
                      <div className="w-3 h-3 rounded-full bg-red-500"></div>
                      <div className="w-3 h-3 rounded-full bg-amber-500"></div>
                      <div className="w-3 h-3 rounded-full bg-green-500"></div>
                    </div>
                    <div className="flex-1 text-center text-xs font-mono text-gray-400">ai-gateway ~ bash</div>
                  </div>
                  {/* Code body */}
                  <div className="p-5 font-mono text-sm leading-relaxed">
                    <div className="text-primary-400 flex items-center gap-2">
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
                    
                    <div className="mt-4 text-gray-500 flex items-center gap-2">
                      <Play className="h-3 w-3 animate-pulse text-amber-500" />
                      <span>Routing to upstream pool...</span>
                    </div>
                    
                    <div className="mt-4 text-green-400">HTTP/1.1 200 OK</div>
                    <div className="text-gray-300">{'{'}</div>
                    <div className="pl-4 text-blue-300">"id": "chatcmpl-123",</div>
                    <div className="pl-4 text-blue-300">"choices": [{'{'} "message": {'{'} "content": "Hello!" {'}'} {'}'}],</div>
                    <div className="pl-4 text-amber-300">"usage": {'{'} "prompt_tokens": 12, "completion_tokens": 5 {'}'}</div>
                    <div className="text-gray-300">{'}'}</div>
                    
                    <div className="mt-4 flex items-center">
                      <span className="text-green-400">$</span>
                      <span className="ml-2 w-2 h-4 bg-gray-400 animate-pulse"></span>
                    </div>
                  </div>
                </div>
                
                {/* Glow behind terminal */}
                <div className="absolute -inset-1 rounded-2xl bg-gradient-to-br from-primary-500 to-blue-600 opacity-20 blur-2xl z-0"></div>
              </div>
            </div>
          </div>

          {/* Features Grid */}
          <div className="mt-32 grid gap-6 md:grid-cols-3">
            <div className="glass-card p-8 group">
              <div className="mb-6 flex h-14 w-14 items-center justify-center rounded-2xl bg-gradient-to-br from-blue-500 to-blue-600 shadow-glow transition-transform group-hover:scale-110">
                <Monitor className="h-6 w-6 text-white" />
              </div>
              <h3 className="mb-3 text-xl font-bold text-gray-900 dark:text-white">统一代理路由</h3>
              <p className="text-gray-600 dark:text-dark-300 leading-relaxed">
                SDK 级别的协议转换。原生支持 OpenAI、Anthropic 和 Google Gemini 接口，一致的输入输出格式，无缝衔接主流框架。
              </p>
            </div>

            <div className="glass-card p-8 group">
              <div className="mb-6 flex h-14 w-14 items-center justify-center rounded-2xl bg-gradient-to-br from-primary-500 to-primary-600 shadow-glow transition-transform group-hover:scale-110">
                <Shield className="h-6 w-6 text-white" />
              </div>
              <h3 className="mb-3 text-xl font-bold text-gray-900 dark:text-white">高可用会话池</h3>
              <p className="text-gray-600 dark:text-dark-300 leading-relaxed">
                不再受限于单一密钥。支持多账户轮询、并发控制、错误重试和自动封禁，保证企业级业务的高可用性。
              </p>
            </div>

            <div className="glass-card p-8 group">
              <div className="mb-6 flex h-14 w-14 items-center justify-center rounded-2xl bg-gradient-to-br from-purple-500 to-purple-600 shadow-glow transition-transform group-hover:scale-110">
                <CreditCard className="h-6 w-6 text-white" />
              </div>
              <h3 className="mb-3 text-xl font-bold text-gray-900 dark:text-white">精细化多租户计费</h3>
              <p className="text-gray-600 dark:text-dark-300 leading-relaxed">
                按 Model ID 自定义定价，配合用户的余额抵扣策略，实时拦截与限流，轻松落地 AI 商业化服务。
              </p>
            </div>
          </div>
        </div>
      </main>

      <footer className="relative z-10 border-t border-border px-6 py-8">
        <div className="mx-auto max-w-6xl flex flex-col md:flex-row items-center justify-between gap-4">
          <p className="text-sm text-muted-foreground">
            &copy; {new Date().getFullYear()} ai-gateway
          </p>
        </div>
      </footer>
    </div>
  )
}
