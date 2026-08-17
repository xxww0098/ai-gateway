import { useEffect, useState, useCallback, useRef } from "react"
import {
  deleteAuthFile,
  fetchProviderConfig,
  pollDeviceOAuth,
  postProviderConfig,
  submitGatewayOAuthCallback,
  submitSdkOAuthCallback,
} from '@/features/admin-proxy/api'
import { parseOAuthCallbackInput } from '@/features/admin-proxy/oauthCallbackUtils'
import {
  kiroStartBody,
  parseDeviceAuthStart,
  parseImportJson,
  type KiroAuthMethod,
} from '@/features/admin-proxy/oauthDeviceUtils'
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

type ManualCallbackMode = 'gateway' | 'sdk' | 'none'

interface OAuthProvider {
  name: string
  key: string
  apiPath: string
  /** Gateway oauth-callback/:provider path segment (gemini, claude, codex, xai, kiro). */
  gatewayCallbackProvider?: string
  manualCallbackMode: ManualCallbackMode
  /** Body provider field for SDK redirect_url callback. */
  sdkCallbackProvider?: string
  deviceFlow?: boolean
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
    manualCallbackMode: "none",
    deviceFlow: true,
  },
  {
    name: "Kiro",
    key: "kiro",
    apiPath: "kiro-auth-url",
    gatewayCallbackProvider: "kiro",
    manualCallbackMode: "gateway",
    deviceFlow: true,
  },
]

type OAuthSessionState = "idle" | "pending" | "polling" | "success" | "error"

interface ProviderSession {
  state: OAuthSessionState
  authURL?: string
  oauthState?: string
  message?: string
  userCode?: string
  verificationUri?: string
  interval?: number
  flow?: string
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
  const [disconnecting, setDisconnecting] = useState<Record<string, boolean>>({})
  const [kiroMethod, setKiroMethod] = useState<KiroAuthMethod>("device")
  const [kiroStartUrl, setKiroStartUrl] = useState("")
  const [kiroRegion, setKiroRegion] = useState("us-east-1")
  const [kiroImport, setKiroImport] = useState("")
  const pollTimerRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({})

  useEffect(() => {
    const timers = pollTimerRef.current
    return () => {
      Object.values(timers).forEach(clearTimeout)
    }
  }, [])

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

  const getProviderAuthFiles = (providerKey: string) => {
    return authFiles.filter(f => {
      const p = (f.provider || f.name || '').toLowerCase()
      if (providerKey === 'xai') return p.includes('xai') || p.includes('grok')
      return p.includes(providerKey)
    })
  }

  const startOAuth = async (provider: OAuthProvider, body?: unknown) => {
    setSessions(prev => ({
      ...prev,
      [provider.key]: { state: "pending" }
    }))

    try {
      const data = await postProviderConfig<{
        url?: string
        auth_url?: string
        state?: string
        status?: string
        auth_id?: string
        flow?: string
        data?: { url?: string; state?: string }
      }>(`/${provider.apiPath}?is_webui=true`, body)
      if (data?.status === "success" || data?.flow === "import") {
        setSessions(prev => ({
          ...prev,
          [provider.key]: { state: "success", message: "OAuth 认证完成", flow: data.flow }
        }))
        toast.success(`${provider.name} 已连接到 AI-GateWay`)
        loadAuthFiles()
        return
      }

      const device = parseDeviceAuthStart(data)
      const authURL = device?.verificationUriComplete
        || device?.verificationUri
        || data?.url
        || data?.auth_url
        || data?.data?.url
      const oauthState = device?.state || data?.state || data?.data?.state

      if (!authURL && !device) {
        throw new Error("AI-GateWay 未返回授权 URL")
      }

      if (authURL) {
        window.open(authURL, '_blank')
      }

      setSessions(prev => ({
        ...prev,
        [provider.key]: {
          state: "polling",
          authURL,
          oauthState,
          userCode: device?.userCode,
          verificationUri: device?.verificationUri,
          interval: device?.interval,
          flow: device ? "device" : data?.flow,
        }
      }))

      if (device) {
        toast.success(`请在浏览器打开链接并输入设备码 ${device.userCode}`)
      } else {
        toast.success(`${provider.name} 授权链接已打开，请在浏览器中完成登录`)
      }

      if (oauthState) {
        pollOAuthStatus(provider, oauthState, Boolean(device || provider.deviceFlow))
      } else {
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

  const startKiro = async () => {
    const provider = oauthProviders.find(item => item.key === "kiro")
    if (!provider) return
    if (kiroMethod === "import") {
      const parsed = parseImportJson(kiroImport)
      if (!parsed.ok) {
        toast.error(parsed.error)
        return
      }
      await startOAuth(provider, kiroStartBody({ method: "import", token: parsed.token }))
      return
    }
    if (kiroMethod === "idc" && !kiroStartUrl.trim()) {
      toast.warning("IDC 登录需要填写 AWS IAM Identity Center 起始 URL")
      return
    }
    await startOAuth(provider, kiroStartBody({
      method: kiroMethod,
      startUrl: kiroStartUrl,
      region: kiroRegion,
    }))
  }

  const pollOAuthStatus = (provider: OAuthProvider, state: string, deviceFlow: boolean) => {
    const deadline = Date.now() + (deviceFlow ? 30 : 6) * 60 * 1000
    let pollCount = 0
    let lastSeenWait = false
    const delay = deviceFlow ? 5000 : 2000

    const poll = async () => {
      pollCount++

      if (Date.now() > deadline) {
        setSessions(prev => ({
          ...prev,
          [provider.key]: { state: "error", message: deviceFlow ? "设备码登录超时（30 分钟）" : "OAuth 登录超时（前端 6 分钟限制）" }
        }))
        return
      }

      try {
        if (deviceFlow && provider.gatewayCallbackProvider) {
          await pollDeviceOAuth(provider.gatewayCallbackProvider, { state })
        }
        const data = await fetchProviderConfig<{
          status?: string
          error?: string
          message?: string
          user_code?: string
          verification_uri?: string
        }>(`/get-auth-status?state=${encodeURIComponent(state)}`)
        const status = data?.status

        if (data?.user_code) {
          setSessions(prev => ({
            ...prev,
            [provider.key]: {
              ...(prev[provider.key] || { state: "polling" }),
              state: "polling",
              userCode: data.user_code,
              verificationUri: data.verification_uri,
              oauthState: state,
            }
          }))
        }

        if (status === "wait") {
          lastSeenWait = true
          pollTimerRef.current[provider.key] = setTimeout(poll, delay)
          return
        }

        if (status === "error") {
          setSessions(prev => ({
            ...prev,
            [provider.key]: { state: "error", message: data?.message || data?.error || "OAuth 认证失败" }
          }))
          toast.error(`${provider.name}: ${data?.message || data?.error || "认证失败"}`)
          loadAuthFiles()
          return
        }

        if (status === "success" || lastSeenWait) {
          setSessions(prev => ({
            ...prev,
            [provider.key]: { state: "success", message: "OAuth 认证完成" }
          }))
          toast.success(`${provider.name} 已连接到 AI-GateWay`)
          loadAuthFiles()
          return
        }

        if (pollCount <= 3) {
          pollTimerRef.current[provider.key] = setTimeout(poll, delay)
          return
        }

        setSessions(prev => ({
          ...prev,
          [provider.key]: { state: "success", message: "OAuth 认证完成" }
        }))
        toast.success(`${provider.name} 已连接到 AI-GateWay`)
        loadAuthFiles()
      } catch {
        pollTimerRef.current[provider.key] = setTimeout(poll, deviceFlow ? 5000 : 3000)
      }
    }

    pollTimerRef.current[provider.key] = setTimeout(poll, delay)
  }

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
          setSessions(prev => ({
            ...prev,
            [provider.key]: { state: "success", message: "OAuth 认证完成" }
          }))
          toast.success(`${provider.name} 已连接到 AI-GateWay`)
          return
        }
        pollTimerRef.current[provider.key] = setTimeout(poll, 3000)
      } catch {
        pollTimerRef.current[provider.key] = setTimeout(poll, 3000)
      }
    }

    pollTimerRef.current[provider.key] = setTimeout(poll, 3000)
  }

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
          toast.warning('请粘贴包含 code 与 state 的完整回调 URL')
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
        pollOAuthStatus(provider, session.oauthState, Boolean(provider.deviceFlow))
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

  const disconnectFile = async (file: AuthFile) => {
    setDisconnecting((prev) => ({ ...prev, [file.name]: true }))
    try {
      await deleteAuthFile(file.name)
      toast.success(`已断开 ${file.label || file.email || file.name}`)
      await loadAuthFiles()
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : '断开失败')
    } finally {
      setDisconnecting((prev) => ({ ...prev, [file.name]: false }))
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

      <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
        {oauthProviders.map(provider => {
          const session = sessions[provider.key]
          const providerFiles = getProviderAuthFiles(provider.key)
          const hasCredentials = providerFiles.some(f => !f.disabled && f.status !== 'disabled')
          const showManual = provider.manualCallbackMode !== 'none' && provider.key !== 'kiro'

          return (
            <div key={provider.key} className="glass-card flex flex-col group overflow-hidden">
              <div className="p-6 flex-1 flex flex-col">
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
                  {providerFiles.length > 0 && (
                    <ul className="mt-2 space-y-1.5">
                      {providerFiles.map(file => (
                        <li key={file.name} className="flex items-center justify-between gap-2 text-[11px] text-gray-600 dark:text-gray-400">
                          <span className="truncate">{file.label || file.email || file.name}</span>
                          <button
                            type="button"
                            onClick={() => void disconnectFile(file)}
                            disabled={Boolean(disconnecting[file.name])}
                            className="shrink-0 text-red-500 hover:underline disabled:opacity-50"
                          >
                            断开
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>

                {provider.key === 'xai' && (
                  <p className="mb-4 text-[11px] leading-relaxed text-gray-500 dark:text-gray-400">
                    xAI Grok 使用设备码登录。AI-GateWay 会显示 user code，请在浏览器打开验证链接并输入。
                  </p>
                )}

                {provider.key === 'kiro' && (
                  <div className="mb-4 space-y-2">
                    <label className="text-[11px] font-medium text-gray-500 dark:text-gray-400">登录方式</label>
                    <select
                      value={kiroMethod}
                      onChange={(e) => setKiroMethod(e.target.value as KiroAuthMethod)}
                      className="w-full h-9 rounded-md border border-border bg-white dark:bg-dark-900 px-2 text-xs"
                    >
                      <option value="device">Builder ID 设备码</option>
                      <option value="authcode">Builder ID 授权码</option>
                      <option value="idc">IAM Identity Center (IDC)</option>
                      <option value="import">导入 Kiro IDE 缓存 JSON</option>
                    </select>
                    {kiroMethod === 'idc' && (
                      <>
                        <Input
                          value={kiroStartUrl}
                          onChange={(e) => setKiroStartUrl(e.target.value)}
                          placeholder="https://d-xxxxxxxxxx.awsapps.com/start"
                          className="text-xs h-9 font-mono"
                        />
                        <Input
                          value={kiroRegion}
                          onChange={(e) => setKiroRegion(e.target.value)}
                          placeholder="us-east-1"
                          className="text-xs h-9 font-mono"
                        />
                      </>
                    )}
                    {kiroMethod === 'import' && (
                      <>
                        <textarea
                          value={kiroImport}
                          onChange={(e) => setKiroImport(e.target.value)}
                          placeholder='{"access_token":"...","refresh_token":"..."}'
                          className="w-full min-h-[88px] rounded-md border border-border bg-white dark:bg-dark-900 p-2 text-[11px] font-mono"
                        />
                        <p className="text-[11px] text-gray-400">
                          从本机 Kiro IDE 的 AWS SSO 缓存（通常是 ~/.aws/sso/cache/）复制 JSON 粘贴。AI-GateWay 不会读取该路径。
                        </p>
                      </>
                    )}
                  </div>
                )}

                {showManual ? (
                  <details className="mb-4 group rounded-lg border border-dashed border-gray-200 dark:border-dark-700 bg-white/60 dark:bg-dark-900/30">
                    <summary className="cursor-pointer list-none px-3 py-2 text-xs font-medium text-gray-600 dark:text-gray-400 flex items-center gap-1.5 select-none">
                      <ClipboardPaste className="h-3.5 w-3.5 shrink-0 text-primary-500" />
                      <span>无法自动回调？手动粘贴授权结果</span>
                    </summary>
                    <div className="px-3 pb-3 space-y-2 border-t border-gray-100 dark:border-dark-700 pt-2">
                      <p className="text-[11px] leading-relaxed text-gray-500 dark:text-gray-400">
                        {provider.manualCallbackMode === 'sdk'
                          ? '在浏览器完成登录后，将地址栏完整回调 URL 粘贴到下方。'
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
                        placeholder="https://.../callback?code=...&state=..."
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
                ) : provider.key === 'kimi' ? (
                  <p className="mb-4 text-[11px] text-gray-400 dark:text-gray-500">
                    Kimi 使用设备码流程，请在弹窗中完成授权，无需手动回填。
                  </p>
                ) : null}

                <div className="mt-auto">
                  {session?.state === "polling" ? (
                    <div className="space-y-3">
                      {session.userCode && (
                        <div className="rounded-lg border border-primary-200 dark:border-primary-800 bg-primary-50/60 dark:bg-primary-900/20 p-3 text-center">
                          <p className="text-[11px] text-gray-500 dark:text-gray-400 mb-1">设备码</p>
                          <p className="text-xl font-mono font-bold tracking-widest text-gray-900 dark:text-white">
                            {session.userCode}
                          </p>
                        </div>
                      )}
                      <div className="flex items-center gap-2 text-sm text-primary-600 dark:text-primary-400">
                        <Loader2 className="h-4 w-4 animate-spin" />
                        <span>{session.userCode ? "等待设备码授权完成..." : "等待浏览器授权完成..."}</span>
                      </div>
                      {session.authURL && (
                        <a
                          href={session.authURL}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-xs text-primary-500 hover:underline flex items-center gap-1 truncate"
                        >
                          <ExternalLink className="h-3 w-3 flex-shrink-0" />
                          <span className="truncate">打开验证页面</span>
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
                      onClick={() => provider.key === 'kiro' ? void startKiro() : void startOAuth(provider)}
                      className="btn btn-sm w-full btn-primary"
                    >
                      <Globe className="h-4 w-4 mr-1.5" />
                      {provider.key === 'kiro' && kiroMethod === 'import' ? '导入并连接' : '发起 OAuth 登录'}
                    </button>
                  )}
                </div>
              </div>
            </div>
          )
        })}
      </div>

      <div className="p-4 bg-gray-50 dark:bg-dark-900/50 rounded-xl border border-border text-sm text-gray-500 dark:text-gray-400 space-y-1">
        <p className="flex items-center gap-2">
          <Shield className="h-4 w-4 text-primary-500" />
          <span>
            OAuth 凭证由 AI-GateWay 加密保存在 auth 库中。可在
            <Link to={adminChannelsTab('credentials')} className="text-primary-500 hover:underline font-medium mx-1">
              「代理账池 (凭证管理)」
            </Link>
            页面查看详情。xAI 走 OpenAI 兼容通道（模型前缀 xai/ 或 grok/）；Kiro 令牌可刷新并导出。
          </span>
        </p>
      </div>
    </div>
  )
}
