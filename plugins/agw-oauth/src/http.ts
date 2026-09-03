import { currentWatch, startLogin } from './command.js'
import { importLocalAndMaybeUpload } from './local-import.js'

export interface HttpAuth {
  origin: string
  apiKey: string
}

export interface HttpDeps {
  config: { origin: string }
  persist: (apiKey: string, origin: string) => Promise<void>
  token: () => HttpAuth | undefined
  savedOrigin: () => string | undefined
  saveOrigin: (origin: string) => Promise<void>
  logout: () => Promise<void>
}

export interface HttpRequest {
  url?: string
  method?: string
  on?(event: string, listener: (...args: unknown[]) => void): unknown
}

export interface HttpResponse {
  setHeader(name: string, value: string): void
  end(body: string): void
  statusCode: number
}

function storedOrigin(deps: HttpDeps): string | undefined {
  return deps.token()?.origin ?? deps.savedOrigin()
}

function resolveLoginOrigin(config: { origin: string }, stored?: string): string {
  return (stored ?? config.origin ?? process.env.AGW_ORIGIN ?? "").trim().replace(/\/+$/, "")
}

export function normalizeSavedOrigin(raw: unknown): string {
  if (typeof raw !== 'string') throw new Error('origin must be a string')
  const trimmed = raw.trim().replace(/\/+$/, '')
  if (trimmed.length === 0) throw new Error('origin is required')
  let parsed: URL
  try {
    parsed = new URL(trimmed)
  } catch {
    throw new Error('origin must be an http(s) URL')
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error('origin must be an http(s) URL')
  }
  return trimmed
}

export async function readJsonBody(req: HttpRequest): Promise<unknown> {
  const raw = await readBody(req)
  if (raw.length === 0) return undefined
  try {
    return JSON.parse(raw) as unknown
  } catch {
    throw new Error('invalid json')
  }
}

function readBody(req: HttpRequest): Promise<string> {
  const on = req.on
  if (typeof on !== 'function') return Promise.resolve('')
  return new Promise((resolve, reject) => {
    const chunks: string[] = []
    on.call(req, 'data', (chunk: unknown) => {
      chunks.push(typeof chunk === 'string' ? chunk : String(chunk))
    })
    on.call(req, 'end', () => resolve(chunks.join('')))
    on.call(req, 'error', (error: unknown) => reject(error instanceof Error ? error : new Error(String(error))))
  })
}

export async function handleHttp(req: HttpRequest, res: HttpResponse, deps: HttpDeps): Promise<void> {
  res.setHeader('content-type', 'application/json')
  const path = (req.url ?? '').split('?')[0] ?? ''
  const method = (req.method ?? 'GET').toUpperCase()
  try {
    if (method === 'POST' && path.endsWith('/login/start')) {
      const result = await startLogin(resolveLoginOrigin(deps.config, storedOrigin(deps)), deps.persist)
      res.end(JSON.stringify(result))
      return
    }
    if (method === 'GET' && path.endsWith('/status')) {
      const current = deps.token()
      res.end(JSON.stringify({
        loggedIn: current !== undefined,
        origin: storedOrigin(deps) || deps.config.origin || undefined,
        watch: currentWatch(),
      }))
      return
    }
    if (method === 'POST' && path.endsWith('/logout')) {
      await deps.logout()
      res.end(JSON.stringify({ ok: true, loggedIn: false }))
      return
    }
    if (method === 'POST' && path.endsWith('/import-local')) {
      const current = deps.token()
      const report = await importLocalAndMaybeUpload({
        origin: current?.origin ?? storedOrigin(deps),
        apiKey: current?.apiKey,
      })
      res.end(JSON.stringify(report))
      return
    }
    if (method === 'POST' && path.endsWith('/origin')) {
      const body = await readJsonBody(req)
      const origin = normalizeSavedOrigin(
        body !== null && typeof body === 'object' && !Array.isArray(body)
          ? (body as { origin?: unknown }).origin
          : undefined,
      )
      await deps.saveOrigin(origin)
      res.end(JSON.stringify({ ok: true, origin }))
      return
    }
    res.statusCode = 404
    res.end(JSON.stringify({ error: 'not found' }))
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    res.statusCode = message === 'invalid json' || message.startsWith('origin ') ? 400 : 500
    res.end(JSON.stringify({ error: message }))
  }
}
