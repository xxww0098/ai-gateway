import { useCallback, useEffect, useState } from 'react'
import type { AgwKey } from './locales.ts'

export interface AgwSectionInjected {
  t: (key: keyof AgwKey) => string
}

interface StatusPayload {
  loggedIn?: boolean
  origin?: string
  watch?: {
    status?: string
    detail?: string
    openUrl?: string
    userCode?: string
  }
}

interface StartPayload {
  kind?: string
  text?: string
  openUrl?: string
  userCode?: string
  error?: string
}

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: 'same-origin',
    ...init,
    headers: {
      accept: 'application/json',
      ...(init?.body === undefined ? {} : { 'content-type': 'application/json' }),
      ...init?.headers,
    },
  })
  let json: unknown = undefined
  try {
    json = await response.json()
  } catch {
    json = undefined
  }
  if (!response.ok) {
    const message = json !== null && typeof json === 'object' && 'error' in json
      && typeof (json as { error: unknown }).error === 'string'
      ? (json as { error: string }).error
      : `HTTP ${response.status}`
    throw new Error(message)
  }
  return json as T
}

const page: Record<string, unknown> = {
  display: 'flex',
  flexDirection: 'column',
  gap: 16,
  maxWidth: 560,
  padding: '8px 0',
}

const titleStyle: Record<string, unknown> = {
  margin: 0,
  fontSize: 20,
  fontWeight: 600,
}

const descStyle: Record<string, unknown> = {
  margin: 0,
  opacity: 0.75,
  lineHeight: 1.5,
}

const fieldStyle: Record<string, unknown> = {
  display: 'flex',
  flexDirection: 'column',
  gap: 6,
}

const inputStyle: Record<string, unknown> = {
  padding: '8px 10px',
  borderRadius: 8,
  border: '1px solid rgba(127,127,127,0.35)',
  background: 'transparent',
  color: 'inherit',
  fontSize: 14,
}

const rowStyle: Record<string, unknown> = {
  display: 'flex',
  alignItems: 'center',
  gap: 10,
}

const btnStyle: Record<string, unknown> = {
  padding: '8px 14px',
  borderRadius: 8,
  border: '1px solid rgba(127,127,127,0.35)',
  background: 'transparent',
  color: 'inherit',
  cursor: 'pointer',
  fontSize: 14,
}

const codeStyle: Record<string, unknown> = {
  fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
  fontSize: 16,
  letterSpacing: 1,
  padding: '6px 10px',
  borderRadius: 6,
  background: 'rgba(127,127,127,0.12)',
}

export function AgwSection(props: Partial<AgwSectionInjected>): JSX.Element {
  const t = (key: keyof AgwKey, fallback: string): string => props.t?.(key) ?? fallback
  const [origin, setOrigin] = useState('')
  const [loggedIn, setLoggedIn] = useState(false)
  const [watch, setWatch] = useState<StatusPayload['watch']>()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const [start, setStart] = useState<StartPayload>()
  const [importText, setImportText] = useState<string>()

  const applyStatus = (payload: StatusPayload): void => {
    setLoggedIn(payload.loggedIn === true)
    if (typeof payload.origin === 'string') setOrigin(payload.origin)
    setWatch(payload.watch)
  }

  const refresh = useCallback(async (): Promise<StatusPayload> => {
    const payload = await api<StatusPayload>('/agw-oauth/status')
    applyStatus(payload)
    return payload
  }, [])

  useEffect(() => {
    void refresh().catch((err: unknown) => {
      setError(err instanceof Error ? err.message : String(err))
    })
  }, [refresh])

  const waiting = watch?.status === 'waiting' || (start !== undefined && !loggedIn && watch?.status !== 'error')

  useEffect(() => {
    if (!waiting) return
    const timer = setInterval(() => {
      void refresh().then((payload) => {
        if (payload.loggedIn === true || payload.watch?.status === 'ok' || payload.watch?.status === 'error') {
          setStart(undefined)
        }
      }).catch((err: unknown) => {
        setError(err instanceof Error ? err.message : String(err))
      })
    }, 2000)
    return () => clearInterval(timer)
  }, [waiting, refresh])

  const persistOrigin = async (): Promise<void> => {
    const trimmed = origin.trim()
    if (trimmed.length === 0) return
    await api('/agw-oauth/origin', { method: 'POST', body: JSON.stringify({ origin: trimmed }) })
  }

  const onLogin = async (): Promise<void> => {
    setBusy(true)
    setError(undefined)
    try {
      await persistOrigin()
      const result = await api<StartPayload>('/agw-oauth/login/start', { method: 'POST' })
      if (result.kind === 'error') {
        setError(result.text ?? result.error ?? t('error', 'Something went wrong'))
        return
      }
      setStart(result)
      await refresh()
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  const onImport = async (): Promise<void> => {
    setBusy(true)
    setError(undefined)
    try {
      const report = await api<{
        found?: Array<{ provider?: string, source?: string, hasAccessToken?: boolean }>
        uploaded?: { status?: number }
        error?: string
      }>('/agw-oauth/import-local', { method: 'POST' })
      const lines = (report.found ?? []).map(row => `${row.provider ?? '?'} · ${row.source ?? ''}`)
      if (report.uploaded?.status !== undefined) lines.push(`Upload HTTP ${report.uploaded.status}`)
      if (report.error) lines.push(report.error)
      setImportText(lines.join('\n') || t('importHint', 'No local CLI files found.'))
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  const onLogout = async (): Promise<void> => {
    setBusy(true)
    setError(undefined)
    try {
      await api('/agw-oauth/logout', { method: 'POST' })
      setStart(undefined)
      setWatch(undefined)
      setLoggedIn(false)
      await refresh()
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  const openUrl = start?.openUrl ?? watch?.openUrl
  const userCode = start?.userCode ?? watch?.userCode
  const watchError = watch?.status === 'error' ? watch.detail : undefined

  return (
    <div style={page}>
      <h1 style={titleStyle}>{t('title', 'AGW Oauth')}</h1>
      <p style={descStyle}>{t('description', '连接 AI-GateWay 网关并通过浏览器安全登录。')}</p>
      <p style={descStyle}>{t('importHint', '已有 ~/.codex/auth.json 或 ~/.claude/.credentials.json 时，优先导入，不必再登录。')}</p>
      <label style={fieldStyle}>
        <span>{t('originLabel', '网关地址')}</span>
        <input
          style={inputStyle}
          value={origin}
          placeholder={t('originPlaceholder', 'https://gw.example.com')}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => setOrigin(event.target.value)}
          onBlur={() => { void persistOrigin().catch((err: unknown) => setError(err instanceof Error ? err.message : String(err))) }}
        />
      </label>
      <div style={rowStyle}>
        <span
          aria-hidden="true"
          style={{
            width: 8,
            height: 8,
            borderRadius: 999,
            background: loggedIn ? '#22c55e' : '#9ca3af',
            display: 'inline-block',
          }}
        />
        <span>{loggedIn ? t('loggedIn', '已登录 · OAuth 凭据可用') : t('loggedOut', '未登录')}</span>
      </div>
      <div style={rowStyle}>
        {loggedIn ? (
          <button type="button" style={btnStyle} disabled={busy} onClick={() => { void onLogout() }}>
            {t('logout', '退出登录')}
          </button>
        ) : (
          <button type="button" style={btnStyle} disabled={busy} onClick={() => { void onLogin() }}>
            {busy ? t('saving', '保存中…') : t('login', '登录')}
          </button>
        )}
        <button type="button" style={btnStyle} disabled={busy} onClick={() => { void onImport() }}>
          {t('importLocal', '导入本机 CLI 凭据')}
        </button>
      </div>
      {importText !== undefined ? (
        <pre style={{ ...descStyle, whiteSpace: 'pre-wrap' }}>{importText}</pre>
      ) : undefined}
      {(openUrl !== undefined || userCode !== undefined) && !loggedIn ? (
        <div style={{ ...fieldStyle, gap: 8 }}>
          <p style={{ ...descStyle, margin: 0 }}>{t('waiting', '请在浏览器中完成 AI-GateWay 登录。')}</p>
          {userCode !== undefined && userCode.length > 0 ? (
            <div style={rowStyle}>
              <span>{t('userCode', '用户码')}</span>
              <code style={codeStyle}>{userCode}</code>
            </div>
          ) : undefined}
          {openUrl !== undefined && openUrl.length > 0 ? (
            <div style={fieldStyle}>
              <span>{t('openUrl', '验证地址')}</span>
              <a href={openUrl} target="_blank" rel="noreferrer">{openUrl}</a>
            </div>
          ) : undefined}
        </div>
      ) : undefined}
      {error !== undefined || watchError !== undefined ? (
        <p style={{ ...descStyle, color: '#ef4444' }}>{error ?? watchError ?? t('error', '出错了')}</p>
      ) : undefined}
    </div>
  )
}
