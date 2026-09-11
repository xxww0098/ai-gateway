/** Parse pasted OAuth callback URL / query for manual backfill. */

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

type ParsedOAuthCallback = {
  code: string | null
  state: string | null
  error: string | null
  redirectUrl: string | null
}

export const parseOAuthCallbackInput = (
  input: string,
  options?: { sessionState?: string }
): ParsedOAuthCallback => {
  const trimmed = input.trim()
  if (!trimmed) {
    return { code: null, state: null, error: null, redirectUrl: null }
  }

  if (trimmed.includes('#') && !trimmed.startsWith('http')) {
    // 只在第一个 `#` 上切，剩下的全归 state —— 与后端 `split_claude_code`
    // 的 `split_once('#')` 一致；`split('#', 2)` 会把后面的部分丢掉。
    const hash = trimmed.indexOf('#')
    const codePart = trimmed.slice(0, hash)
    const statePart = trimmed.slice(hash + 1)
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
