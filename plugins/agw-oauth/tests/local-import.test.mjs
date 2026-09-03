import assert from 'node:assert/strict'
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { discoverLocalOauth, parseCliJson, redact, toUploadJson } from '../lib/local-import.js'

test('parseCliJson reads Claude Code camelCase without a provider field', () => {
  const found = parseCliJson({
    claudeAiOauth: { accessToken: 'at', refreshToken: 'rt', expiresAt: 1800000000000 },
  }, '/tmp/.claude/.credentials.json')
  assert.equal(found.length, 1)
  assert.equal(found[0].provider, 'claude')
  assert.equal(found[0].accessToken, 'at')
  assert.equal(found[0].refreshToken, 'rt')
})

test('parseCliJson reads Codex CLI tokens object', () => {
  const found = parseCliJson({
    tokens: { access_token: 'at', refresh_token: 'rt', id_token: 'id' },
    last_refresh: 'now',
  }, '/tmp/.codex/auth.json')
  assert.equal(found.length, 1)
  assert.equal(found[0].provider, 'codex')
  assert.equal(found[0].idToken, 'id')
})

test('redact drops token material from the report', () => {
  const report = redact([{
    provider: 'codex',
    source: '/tmp/.codex/auth.json',
    accessToken: 'secret-access',
    refreshToken: 'secret-refresh',
    idToken: 'secret-id',
    expiresAt: '',
    email: '',
  }])
  const text = JSON.stringify(report)
  assert.equal(report[0].hasAccessToken, true)
  assert.doesNotMatch(text, /secret-/)
})

test('toUploadJson keeps provider so the gateway inventory can file the row', () => {
  const body = toUploadJson({
    provider: 'claude',
    source: '/tmp/.claude/.credentials.json',
    accessToken: 'at',
    refreshToken: 'rt',
    idToken: '',
    expiresAt: '',
    email: '',
  })
  assert.equal(body.provider, 'claude')
  assert.equal(body.access_token, 'at')
})

test('discoverLocalOauth finds both Codex and Claude files under a fake home', async () => {
  const home = await mkdtemp(join(tmpdir(), 'agw-oauth-'))
  await mkdir(join(home, '.codex'), { recursive: true })
  await mkdir(join(home, '.claude'), { recursive: true })
  await writeFile(join(home, '.codex', 'auth.json'), JSON.stringify({
    tokens: { access_token: 'c-at', refresh_token: 'c-rt', id_token: 'c-id' },
    last_refresh: 'now',
  }))
  await writeFile(join(home, '.claude', '.credentials.json'), JSON.stringify({
    claudeAiOauth: { accessToken: 'l-at', refreshToken: 'l-rt' },
  }))
  const found = await discoverLocalOauth(home)
  const providers = found.map(row => row.provider).sort()
  assert.deepEqual(providers, ['claude', 'codex'])
})
