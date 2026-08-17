import type { ApiCallRequest, ApiCallResult } from '@/features/admin-proxy/api'

type AmpcodeUpstreamTestStatus = 'connected' | 'reachable' | 'failed'

export interface AmpcodeUpstreamTestInput {
  upstreamUrl: string
  upstreamApiKey: string
}

export interface AmpcodeUpstreamTestResult {
  status: AmpcodeUpstreamTestStatus
  message: string
  endpoint: string
  checkedAt: string
  statusCode?: number
  elapsedMs?: number
  bodyPreview?: string
}

export type AmpcodeApiCall = (payload: ApiCallRequest) => Promise<ApiCallResult>

interface EndpointAttempt {
  endpoint: string
  ok: boolean
  elapsedMs?: number
  statusCode?: number
  bodyPreview?: string
  error?: string
}

const PROBE_HEADERS = {
  Accept: 'application/json',
  'User-Agent': 'AI-GateWay-Upstream-Probe/1.0',
}

const isHttpUrl = (url: string) => /^https?:\/\//i.test(url)

const trimTrailingSlash = (value: string) => value.replace(/\/+$/g, '')

const joinUrl = (base: string, path: string) =>
  `${trimTrailingSlash(base)}/${path.replace(/^\/+/g, '')}`

export function normalizeAmpcodeGatewayBase(value: string): string {
  const raw = String(value ?? '').trim()
  if (!raw) return ''

  const stripKnownSuffixes = (input: string) => trimTrailingSlash(input)
    .replace(/\/api\/(user|auth|threads|internal|meta|telemetry|ads|otel|tab).*$/i, '')
    .replace(/\/v1\/chat\/completions$/i, '')
    .replace(/\/chat\/completions$/i, '')
    .replace(/\/v1\/models$/i, '')
    .replace(/\/models$/i, '')
    .replace(/\/v1$/i, '')

  try {
    const url = new URL(raw)
    url.pathname = stripKnownSuffixes(url.pathname)
    url.search = ''
    url.hash = ''
    return trimTrailingSlash(url.toString())
  } catch {
    return stripKnownSuffixes(raw)
  }
}

export function buildAmpcodeControlPlaneEndpoints(upstreamUrl: string): string[] {
  const base = normalizeAmpcodeGatewayBase(upstreamUrl)
  if (!base) return []

  return [
    joinUrl(base, '/api/user'),
    joinUrl(base, '/api/auth'),
  ]
}

function bodyToText(result: ApiCallResult): string {
  if (result.bodyText) return result.bodyText
  if (result.body === null || result.body === undefined) return ''
  if (typeof result.body === 'string') return result.body

  try {
    return JSON.stringify(result.body)
  } catch {
    return String(result.body)
  }
}

function redact(value: string, apiKey: string): string {
  const key = apiKey.trim()
  if (!key) return value
  return value.split(key).join('[API Key 已隐藏]')
}

function createPreview(result: ApiCallResult, apiKey: string): string | undefined {
  const text = redact(bodyToText(result), apiKey).trim()
  if (!text) return undefined
  return text.length > 240 ? `${text.slice(0, 240)}...` : text
}

async function probeEndpoint(
  request: AmpcodeApiCall,
  endpoint: string,
  headers: Record<string, string>,
  apiKey: string,
): Promise<EndpointAttempt> {
  const startedAt = Date.now()

  try {
    const result = await request({
      method: 'GET',
      url: endpoint,
      header: headers,
    })
    const elapsedMs = Date.now() - startedAt
    const ok = result.statusCode >= 200 && result.statusCode < 400

    return {
      endpoint,
      ok,
      elapsedMs,
      statusCode: result.statusCode,
      bodyPreview: createPreview(result, apiKey),
    }
  } catch (error) {
    return {
      endpoint,
      ok: false,
      elapsedMs: Date.now() - startedAt,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

const attemptSummary = (attempt?: EndpointAttempt) => {
  if (!attempt) return '请求未完成'
  if (attempt.statusCode) return `HTTP ${attempt.statusCode}`
  return attempt.error || '请求未完成'
}

function resultFromAttempt(
  status: AmpcodeUpstreamTestStatus,
  message: string,
  attempt: EndpointAttempt,
): AmpcodeUpstreamTestResult {
  return {
    status,
    message,
    endpoint: attempt.endpoint,
    checkedAt: new Date().toISOString(),
    statusCode: attempt.statusCode,
    elapsedMs: attempt.elapsedMs,
    bodyPreview: attempt.bodyPreview || attempt.error,
  }
}

export async function testAmpcodeUpstream(
  input: AmpcodeUpstreamTestInput,
  request: AmpcodeApiCall,
): Promise<AmpcodeUpstreamTestResult> {
  const upstreamUrl = normalizeAmpcodeGatewayBase(input.upstreamUrl)
  const upstreamApiKey = String(input.upstreamApiKey ?? '').trim()
  const checkedAt = new Date().toISOString()

  if (!upstreamUrl) {
    return {
      status: 'failed',
      message: '请输入上游地址',
      endpoint: '',
      checkedAt,
    }
  }

  if (!isHttpUrl(upstreamUrl)) {
    return {
      status: 'failed',
      message: '上游地址必须以 http:// 或 https:// 开头',
      endpoint: upstreamUrl,
      checkedAt,
    }
  }

  const headers: Record<string, string> = { ...PROBE_HEADERS }
  if (upstreamApiKey) {
    headers.Authorization = `Bearer ${upstreamApiKey}`
    headers['X-Api-Key'] = upstreamApiKey
  }

  const attempts: EndpointAttempt[] = []
  for (const endpoint of buildAmpcodeControlPlaneEndpoints(upstreamUrl)) {
    const attempt = await probeEndpoint(request, endpoint, headers, upstreamApiKey)
    attempts.push(attempt)

    if (attempt.ok) {
      return resultFromAttempt('connected', '连接成功，Amp control-plane 已响应', attempt)
    }

    if (attempt.statusCode === 401 || attempt.statusCode === 403) {
      return resultFromAttempt(
        'failed',
        `上游已响应，但认证失败或访问被拒绝：${attemptSummary(attempt)}`,
        attempt,
      )
    }

    if (attempt.statusCode && attempt.statusCode !== 404) {
      return resultFromAttempt(
        'reachable',
        `上游网络可达，但 Amp control-plane 未确认可用：${attemptSummary(attempt)}`,
        attempt,
      )
    }
  }

  const lastAttempt = attempts[attempts.length - 1]
  return resultFromAttempt(
    'failed',
    `上游连接失败或不是 Amp control-plane：${attemptSummary(lastAttempt)}`,
    lastAttempt || {
      endpoint: upstreamUrl,
      ok: false,
      error: '请求未完成',
    },
  )
}
