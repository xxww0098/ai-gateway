/** Parse device-code OAuth start / status payloads for the AI-GateWay panel. */

export type DeviceAuthStart = {
  state: string
  userCode: string
  verificationUri: string
  verificationUriComplete?: string
  expiresIn?: number
  interval?: number
  flow: 'device'
}

const asRecord = (data: unknown): Record<string, unknown> | null => {
  if (!data || typeof data !== 'object' || Array.isArray(data)) return null
  return data as Record<string, unknown>
}

const text = (value: unknown): string => (typeof value === 'string' ? value.trim() : '')

const numberField = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) ? value : undefined

/** True when the auth-url response is a device-code start (has user_code). */
export function isDeviceFlowResponse(data: unknown): boolean {
  return parseDeviceAuthStart(data) !== null
}

export function parseDeviceAuthStart(data: unknown): DeviceAuthStart | null {
  const object = asRecord(data)
  if (!object) return null
  const userCode = text(object.user_code)
  if (!userCode) return null
  const complete = text(object.verification_uri_complete)
  const verificationUri = text(object.verification_uri)
  const url = text(object.url) || text(object.auth_url)
  return {
    state: text(object.state),
    userCode,
    verificationUri: verificationUri || complete || url,
    verificationUriComplete: complete || undefined,
    expiresIn: numberField(object.expires_in),
    interval: numberField(object.interval),
    flow: 'device',
  }
}

export function parseImportJson(raw: string): { ok: true; token: unknown } | { ok: false; error: string } {
  const trimmed = raw.trim()
  if (!trimmed) {
    return { ok: false, error: '请粘贴 Kiro IDE 缓存 JSON' }
  }
  try {
    const parsed = JSON.parse(trimmed) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { ok: false, error: '导入内容必须是 JSON 对象' }
    }
    const record = parsed as Record<string, unknown>
    const access = text(record.access_token) || text(record.accessToken)
    if (!access) {
      return { ok: false, error: 'JSON 缺少 access_token' }
    }
    return { ok: true, token: parsed }
  } catch {
    return { ok: false, error: 'JSON 无法解析' }
  }
}

export type KiroAuthMethod = 'device' | 'authcode' | 'idc' | 'import'

export function kiroStartBody(input: {
  method: KiroAuthMethod
  startUrl?: string
  region?: string
  token?: unknown
}): Record<string, unknown> {
  if (input.method === 'import') {
    return { method: 'import', token: input.token ?? {} }
  }
  const body: Record<string, unknown> = { method: input.method }
  if (input.startUrl?.trim()) body.start_url = input.startUrl.trim()
  if (input.region?.trim()) body.region = input.region.trim()
  return body
}
