/**
 * AGW-Oauth: OAuth into AI-GateWay from DeepSeek Harness.
 * Function plugin — export name, inject, Config, apply. No export default.
 */

import type { Context } from '@deepseek-ai/cordis'
import { AgwAdapter, PROVIDER } from './adapter.js'
import { parseModelsPayload, type GatewayModel } from './catalog.js'
import { Config, resolveOrigin } from './config.js'
import { currentWatch, resetLoginWatch, startLogin, usageText, type CommandResult } from './command.js'
import { importLocalAndMaybeUpload } from './local-import.js'
import { handleHttp } from './http.js'
import { TokenStore } from './store.js'

export const name = 'agw-oauth'
export const inject = ['llm']
export { Config }
export type { Config as ConfigType } from './config.js'
export { PROVIDER, parseModelsPayload, parseGatewayModel, toResolvedModel } from './catalog.js'
export { startDevice, pollDevice } from './oauth.js'
export { AgwAdapter } from './adapter.js'
export { handleHttp } from './http.js'
export { discoverLocalOauth, parseCliJson } from './local-import.js'

export function apply(ctx: Context, config: { origin: string }): void {
  ctx.logger.info('[agw-oauth] plugin loaded!')
  const store = new TokenStore()
  let models: GatewayModel[] = []
  let registration: { replace(providers: string[]): void } | undefined
  let savedOrigin: string | undefined

  const auth = () => {
    // filled after first read; sync snapshot for the adapter
    return snapshot
  }
  let snapshot: { origin: string, apiKey: string } | undefined

  const adapter = new AgwAdapter(auth, () => models)

  const ensureAdapter = (): void => {
    if (snapshot === undefined) {
      registration?.replace([])
      return
    }
    if (registration === undefined) {
      registration = ctx.llm.registerAdapter([PROVIDER], adapter)
    } else {
      registration.replace([PROVIDER])
    }
  }

  const refreshModels = async (): Promise<void> => {
    if (snapshot === undefined) {
      models = []
      return
    }
    const response = await fetch(`${snapshot.origin}/v1/models`, {
      headers: { authorization: `Bearer ${snapshot.apiKey}` },
    })
    if (!response.ok) {
      throw new Error(`GET /v1/models failed: HTTP ${response.status}`)
    }
    models = parseModelsPayload(await response.json())
    ensureAdapter()
  }

  const persist = async (apiKey: string, origin: string): Promise<void> => {
    snapshot = { apiKey, origin }
    savedOrigin = origin
    await store.write(snapshot)
    await refreshModels()
  }

  const logout = async (): Promise<void> => {
    resetLoginWatch()
    snapshot = undefined
    savedOrigin = undefined
    models = []
    await store.clear()
    ensureAdapter()
  }

  const saveOrigin = async (origin: string): Promise<void> => {
    await store.writeOrigin(origin)
    savedOrigin = origin
    if (snapshot !== undefined) snapshot = { ...snapshot, origin }
  }

  const loginOrigin = (): string => resolveOrigin(config, snapshot?.origin ?? savedOrigin)

  void (async () => {
    savedOrigin = await store.peekOrigin()
    const token = await store.read()
    if (token === undefined) {
      ctx.logger.info('[agw-oauth] not logged in; run /agw login or Settings → AGW Oauth')
      return
    }
    snapshot = token
    savedOrigin = token.origin
    try {
      await refreshModels()
      ctx.logger.info(`[agw-oauth] ready origin=${token.origin} models=${models.length}`)
    } catch (error) {
      ctx.logger.warn(`[agw-oauth] listed no models: ${error instanceof Error ? error.message : String(error)}`)
      ensureAdapter()
    }
  })()

  const commands = ctx.get('commands') as {
    register(definition: {
      name: string
      description: string
      input?: { hint: string }
      handler: (invocation: { rawInput: string, signal?: AbortSignal }) => Promise<CommandResult>
    }): void
  } | undefined
  if (commands !== undefined) {
    commands.register({
      name: 'agw',
      description: 'AI-GateWay OAuth: status, import, login, logout',
      input: { hint: '[status|import|login|logout]' },
      handler: async (invocation) => {
        const action = (invocation.rawInput.trim().split(/\s+/)[0] ?? 'status').toLowerCase()
        if (action === 'help' || action === '-h' || action === '--help') {
          return { kind: 'success', text: usageText() }
        }
        if (action === 'logout') {
          await logout()
          return { kind: 'success', text: 'Logged out of AI-GateWay.' }
        }
        if (action === 'import') {
          const report = await importLocalAndMaybeUpload({
            origin: snapshot?.origin ?? savedOrigin,
            apiKey: snapshot?.apiKey,
          })
          const lines = report.found.length === 0
            ? ['No local CLI OAuth files found under ~/.codex, ~/.claude, ~/.grok, or ~/.kiro.']
            : report.found.map(row => `${row.provider}: ${row.source} (access=${row.hasAccessToken} refresh=${row.hasRefreshToken})`)
          if (report.uploaded !== undefined) {
            lines.push(`Upload HTTP ${report.uploaded.status}`)
          } else if (report.found.length > 0 && snapshot === undefined) {
            lines.push('Not logged into AI-GateWay; files were only listed. Run /agw login as an admin to upload, or POST /auth-files/import-local on the gateway host.')
          }
          if (report.error !== undefined) lines.push(report.error)
          return { kind: report.error === undefined ? 'success' : 'error', text: lines.join('\n') }
        }
        if (action === 'login') {
          return startLogin(loginOrigin(), persist, invocation.signal)
        }
        const watch = currentWatch()
        const login = snapshot === undefined ? 'not logged in' : `ok (${snapshot.origin})`
        const extra = watch === undefined
          ? ''
          : watch.status === 'waiting'
            ? ' — browser login in progress'
            : watch.status === 'error'
              ? ` — last login error: ${watch.detail ?? 'failed'}`
              : ' — last login finished'
        return {
          kind: 'success',
          text: [
            `AI-GateWay: ${login}${extra}`,
            snapshot === undefined ? '' : `Models: ${models.map(m => m.id).join(', ') || '(none listed)'}`,
            '',
            usageText(),
          ].filter(Boolean).join('\n'),
        }
      },
    })
  }

  ctx.inject(['webServer'], (httpCtx) => {
    const server = (httpCtx as unknown as {
      webServer: { register(spec: { kind: string, path: string, handler: (req: unknown, res: unknown) => void }): () => void }
    }).webServer
    httpCtx.effect(() => server.register({
      kind: 'prefix',
      path: '/agw-oauth',
      handler: (req, res) => {
        void handleHttp(req as { url?: string, method?: string }, res as {
          setHeader(name: string, value: string): void
          end(body: string): void
          statusCode: number
        }, {
          config,
          persist,
          token: () => snapshot,
          savedOrigin: () => savedOrigin,
          saveOrigin,
          logout,
        })
      },
    }), 'agw-oauth: http api')
  })
}
