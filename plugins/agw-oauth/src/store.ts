import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { homedir } from 'node:os'

export interface StoredToken {
  origin: string
  apiKey: string
}

export function defaultTokenPath(): string {
  const home = process.env.DSH_HOME?.trim() || join(homedir(), '.dsh')
  return join(home, 'agw-oauth.json')
}

function errCode(error: unknown): string | undefined {
  if (error !== null && typeof error === 'object' && 'code' in error) {
    const code = (error as { code?: unknown }).code
    return typeof code === 'string' ? code : undefined
  }
  return undefined
}

export class TokenStore {
  private cache: StoredToken | undefined
  constructor(readonly path: string = defaultTokenPath()) {}

  async read(): Promise<StoredToken | undefined> {
    if (this.cache !== undefined) return this.cache
    try {
      const raw = await readFile(this.path, 'utf8')
      const parsed: unknown = JSON.parse(raw)
      if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return undefined
      const origin = typeof (parsed as { origin?: unknown }).origin === 'string'
        ? (parsed as { origin: string }).origin.trim()
        : ''
      const apiKey = typeof (parsed as { api_key?: unknown }).api_key === 'string'
        ? (parsed as { api_key: string }).api_key.trim()
        : ''
      if (origin.length === 0 || apiKey.length === 0) return undefined
      this.cache = { origin, apiKey }
      return this.cache
    } catch (error) {
      if (errCode(error) === 'ENOENT') return undefined
      throw error
    }
  }

  async write(token: StoredToken): Promise<void> {
    this.cache = token
    await mkdir(dirname(this.path), { recursive: true })
    await writeFile(this.path, `${JSON.stringify({ origin: token.origin, api_key: token.apiKey }, null, 2)}\n`)
  }

  async clear(): Promise<void> {
    this.cache = undefined
    try {
      await writeFile(this.path, '{}\n')
    } catch (error) {
      if (errCode(error) !== 'ENOENT') throw error
    }
  }
}
