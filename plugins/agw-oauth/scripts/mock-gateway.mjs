#!/usr/bin/env node
/**
 * Stand-in AI-GateWay for plugin smoke tests.
 * Device-code start/poll/approve plus /v1/models and a tiny SSE chat.
 */
import { createServer } from 'node:http'

const port = Number(process.env.AGW_MOCK_PORT ?? 8787)
const origin = process.env.AGW_PUBLIC_ORIGIN ?? `http://127.0.0.1:${port}`
const sessions = new Map()

function send(res, status, body) {
  const data = JSON.stringify(body)
  res.writeHead(status, {
    'content-type': 'application/json',
    'access-control-allow-origin': '*',
  })
  res.end(data)
}

function ok(res, data) {
  send(res, 200, { code: 0, message: 'ok', data })
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = []
    req.on('data', (c) => chunks.push(c))
    req.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8')
      if (raw.length === 0) {
        resolve({})
        return
      }
      try {
        resolve(JSON.parse(raw))
      } catch (error) {
        reject(error)
      }
    })
    req.on('error', reject)
  })
}

function randomCode() {
  return Math.random().toString(36).slice(2, 6).toUpperCase() + '-' + Math.random().toString(36).slice(2, 6).toUpperCase()
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url ?? '/', origin)
  if (req.method === 'OPTIONS') {
    res.writeHead(204, {
      'access-control-allow-origin': '*',
      'access-control-allow-headers': 'authorization,content-type',
      'access-control-allow-methods': 'GET,POST,OPTIONS',
    })
    res.end()
    return
  }
  try {
    if (req.method === 'POST' && url.pathname === '/api/panel/oauth/dsh/device/code') {
      const deviceCode = `dev-${Date.now()}`
      const userCode = randomCode()
      sessions.set(deviceCode, { userCode, status: 'pending' })
      ok(res, {
        device_code: deviceCode,
        user_code: userCode,
        verification_uri: `${origin}/oauth/dsh`,
        verification_uri_complete: `${origin}/oauth/dsh?user_code=${userCode}`,
        expires_in: 600,
        interval: 1,
      })
      return
    }
    if (req.method === 'POST' && url.pathname === '/api/panel/oauth/dsh/device/token') {
      const body = await readBody(req)
      const session = sessions.get(String(body.device_code ?? ''))
      if (session === undefined) {
        send(res, 404, { code: 404, message: 'unknown device_code' })
        return
      }
      if (session.status === 'approved') {
        ok(res, { status: 'approved', api_key: 'agw-test-key', origin })
        return
      }
      if (session.status === 'denied') {
        ok(res, { status: 'denied' })
        return
      }
      ok(res, { status: 'pending' })
      return
    }
    if (req.method === 'POST' && url.pathname === '/api/panel/oauth/dsh/device/approve') {
      const body = await readBody(req)
      const userCode = String(body.user_code ?? url.searchParams.get('user_code') ?? '')
      for (const session of sessions.values()) {
        if (session.userCode === userCode) {
          session.status = 'approved'
          ok(res, { approved: true })
          return
        }
      }
      send(res, 404, { code: 404, message: 'unknown user_code' })
      return
    }
    if (req.method === 'GET' && url.pathname === '/oauth/dsh') {
      const userCode = url.searchParams.get('user_code') ?? ''
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
      res.end(`<!doctype html><title>AGW mock consent</title><p>user_code=${userCode}</p><p>POST /api/panel/oauth/dsh/device/approve to approve.</p>`)
      return
    }
    if (req.method === 'GET' && url.pathname === '/v1/models') {
      send(res, 200, {
        object: 'list',
        data: [
          {
            id: 'vision-think',
            object: 'model',
            created: 0,
            owned_by: 'ai-gateway',
            context_length: 128000,
            max_output_tokens: 16384,
            input_modalities: ['text', 'image'],
            reasoning: { efforts: [{ id: 'low', name: 'Low' }, { id: 'high', name: 'High' }], default_effort: 'high' },
          },
          {
            id: 'text-only',
            object: 'model',
            created: 0,
            owned_by: 'ai-gateway',
            context_length: 32000,
            max_output_tokens: 4096,
            input_modalities: ['text'],
          },
        ],
      })
      return
    }
    if (req.method === 'POST' && url.pathname === '/v1/chat/completions') {
      res.writeHead(200, { 'content-type': 'text/event-stream' })
      res.write('data: {"choices":[{"delta":{"content":"hello from mock"}}]}\n\n')
      res.write('data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4}}\n\n')
      res.write('data: [DONE]\n\n')
      res.end()
      return
    }
    send(res, 404, { code: 404, message: 'not found' })
  } catch (error) {
    send(res, 500, { code: 500, message: String(error) })
  }
})

server.listen(port, '127.0.0.1', () => {
  process.stdout.write(`mock-gateway listening on ${origin}\n`)
})
