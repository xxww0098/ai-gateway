import assert from 'node:assert/strict'
import { Readable } from 'node:stream'
import { test } from 'node:test'
import { handleHttp, normalizeSavedOrigin } from '../lib/http.js'
import { resetLoginWatch } from '../lib/command.js'

function mockReq(method, url, body) {
  const req = body === undefined
    ? Readable.from([])
    : Readable.from([JSON.stringify(body)])
  req.method = method
  req.url = url
  return req
}

function mockRes() {
  return {
    statusCode: 200,
    headers: {},
    body: '',
    setHeader(name, value) { this.headers[name] = value },
    end(body) { this.body = body },
  }
}

function deps(state) {
  return {
    config: { origin: '' },
    persist: async () => undefined,
    token: () => state.token,
    savedOrigin: () => state.origin,
    saveOrigin: async (origin) => { state.origin = origin },
    logout: async () => {
      state.token = undefined
      state.origin = undefined
      resetLoginWatch()
    },
  }
}

test('normalizeSavedOrigin accepts http(s) and strips trailing slash', () => {
  assert.equal(normalizeSavedOrigin('https://gw.example.com/'), 'https://gw.example.com')
  assert.throws(() => normalizeSavedOrigin('ftp://x'), /http/)
  assert.throws(() => normalizeSavedOrigin(''), /required/)
})

test('POST /origin then GET /status returns origin while logged out', async () => {
  const state = { token: undefined, origin: undefined }
  const post = mockRes()
  await handleHttp(mockReq('POST', '/agw-oauth/origin', { origin: 'https://gw.example.com/' }), post, deps(state))
  assert.equal(post.statusCode, 200)
  assert.deepEqual(JSON.parse(post.body), { ok: true, origin: 'https://gw.example.com' })
  assert.equal(state.origin, 'https://gw.example.com')

  const get = mockRes()
  await handleHttp(mockReq('GET', '/agw-oauth/status'), get, deps(state))
  const status = JSON.parse(get.body)
  assert.equal(status.loggedIn, false)
  assert.equal(status.origin, 'https://gw.example.com')
})

test('POST /logout clears credentials like /agw logout', async () => {
  const state = { token: { origin: 'https://gw.example.com', apiKey: 'agw-key' }, origin: 'https://gw.example.com' }
  const res = mockRes()
  await handleHttp(mockReq('POST', '/agw-oauth/logout'), res, deps(state))
  assert.equal(res.statusCode, 200)
  assert.deepEqual(JSON.parse(res.body), { ok: true, loggedIn: false })
  assert.equal(state.token, undefined)
  assert.equal(state.origin, undefined)
})
