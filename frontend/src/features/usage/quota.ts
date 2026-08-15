/**
 * 通过 SDK 的 /v0/management/api-call 代理端点获取各 Provider 的账号额度信息。
 *
 * 原理：SDK 会使用指定 authIndex 对应的凭证，代理发出 HTTP 请求到 Provider 的官方 API，
 * 然后将响应数据原样返回给前端。
 */

import { fetchMgmtApi } from '@/features/admin-proxy/api'

// ── Types ──

interface QuotaWindow {
  id: string
  label: string
  usedPercent: number | null   // 已使用比例 0–100
  resetLabel: string           // 重置时间描述
}

export interface QuotaResult {
  provider: string
  planLabel?: string | null
  windows: QuotaWindow[]
  error?: string
}

interface ApiCallPayload {
  authIndex: string
  auth_index: string
  method: string
  url: string
  header: Record<string, string>
  data?: string
}

interface ApiCallResult {
  status_code?: number
  statusCode?: number
  body?: unknown
  bodyText?: string
}

// ── Provider Constants ──

const CLAUDE_USAGE_URL = 'https://api.anthropic.com/api/oauth/usage'
const CLAUDE_HEADERS = {
  Authorization: 'Bearer $TOKEN$',
  'Content-Type': 'application/json',
  'anthropic-beta': 'oauth-2025-04-20',
}

const CODEX_USAGE_URL = 'https://chatgpt.com/backend-api/wham/usage'
const CODEX_HEADERS = {
  Authorization: 'Bearer $TOKEN$',
  'Content-Type': 'application/json',
  'User-Agent': 'codex_cli_rs/0.76.0 (Debian 13.0.0; x86_64) WindowsTerminal',
}

const GEMINI_QUOTA_URL = 'https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota'
const GEMINI_HEADERS = {
  Authorization: 'Bearer $TOKEN$',
  'Content-Type': 'application/json',
}

const ANTIGRAVITY_QUOTA_URLS = [
  'https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels',
  'https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels',
]
const ANTIGRAVITY_HEADERS = {
  Authorization: 'Bearer $TOKEN$',
  'Content-Type': 'application/json',
  'User-Agent': 'antigravity/1.11.5 windows/amd64',
}

// ── Core: proxy api-call ──

async function apiCall(payload: ApiCallPayload): Promise<ApiCallResult> {
  const res = await fetchMgmtApi('/api-call', {
    method: 'POST',
    body: JSON.stringify(payload),
  })
  return res as ApiCallResult
}

function getStatusCode(r: ApiCallResult): number {
  return Number(r.status_code ?? r.statusCode ?? 0)
}

function parseBody(r: ApiCallResult): unknown {
  if (r.body) return r.body
  if (typeof r.bodyText === 'string') {
    try { return JSON.parse(r.bodyText) } catch { return null }
  }
  return null
}

function formatResetTime(seconds?: number | null, resetAt?: string | null): string {
  if (resetAt) {
    try {
      const ms = new Date(resetAt).getTime() - Date.now()
      if (ms > 0) {
        const h = Math.floor(ms / 3600000)
        const m = Math.floor((ms % 3600000) / 60000)
        return h > 0 ? `${h}h${m}m` : `${m}m`
      }
    } catch { /* fall through */ }
  }
  if (seconds && seconds > 0) {
    const h = Math.floor(seconds / 3600)
    const m = Math.floor((seconds % 3600) / 60)
    return h > 0 ? `${h}h${m}m` : `${m}m`
  }
  return '—'
}

// ── Provider Implementations ──

async function fetchClaudeQuota(authIndex: string): Promise<QuotaResult> {
  const result = await apiCall({ authIndex, auth_index: authIndex, method: 'GET', url: CLAUDE_USAGE_URL, header: CLAUDE_HEADERS })
  const status = getStatusCode(result)
  if (status === 401 || status === 403) throw new Error(`Claude API 返回 ${status} (凭证可能已失效)`)
  if (status < 200 || status >= 300) throw new Error(`Claude API 返回 ${status}`)

  const body = parseBody(result) as Record<string, unknown> | null
  if (!body) throw new Error('Claude 返回数据为空')

  const WINDOW_KEYS = [
    { key: 'five_hour', label: '5h窗口' },
    { key: 'seven_day', label: '7天窗口' },
    { key: 'seven_day_opus', label: '7天 Opus' },
    { key: 'seven_day_sonnet', label: '7天 Sonnet' },
    { key: 'iguana_necktie', label: 'Iguana Necktie' },
  ]

  const windows: QuotaWindow[] = []
  for (const wk of WINDOW_KEYS) {
    const w = body[wk.key] as Record<string, unknown> | undefined
    if (!w) continue
    const pct = w.used_percent ?? w.usedPercent
    const resetSec = w.reset_after_seconds ?? w.resetAfterSeconds
    windows.push({
      id: wk.key,
      label: wk.label,
      usedPercent: typeof pct === 'number' ? Math.round(pct * 100) / 100 : null,
      resetLabel: formatResetTime(typeof resetSec === 'number' ? resetSec : null),
    })
  }

  return { provider: 'claude', windows }
}

async function fetchCodexQuota(authIndex: string, accountId?: string): Promise<QuotaResult> {
  const headers: Record<string, string> = { ...CODEX_HEADERS }
  if (accountId) headers['Chatgpt-Account-Id'] = accountId

  const result = await apiCall({ authIndex, auth_index: authIndex, method: 'GET', url: CODEX_USAGE_URL, header: headers })
  const status = getStatusCode(result)
  if (status === 401 || status === 403) throw new Error(`Codex API 返回 ${status} (凭证可能已失效)`)
  if (status < 200 || status >= 300) throw new Error(`Codex API 返回 ${status}`)

  const body = parseBody(result) as Record<string, unknown> | null
  if (!body) throw new Error('Codex 返回数据为空')

  const planType = (body.plan_type ?? body.planType) as string | undefined
  const rateLimit = (body.rate_limit ?? body.rateLimit) as Record<string, unknown> | undefined
  const windows: QuotaWindow[] = []

  if (rateLimit) {
    const primary = (rateLimit.primary_window ?? rateLimit.primaryWindow) as Record<string, unknown> | undefined
    const secondary = (rateLimit.secondary_window ?? rateLimit.secondaryWindow) as Record<string, unknown> | undefined

    if (primary) {
      const pct = primary.used_percent ?? primary.usedPercent
      const resetSec = primary.reset_after_seconds ?? primary.resetAfterSeconds
      windows.push({
        id: 'primary',
        label: '5h窗口',
        usedPercent: typeof pct === 'number' ? pct : null,
        resetLabel: formatResetTime(typeof resetSec === 'number' ? resetSec : null),
      })
    }
    if (secondary) {
      const pct = secondary.used_percent ?? secondary.usedPercent
      const resetSec = secondary.reset_after_seconds ?? secondary.resetAfterSeconds
      windows.push({
        id: 'secondary',
        label: '7天窗口',
        usedPercent: typeof pct === 'number' ? pct : null,
        resetLabel: formatResetTime(typeof resetSec === 'number' ? resetSec : null),
      })
    }
  }

  return { provider: 'codex', planLabel: planType || null, windows }
}

async function fetchGeminiQuota(authIndex: string, projectId?: string): Promise<QuotaResult> {
  const project = projectId || 'bamboo-precept-lgxtn'
  const result = await apiCall({
    authIndex, auth_index: authIndex, method: 'POST', url: GEMINI_QUOTA_URL,
    header: GEMINI_HEADERS,
    data: JSON.stringify({ project }),
  })
  const status = getStatusCode(result)
  if (status === 401 || status === 403) throw new Error(`Gemini API 返回 ${status} (凭证可能已失效)`)
  if (status < 200 || status >= 300) throw new Error(`Gemini API 返回 ${status}`)

  const body = parseBody(result) as Record<string, unknown> | null
  if (!body) throw new Error('Gemini 返回数据为空')

  const buckets = (body.buckets as Array<Record<string, unknown>>) || []
  const windows: QuotaWindow[] = []

  for (const b of buckets) {
    const modelId = (b.modelId ?? b.model_id) as string | undefined
    if (!modelId) continue
    const frac = b.remainingFraction ?? b.remaining_fraction
    const resetTime = (b.resetTime ?? b.reset_time) as string | undefined
    const remaining = typeof frac === 'number' ? Math.round(frac * 100) : null

    windows.push({
      id: modelId,
      label: modelId,
      usedPercent: remaining !== null ? 100 - remaining : null,
      resetLabel: formatResetTime(null, resetTime || null),
    })
  }

  // Dedupe by grouping similar model names
  const grouped = new Map<string, QuotaWindow>()
  for (const w of windows) {
    const baseModel = w.label.replace(/-preview$/, '').replace(/-thinking$/, '')
    const existing = grouped.get(baseModel)
    if (!existing || (w.usedPercent !== null && (existing.usedPercent === null || w.usedPercent > existing.usedPercent))) {
      grouped.set(baseModel, { ...w, label: baseModel })
    }
  }

  return { provider: 'gemini-cli', windows: Array.from(grouped.values()).slice(0, 8) }
}

async function fetchAntigravityQuota(authIndex: string): Promise<QuotaResult> {
  let lastError = ''
  for (const url of ANTIGRAVITY_QUOTA_URLS) {
    try {
      const result = await apiCall({
        authIndex, auth_index: authIndex, method: 'POST', url,
        header: ANTIGRAVITY_HEADERS,
        data: JSON.stringify({ project: 'bamboo-precept-lgxtn' }),
      })
      const status = getStatusCode(result)
      if (status === 401 || status === 403) { lastError = `HTTP ${status} (凭证可能已失效)`; continue }
      if (status < 200 || status >= 300) { lastError = `HTTP ${status}`; continue }

      const body = parseBody(result) as Record<string, unknown> | null
      if (!body) { lastError = '返回数据为空'; continue }

      const models = body.models as Record<string, Record<string, unknown>> | undefined
      if (!models || typeof models !== 'object') { lastError = '无模型数据'; continue }

      const windows: QuotaWindow[] = []
      for (const [modelId, info] of Object.entries(models)) {
        const frac = info.remainingFraction ?? info.remaining_fraction
        const resetTime = (info.resetTime ?? info.reset_time) as string | undefined
        const remaining = typeof frac === 'number' ? Math.round(frac * 100) : null

        windows.push({
          id: modelId,
          label: modelId.length > 25 ? modelId.slice(0, 22) + '...' : modelId,
          usedPercent: remaining !== null ? 100 - remaining : null,
          resetLabel: formatResetTime(null, resetTime || null),
        })
      }

      return { provider: 'antigravity', windows: windows.slice(0, 10) }
    } catch (e) {
      lastError = e instanceof Error ? e.message : '未知错误'
    }
  }
  throw new Error(lastError || 'Antigravity 额度查询失败')
}

// ── Public API ──

export interface AuthFileForQuota {
  name: string
  provider?: string
  type?: string
  auth_index?: string | number
  authIndex?: string | number
  chatgpt_account_id?: string
  project_id?: string
}

function resolveAuthIndex(file: AuthFileForQuota): string | null {
  const raw = file.auth_index ?? file.authIndex
  if (raw === undefined || raw === null) return null
  return String(raw)
}

function resolveProvider(file: AuthFileForQuota): string {
  const p = (file.provider || file.type || '').toLowerCase()
  if (p.includes('claude') || p.includes('anthropic')) return 'claude'
  if (p.includes('codex') || p.includes('openai') || p.includes('chatgpt')) return 'codex'
  if (p.includes('gemini') && p.includes('cli')) return 'gemini-cli'
  if (p.includes('gemini') || p.includes('google') || p.includes('aistudio')) return 'gemini-cli'
  if (p.includes('antigravity')) return 'antigravity'
  return p
}

export async function fetchQuotaForFile(file: AuthFileForQuota): Promise<QuotaResult> {
  const authIndex = resolveAuthIndex(file)
  if (!authIndex) return { provider: resolveProvider(file), windows: [], error: '未找到 auth_index' }

  const provider = resolveProvider(file)

  switch (provider) {
    case 'claude':
      return fetchClaudeQuota(authIndex)
    case 'codex':
      return fetchCodexQuota(authIndex, file.chatgpt_account_id)
    case 'gemini-cli':
      return fetchGeminiQuota(authIndex, file.project_id)
    case 'antigravity':
      return fetchAntigravityQuota(authIndex)
    default:
      return { provider, windows: [], error: `不支持 ${provider} 的额度查询` }
  }
}
