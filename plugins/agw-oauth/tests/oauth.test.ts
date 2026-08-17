import assert from 'node:assert/strict'
import { test } from 'node:test'
import { pollDevice, startDevice } from '../lib/oauth.js'

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify({ code: 0, message: 'ok', data }), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

test('startDevice returns user_code and verification URL from the gateway', async () => {
  const original = globalThis.fetch
  globalThis.fetch = async (input) => {
    const url = String(input)
    assert.match(url, /\/api\/panel\/oauth\/dsh\/device\/code$/)
    return jsonResponse({
      device_code: 'dev-1',
      user_code: 'ABCD-EFGH',
      verification_uri: 'https://gw.example/oauth/dsh',
      verification_uri_complete: 'https://gw.example/oauth/dsh?user_code=ABCD-EFGH',
      expires_in: 600,
      interval: 2,
    })
  }
  try {
    const started = await startDevice('https://gw.example/')
    assert.equal(started.userCode, 'ABCD-EFGH')
    assert.equal(started.verificationUri, 'https://gw.example/oauth/dsh')
    assert.equal(started.deviceCode, 'dev-1')
  } finally {
    globalThis.fetch = original
  }
})

test('pollDevice maps pending then approved with the agw key and origin', async () => {
  const original = globalThis.fetch
  const bodies: unknown[] = [
    jsonResponse({ status: 'pending' }),
    jsonResponse({ status: 'approved', api_key: 'agw-test-key', origin: 'https://gw.example' }),
  ]
  globalThis.fetch = async () => bodies.shift() as Response
  try {
    const pending = await pollDevice('https://gw.example', 'dev-1')
    assert.equal(pending.status, 'pending')
    const approved = await pollDevice('https://gw.example', 'dev-1')
    assert.equal(approved.status, 'approved')
    if (approved.status === 'approved') {
      assert.equal(approved.apiKey, 'agw-test-key')
      assert.equal(approved.origin, 'https://gw.example')
    }
  } finally {
    globalThis.fetch = original
  }
})
