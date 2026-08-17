import assert from 'node:assert/strict'
import { mkdtemp, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { TokenStore } from '../lib/store.js'

async function tempStore() {
  const dir = await mkdtemp(join(tmpdir(), 'agw-oauth-'))
  return new TokenStore(join(dir, 'agw-oauth.json'))
}

test('write origin-only: peekOrigin works, read() is still logged-out', async () => {
  const store = await tempStore()
  await store.write({ origin: 'https://gw.example.com/' })
  assert.equal(await store.read(), undefined)
  assert.equal(await store.peekOrigin(), 'https://gw.example.com')
  const raw = JSON.parse(await readFile(store.path, 'utf8'))
  assert.equal(raw.origin, 'https://gw.example.com')
  assert.equal(raw.api_key, undefined)
})

test('read() is logged in only when both origin and api_key exist', async () => {
  const store = await tempStore()
  await store.write({ origin: 'https://gw.example.com', apiKey: 'agw-key' })
  assert.deepEqual(await store.read(), { origin: 'https://gw.example.com', apiKey: 'agw-key' })
})

test('writeOrigin keeps an existing api_key', async () => {
  const store = await tempStore()
  await store.write({ origin: 'https://old.example', apiKey: 'agw-key' })
  await store.writeOrigin('https://new.example')
  assert.deepEqual(await store.read(), { origin: 'https://new.example', apiKey: 'agw-key' })
})
