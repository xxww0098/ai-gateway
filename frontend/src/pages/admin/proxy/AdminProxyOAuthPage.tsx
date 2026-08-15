import { useEffect, useState, useCallback, useRef } from "react"
import {
  fetchProviderConfig,
  postProviderConfig,
  submitGatewayOAuthCallback,
  submitSdkOAuthCallback,
} from '@/features/admin-proxy/api'
import { parseOAuthCallbackInput } from '@/features/admin-proxy/oauthCallbackUtils'
import { Input } from '@/shared/components/ui/input'
import { toast } from "sonner"
import {
  Shield, RefreshCw, ExternalLink, Loader2, CheckCircle2,
  XCircle, KeyRound, Globe, ClipboardPaste
} from "lucide-react"
import { Link } from "react-router-dom"
import { adminChannelsTab } from "@/shared/routes/admin"
import { OAuthProviderBrandIcon } from '@/features/admin-proxy/components/OAuthProviderBrandIcon'
import { cn } from '@/shared/utils/utils'

// ── OAuth 提供商定义（与 SDK TUI 保持一致）──
type ManualCallbackMode = 'gateway' | 'sdk' | 'none'

interface OAuthProvider {
  name: string
  key: string
  apiPath: string
  /** Gateway oauth-callback/:provider path segment (gemini, claude, codex, xai). */
  gatewayCallbackProvider?: string
  manualCallbackMode: ManualCallbackMode
  /** Body provider field for SDK redirect_url callback. */
  sdkCallbackProvider?: string
}

const oauthProviders: OAuthProvider[] = [
  {
    name: "Gemini CLI",
    key: "gemini",
    apiPath: "gemini-cli-auth-url",
    gatewayCallbackProvider: "gemini",
    manualCallbackMode: "gateway",
  },
  {
    name: "Claude (Anthropic)",
    key: "anthropic",
    apiPath: "anthropic-auth-url",
    gatewayCallbackProvider: "claude",
    manualCallbackMode: "gateway",
  },
  {
    name: "Codex (OpenAI)",
    key: "codex",
    apiPath: "codex-auth-url",
    gatewayCallbackProvider: "codex",
    manualCallbackMode: "gateway",
  },
  {
    name: "Antigravity",
    key: "antigravity",
    apiPath: "antigravity-auth-url",
    manualCallbackMode: "sdk",
    sdkCallbackProvider: "antigravity",
  },
  {
    name: "Kimi",
    key: "kimi",
    apiPath: "kimi-auth-url",
    manualCallbackMode: "none",
  },
  {
    name: "xAI (Grok)",
    key: "xai",
    apiPath: "xai-auth-url",
    gatewayCallbackProvider: "xai",
    manualCallbackMode: "gateway",
  },
]

type OAuthSessionState = "idle" | "pending" | "polling" | "success" | "error"

interface ProviderSession {
  state: OAuthSessionState
  authURL?: string
  oauthState?: string   // SDK OAuth state 参数
  message?: string
}

interface AuthFile {
  name: string
  provider: string
  status: string
  status_message?: string
  disabled: boolean
  email?: string
  label?: string
  runtime_only?: boolean
  last_refresh?: string
  updated_at?: string
}

export default function AdminProxyOAuthPage() {
  const [loading, setLoading] = useState(true)
  const [authFiles, setAuthFiles] = useState<AuthFile[]>([])
  const [sessions, setSessions] = useState<Record<string, ProviderSession>>({})
  const [manualInputs, setManualInputs] = useState<Record<string, string>>({})
  const [manualSubmitting, setManualSubmitting] = useState<Record<string, boolean>>({})
  const pollTimerRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({})

  // 清理轮询定时器
  useEffect(() => {
    const timers = pollTimerRef.current
    return () => {
      Object.values(timers).forEach(clearTimeout)
    }
  }, [])

  // 加载已有的 auth files（凭证文件），按 provider 分组展示状态
  const loadAuthFiles = useCallback(async () => {
    try {
      const res = await fetchProviderConfig<{ files?: AuthFile[] }>("/auth-files")
      const files: AuthFile[] = res?.files || []
      setAuthFiles(files)
    } catch {
      // auth-files 不可用时静默，不影响 OAuth 发起功能
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadAuthFiles()
  }, [loadAuthFiles])

  // 获取某个 provider 的已有凭证
  const getProviderAuthFiles = (providerKey: string) => {
    return authFiles.filter(f => {
      const p = (f.provider || f.name || '').toLowerCase()
      return p.includes(providerKey)
    })
  }

  // 发起 OAuth 登录
  const startOAuth = async (provider: OAuthProvider) => {
    setSessions(prev => ({
      ...prev,
      [provider.key]: { state: "pending" }
    }))

    try {
      const data = await postProviderConfig<{
        url?: string
        auth_url?: string
        state?: string
        data?: { url?: string; state?: string }
      }>(`/${provider.apiPath}?is_webui=true`)
      const authURL = data?.url || data?.auth_url || data?.data?.url
      const oauthState = data?.state || data?.data?.state

      if (!authURL) {
        throw new Error("SDK 未返回授权 URL")
      }

      // 尝试打开浏览器
      window.open(authURL, '_blank')

      setSessions(prev => ({
        ...prev,
        [provider.key]: {
          state: "polling",
          authURL,
          oauthState,
        }
      }))

      toast.success(`${provider.name} 授权链接已打开，请在浏览器中完成登录`)

      // 开始轮询状态
      if (oauthState) {
        pollOAuthStatus(provider, oauthState)
      } else {
        // SDK 未返回 state（如 device flow），仅靠 auth-files 变化检测完成
        pollAuthFilesForCompletion(provider)
      }
    } catch (err: unknown) {
      setSessions(prev => ({
        ...prev,
        [provider.key]: {
          state: "error",
          message: err instanceof Error ? err.message : "发起 OAuth 失败"
        }
      }))
      toast.error(err instanceof Error ? err.message : `${provider.name} OAuth 发起失败`)
    }
  }

  // 轮询 OAuth 完成状态（通过 SDK session state 参数）
  const pollOAuthStatus = (provider: OAuthProvider, state: string) => {
    const deadline = Date.now() + 6 * 60 * 1000 // 6 分钟超时（比 SDK 5 分钟略长）
    let pollCount = 0
    let lastSeenWait = false // 是否曾经收到过 "wait" 状态

    const poll = async () => {
      pollCount++

      if (Date.now() > deadline) {
        setSessions(prev => ({
          ...prev,
          [provider.key]: { state: "error", message: "OAuth 登录超时（前端 6 分钟限制）" }
        }))
        return
      }

      try {
        const data = await fetchProviderConfig<{ status?: string; error?: string }>(`/get-auth-status?state=${encodeURIComponent(state)}`)
        const status = data?.status

        if (status === "wait") {
          // SDK session 存在且等待中，继续轮询
          lastSeenWait = true
          pollTimerRef.current[provider.key] = setTimeout(poll, 2000)
          return
        }

        if (status === "error") {
          // SDK goroutine 设置了错误（超时 / 认证失败等）
          setSessions(prev => ({
            ...prev,
            [provider.key]: { state: "error", message: data?.error || "OAuth 认证失败" }
          }))
          toast.error(`${provider.name}: ${data?.error || "认证失败"}`)
          loadAuthFiles()
          return
        }

        // status === "ok"：session 不存在
        // 如果我们之前收到过 "wait"，说明 session 曾经存在 → 现在被 CompleteOAuthSession 删除 → 成功
        if (lastSeenWait) {
          setSessions(prev => ({
            ...prev,
            [provider.key]: { state: "success", message: "OAuth 认证完成" }
          }))
          toast.success(`${provider.name} OAuth 认证成功！`)
          loadAuthFiles()
          return
        }

        // 前几次 poll 就收到 ok，可能是 session 注册前的竞态，继续等待
        if (pollCount <= 3) {
          pollTimerRef.current[provider.key] = setTimeout(poll, 2000)
          return
        }

        // 多次 poll 仍然 ok 且从未 wait → session 可能已经完成或从未创建
        // 刷新 auth files 检查是否有新凭证
        setSessions(prev => ({
          ...prev,
          [provider.key]: { state: "success", message: "OAuth 认证完成" }
        }))
        toast.success(`${provider.name} OAuth 认证成功！`)
        loadAuthFiles()
      } catch {
        // 瞬态错误，继续轮询
        pollTimerRef.current[provider.key] = setTimeout(poll, 3000)
      }
    }

    // 初次等 2 秒再开始轮询
    pollTimerRef.current[provider.key] = setTimeout(poll, 2000)
  }

  // 对于 device flow（无 state 参数），通过轮询 auth-files 变化检测完成
  const pollAuthFilesForCompletion = (provider: OAuthProvider) => {
    const deadline = Date.now() + 6 * 60 * 1000
    const initialFiles = getProviderAuthFiles(provider.key)
    const initialCount = initialFiles.length

    const poll = async () => {
      if (Date.now() > deadline) {
        setSessions(prev => ({
          ...prev,
          [provider.key]: { state: "error", message: "OAuth 登录超时（6分钟）" }
        }))
        return
      }

      try {
        await loadAuthFiles()
        const currentFiles = getProviderAuthFiles(provider.key)
        if (currentFiles.length > initialCount) {
          // 有新凭证出现 → 认证完成
          setSessions(prev => ({
            ...prev,
            [provider.key]: { state: "success", message: "OAuth 认证完成" }
          }))
          toast.success(`${provider.name} OAuth 认证成功！`)
          return
        }
        // 继续轮询
        pollTimerRef.current[provider.key] = setTimeout(poll, 3000)
      } catch {
        pollTimerRef.current[provider.key] = setTimeout(poll, 3000)
      }
    }

    pollTimerRef.current[provider.key] = setTimeout(poll, 3000)
  }

  // 取消/重置某个 provider 的 session
  const submitManualCallback = async (provider: OAuthProvider) => {
    if (provider.manualCallbackMode === 'none') return

    const rawInput = (manualInputs[provider.key] || '').trim()
    if (!rawInput) {
      toast.warning('请粘贴浏览器授权完成后的回调地址或 code/state 参数')
      return
    }

    const session = sessions[provider.key]
    const parsed = parseOAuthCallbackInput(rawInput, {
      sessionState: session?.oauthState,
      isXai: provider.key === 'xai',
    })

    if (parsed.error) {
      toast.error(`授权失败：${parsed.error}`)
      return
    }

    setManualSubmitting((prev) => ({ ...prev, [provider.key]: true }))
    try {
      if (provider.manualCallbackMode === 'sdk') {
        const redirectUrl =
          parsed.redirectUrl ||
          (parsed.code && parsed.state
            ? `http://127.0.0.1/?code=${encodeURIComponent(parsed.code)}&state=${encodeURIComponent(parsed.state)}`
            : null)
        if (!redirectUrl) {
          toast.warning('无法解析回调内容，请粘贴完整回调 URL')
          return
        }
        await submitSdkOAuthCallback({
          provider: provider.sdkCallbackProvider || provider.key,
          redirect_url: redirectUrl,
        })
      } else {
        const code = parsed.code?.trim()
        const state = (parsed.state || session?.oauthState || '').trim()
        if (!code || !state) {
          toast.warning(
            provider.key === 'xai'
              ? '请粘贴含 code 的回调内容，并先发起 OAuth 以获取 state'
              : '请粘贴包含 code 与 state 的完整回调 URL'
          )
          return
        }
        await submitGatewayOAuthCallback(provider.gatewayCallbackProvider || provider.key, {
          code,
          state,
        })
      }

      toast.success(`${provider.name} 手动回填已提交，正在完成认证…`)
      setManualInputs((prev) => ({ ...prev, [provider.key]: '' }))

      if (session?.oauthState) {
        pollOAuthStatus(provider, session.oauthState)
      } else {
        pollAuthFilesForCompletion(provider)
      }
      await loadAuthFiles()
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : '手动回填失败')
    } finally {
      setManualSubmitting((prev) => ({ ...prev, [provider.key]: false }))
    }
  }

  const resetSession = (key: string) => {
    if (pollTimerRef.current[key]) {
      clearTimeout(pollTimerRef.current[key])
      delete pollTimerRef.current[key]
    }
    setSessions(prev => {
      const next = { ...prev }
      delete next[key]
      return next
    })
  }

  if (loading) {
    return (
      <div className="flex justify-center p-12">
        <RefreshCw className="h-6 w-6 animate-spin text-primary-500" />
      </div>
    )
  }

  return (
    <div className="space-y-8">

      {/* ── OAuth 提供商列表 ── */}
      <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
        {oauthProviders.map(provider => {
          const session = sessions[provider.key]
          const providerFiles = getProviderAuthFiles(provider.key)
          const hasCredentials = providerFiles.some(f => !f.disabled && f.status !== 'disabled')

          return (
            <div key={provider.key} className="glass-card flex flex-col group overflow-hidden">
              <div className="p-6 flex-1 flex flex-col">
                {/* Header */}
                <div className="flex items-center justify-between mb-4">
                  <div className="flex items-center gap-3">
                    <div
                      className={cn(
                        'h-10 w-10 rounded-xl flex items-center justify-center shrink-0',
                        hasCredentials
                          ? 'bg-primary-50 dark:bg-primary-900/30'
                          : 'bg-gray-100 dark:bg-dark-800'
                      )}
                    >
                      <OAuthProviderBrandIcon providerKey={provider.key} size={24} />
                    </div>
                    <div>
                      <h3 className="text-base font-bold text-gray-900 dark:text-white">
                        {provider.name}
                      </h3>
                    </div>
                  </div>
                </div>

                {/* 凭证状态 */}
                <div className="mb-4 bg-gray-50 dark:bg-dark-900/50 p-3 rounded-lg border border-border">
                  <div className="flex justify-between items-center">
                    <span className="text-sm font-medium text-gray-500 dark:text-gray-400 flex items-center gap-1.5">
                      <KeyRound className="h-3.5 w-3.5" /> 凭证状态
                    </span>
                    {hasCredentials ? (
                      <span className="inline-flex items-center gap-1.5 rounded-md bg-green-50 px-2 py-1 text-xs font-semibold text-green-700 dark:bg-green-900/30 dark:text-green-400">
                        <span className="h-1.5 w-1.5 rounded-full bg-green-500 animate-pulse"></span>
                        已获取 ({providerFiles.filter(f => !f.disabled).length})
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1.5 rounded-md bg-gray-100 px-2 py-1 text-xs font-semibold text-gray-600 dark:bg-dark-800 dark:text-gray-400">
                        <span className="h-1.5 w-1.5 rounded-full bg-gray-400"></span>
                        未配置
                      </span>
                    )}
                  </div>
                </div>

                {provider.manualCallbackMode !== 'none' ? (
                  <details className="mb-4 group rounded-lg border border-dashed border-gray-200 dark:border-dark-700 bg-white/60 dark:bg-dark-900/30">
                    <summary className="cursor-pointer list-none px-3 py-2 text-xs font-medium text-gray-600 dark:text-gray-400 flex items-center gap-1.5 select-none">
                      <ClipboardPaste className="h-3.5 w-3.5 shrink-0 text-primary-500" />
                      <span>无法自动回调？手动粘贴授权结果</span>
                    </summary>
                    <div className="px-3 pb-3 space-y-2 border-t border-gray-100 dark:border-dark-700 pt-2">
                      <p className="text-[11px] leading-relaxed text-gray-500 dark:text-gray-400">
                        {provider.manualCallbackMode === 'sdk'
                          ? '在浏览器完成登录后，将地址栏完整回调 URL 粘贴到下方。'
                          : provider.key === 'xai'
                            ? '先点击「发起 OAuth 登录」，再将回调页中的 code 或完整 URL 粘贴到下方。'
                            : '先发起 OAuth，再将浏览器跳转后的完整回调 URL（含 code 与 state）粘贴到下方。'}
                      </p>
                      <Input
                        value={manualInputs[provider.key] || ''}
                        onChange={(e) =>
                          setManualInputs((prev) => ({
                            ...prev,
                            [provider.key]: e.target.value,
                          }))
                        }
                        placeholder={
                          provider.key === 'xai'
                            ? 'http://127.0.0.1:56121/callback?code=...&state=...'
                            : 'https://.../callback?code=...&state=...'
                        }
                        className="text-xs h-9 font-mono"
                        disabled={Boolean(manualSubmitting[provider.key])}
                      />
                      <button
                        type="button"
                        onClick={() => void submitManualCallback(provider)}
                        disabled={Boolean(manualSubmitting[provider.key])}
                        className="btn btn-sm w-full bg-gray-100 text-gray-700 hover:bg-gray-200 dark:bg-dark-800 dark:text-gray-300 dark:hover:bg-dark-700"
                      >
                        {manualSubmitting[provider.key] ? (
                          <>
                            <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                            提交中…
                          </>
                        ) : (
                          '提交手动回填'
                        )}
                      </button>
                    </div>
                  </details>
                ) : (
                  <p className="mb-4 text-[11px] text-gray-400 dark:text-gray-500">
                    Kimi 使用设备码流程，请在弹窗中完成授权，无需手动回填。
                  </p>
                )}

                {/* Session 状态 / 操作按钮 */}
                <div className="mt-auto">
                  {session?.state === "polling" ? (
                    <div className="space-y-3">
                      <div className="flex items-center gap-2 text-sm text-primary-600 dark:text-primary-400">
                        <Loader2 className="h-4 w-4 animate-spin" />
                        <span>等待浏览器授权完成...</span>
                      </div>
                      {session.authURL && (
                        <a
                          href={session.authURL}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-xs text-primary-500 hover:underline flex items-center gap-1 truncate"
                        >
                          <ExternalLink className="h-3 w-3 flex-shrink-0" />
                          <span className="truncate">重新打开授权页面</span>
                        </a>
                      )}
                      <button
                        onClick={() => resetSession(provider.key)}
                        className="btn btn-sm w-full bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-dark-800 dark:text-gray-400 dark:hover:bg-dark-700"
                      >
                        取消
                      </button>
                    </div>
                  ) : session?.state === "success" ? (
                    <div className="space-y-3">
                      <div className="flex items-center gap-2 text-sm text-green-600 dark:text-green-400">
                        <CheckCircle2 className="h-4 w-4" />
                        <span>{session.message}</span>
                      </div>
                      <button
                        onClick={() => resetSession(provider.key)}
                        className="btn btn-sm w-full btn-primary"
                      >
                        完成
                      </button>
                    </div>
                  ) : session?.state === "error" ? (
                    <div className="space-y-3">
                      <div className="flex items-center gap-2 text-sm text-red-600 dark:text-red-400">
                        <XCircle className="h-4 w-4 flex-shrink-0" />
                        <span className="truncate">{session.message}</span>
                      </div>
                      <button
                        onClick={() => resetSession(provider.key)}
                        className="btn btn-sm w-full bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-dark-800 dark:text-gray-400"
                      >
                        关闭
                      </button>
                    </div>
                  ) : session?.state === "pending" ? (
                    <button disabled className="btn btn-sm w-full btn-primary opacity-70">
                      <Loader2 className="h-4 w-4 animate-spin mr-1" />
                      正在发起...
                    </button>
                  ) : (
                    <button
                      onClick={() => startOAuth(provider)}
                      className="btn btn-sm w-full btn-primary"
                    >
                      <Globe className="h-4 w-4 mr-1.5" />
                      发起 OAuth 登录
                    </button>
                  )}
                </div>
              </div>
            </div>
          )
        })}
      </div>

      {/* ── 底部说明 ── */}
      <div className="p-4 bg-gray-50 dark:bg-dark-900/50 rounded-xl border border-border text-sm text-gray-500 dark:text-gray-400 space-y-1">
        <p className="flex items-center gap-2">
          <Shield className="h-4 w-4 text-primary-500" />
          <span>
            OAuth 凭证由 SDK 网关内部管理，保存在本地 auth 目录中。可在
            <Link to={adminChannelsTab('credentials')} className="text-primary-500 hover:underline font-medium mx-1">
              「代理账池 (凭证管理)」
            </Link>
            页面查看详情。
          </span>
        </p>
      </div>
    </div>
  )
}
