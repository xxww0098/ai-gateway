import assert from 'node:assert/strict'
import { test } from 'node:test'
import { resetLoginWatch, startLogin } from '../lib/command.js'

test('startLogin returns the user_code and URL without waiting for approve', async () => {
  resetLoginWatch()
  const original = globalThis.fetch
  globalThis.fetch = async () => new Response(JSON.stringify({
    code: 0,
    message: 'ok',
    data: {
      device_code: 'dev-9',
      user_code: 'WXYZ-2345',
      verification_uri: 'https://gw.example/oauth/dsh',
      verification_uri_complete: 'https://gw.example/oauth/dsh?user_code=WXYZ-2345',
      expires_in: 600,
      interval: 30,
    },
  }), { status: 200, headers: { 'content-type': 'application/json' } })
  try {
    const started = Date.now()
    const result = await startLogin('https://gw.example', async () => undefined)
    assert.ok(Date.now() - started < 1000, 'login must return as soon as the URL exists')
    assert.equal(result.kind, 'success')
    assert.equal(result.userCode, 'WXYZ-2345')
    assert.equal(result.openUrl, 'https://gw.example/oauth/dsh?user_code=WXYZ-2345')
    assert.match(result.text, /WXYZ-2345/)
  } finally {
    globalThis.fetch = original
    resetLoginWatch()
  }
})
