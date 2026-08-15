/** Parse pasted OAuth callback URL / query for manual backfill. */

const XAI_CALLBACK_URL = 'http://127.0.0.1:56121/callback'

const isAbsoluteUrl = (value: string): boolean => {
  try {
    new URL(value)
    return true
  } catch {
    return false
  }
}

const readQueryLikeCallbackInput = (value: string): URLSearchParams | null => {
  const trimmed = value.trim()
  if (!trimmed) return null
  const queryStart = trimmed.indexOf('?')
  const hashStart = trimmed.indexOf('#')
  const rawParams =
    queryStart >= 0
      ? trimmed.slice(queryStart + 1)
      : hashStart >= 0
        ? trimmed.slice(hashStart + 1)
        : trimmed

  if (!/(^|[&#?])(code|state|error)=/i.test(rawParams)) return null
  return new URLSearchParams(rawParams.replace(/^[?#]/, ''))
}

const extractDisplayedXaiCode = (value: string): string => {
  const trimmed = value.trim()
  const codeMatch = trimmed.match(/\bcode\s*[:=]\s*([^\s&]+)/i)
  return (codeMatch?.[1] ?? trimmed).trim()
}

type ParsedOAuthCallback = {
  code: string | null
  state: string | null
  error: string | null
  redirectUrl: string | null
}

const buildXaiRedirectUrl = (input: string, sessionState?: string): string | null => {
  const trimmed = input.trim()
  if (!trimmed) return null
  if (isAbsoluteUrl(trimmed)) return trimmed

  const params = readQueryLikeCallbackInput(trimmed)
  if (params) {
    const code = params.get('code')?.trim()
    const error = params.get('error')?.trim()
    const errorDescription = params.get('error_description')?.trim()
    const callbackState = params.get('state')?.trim() || sessionState?.trim()
    if (!callbackState) return null

    const callbackUrl = new URL(XAI_CALLBACK_URL)
    callbackUrl.searchParams.set('state', callbackState)
    if (code) callbackUrl.searchParams.set('code', code)
    if (error) callbackUrl.searchParams.set('error', error)
    if (errorDescription) callbackUrl.searchParams.set('error_description', errorDescription)
    return callbackUrl.toString()
  }

  const code = extractDisplayedXaiCode(trimmed)
  const callbackState = sessionState?.trim()
  if (!code || !callbackState) return null

  const callbackUrl = new URL(XAI_CALLBACK_URL)
  callbackUrl.searchParams.set('code', code)
  callbackUrl.searchParams.set('state', callbackState)
  return callbackUrl.toString()
}

export const parseOAuthCallbackInput = (
  input: string,
  options?: { sessionState?: string; isXai?: boolean }
): ParsedOAuthCallback => {
  const trimmed = input.trim()
  if (!trimmed) {
    return { code: null, state: null, error: null, redirectUrl: null }
  }

  if (options?.isXai) {
    const redirectUrl = buildXaiRedirectUrl(trimmed, options.sessionState)
    if (!redirectUrl) {
      return { code: null, state: null, error: null, redirectUrl: null }
    }
    try {
      const url = new URL(redirectUrl)
      return {
        code: url.searchParams.get('code'),
        state: url.searchParams.get('state'),
        error: url.searchParams.get('error') || url.searchParams.get('error_description'),
        redirectUrl,
      }
    } catch {
      return { code: null, state: null, error: null, redirectUrl: null }
    }
  }

  if (trimmed.includes('#') && !trimmed.startsWith('http')) {
    const [codePart, statePart] = trimmed.split('#', 2)
    return {
      code: codePart.trim() || null,
      state: statePart.trim() || options?.sessionState?.trim() || null,
      error: null,
      redirectUrl: null,
    }
  }

  if (isAbsoluteUrl(trimmed)) {
    try {
      const url = new URL(trimmed)
      return {
        code: url.searchParams.get('code'),
        state: url.searchParams.get('state') || options?.sessionState || null,
        error: url.searchParams.get('error') || url.searchParams.get('error_description'),
        redirectUrl: trimmed,
      }
    } catch {
      return { code: null, state: null, error: null, redirectUrl: null }
    }
  }

  const params = readQueryLikeCallbackInput(trimmed)
  if (params) {
    return {
      code: params.get('code'),
      state: params.get('state') || options?.sessionState || null,
      error: params.get('error') || params.get('error_description'),
      redirectUrl: null,
    }
  }

  return { code: null, state: null, error: null, redirectUrl: null }
}
