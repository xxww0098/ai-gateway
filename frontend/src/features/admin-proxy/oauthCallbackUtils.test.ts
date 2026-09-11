import { describe, expect, it } from 'vitest'
import { parseOAuthCallbackInput } from './oauthCallbackUtils'

const SESSION = 'sess-1'

describe('parseOAuthCallbackInput — 网关回调（gemini/claude/codex/kiro）', () => {
  it('从完整回调 URL 读出 code 与 state', () => {
    const parsed = parseOAuthCallbackInput(
      'https://gw.example/api/panel/admin/sdk-management/oauth-callback/codex?code=ac-1&state=st-1',
    )
    expect(parsed.code).toBe('ac-1')
    expect(parsed.state).toBe('st-1')
    expect(parsed.error).toBeNull()
  })

  it('URL 里没有 state 时回落到会话 state', () => {
    const parsed = parseOAuthCallbackInput('https://gw.example/cb?code=ac-1', {
      sessionState: SESSION,
    })
    expect(parsed.state).toBe(SESSION)
  })

  it('把 error / error_description 提出来，而不是当成成功', () => {
    const denied = parseOAuthCallbackInput('https://gw.example/cb?error=access_denied&state=st-1')
    expect(denied.error).toBe('access_denied')
    expect(denied.code).toBeNull()

    const described = parseOAuthCallbackInput(
      'https://gw.example/cb?error_description=user%20cancelled&state=st-1',
    )
    expect(described.error).toBe('user cancelled')
  })

  it('接受只粘贴出来的 query 串', () => {
    for (const raw of ['?code=ac-1&state=st-1', 'code=ac-1&state=st-1']) {
      const parsed = parseOAuthCallbackInput(raw)
      expect(parsed.code).toBe('ac-1')
      expect(parsed.state).toBe('st-1')
    }
  })

  it('Claude 的 `code#state` 只在第一个 # 上切，剩下的全归 state', () => {
    // 后端 `split_claude_code` 用的是 `split_once('#')`；两边切法必须一致，
    // 否则回填的 state 查不到会话。
    const parsed = parseOAuthCallbackInput('ac-1#st-1#extra')
    expect(parsed.code).toBe('ac-1')
    expect(parsed.state).toBe('st-1#extra')
  })

  it('`code#` 后面为空时用会话 state 补上', () => {
    const parsed = parseOAuthCallbackInput('ac-1#', { sessionState: SESSION })
    expect(parsed.code).toBe('ac-1')
    expect(parsed.state).toBe(SESSION)
  })

  it('空输入与无法识别的内容都不返回 code', () => {
    for (const raw of ['', '   ', '随便贴的一段话']) {
      const parsed = parseOAuthCallbackInput(raw)
      expect(parsed.code).toBeNull()
      expect(parsed.state).toBeNull()
    }
  })
})

