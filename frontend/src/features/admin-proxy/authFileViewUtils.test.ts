import { describe, expect, it } from 'vitest'
import { getBatchStatusTargets, resolveAuthFileIdentity } from './authFileViewUtils'
import type { AuthFileItem } from './types'

describe('resolveAuthFileIdentity', () => {
  it('does not repeat the email when it is already the primary identity', () => {
    const identity = resolveAuthFileIdentity({
      name: 'tmp77svn879k9v@639366.xyz',
      email: 'tmp77svn879k9v@639366.xyz',
    })

    expect(identity.primary).toBe('tmp77svn879k9v@639366.xyz')
    expect(identity.email).toBeUndefined()
  })

  it('keeps email as secondary context when label is different', () => {
    const identity = resolveAuthFileIdentity({
      name: 'codex-session.json',
      label: 'Codex 主账号',
      email: 'codex@example.com',
    })

    expect(identity.primary).toBe('Codex 主账号')
    expect(identity.email).toBe('codex@example.com')
  })

  it('falls back to name when no label or email is available', () => {
    const identity = resolveAuthFileIdentity({ name: 'anthropic-auth.json' })

    expect(identity.primary).toBe('anthropic-auth.json')
    expect(identity.email).toBeUndefined()
  })
})

describe('getBatchStatusTargets', () => {
  const files: AuthFileItem[] = [
    { name: 'enabled-a.json', disabled: false },
    { name: 'disabled-b.json', disabled: true },
    { name: 'enabled-c.json' },
  ]

  it('only pauses selected enabled files', () => {
    const targets = getBatchStatusTargets(files, new Set(['enabled-a.json', 'disabled-b.json']))

    expect(targets.pauseTargets.map(file => file.name)).toEqual(['enabled-a.json'])
  })

  it('only resumes selected disabled files', () => {
    const targets = getBatchStatusTargets(files, new Set(['enabled-a.json', 'disabled-b.json', 'enabled-c.json']))

    expect(targets.resumeTargets.map(file => file.name)).toEqual(['disabled-b.json'])
  })

  it('ignores unselected files when calculating batch actions', () => {
    const targets = getBatchStatusTargets(files, new Set(['enabled-c.json']))

    expect(targets.selected.map(file => file.name)).toEqual(['enabled-c.json'])
    expect(targets.pauseTargets.map(file => file.name)).toEqual(['enabled-c.json'])
    expect(targets.resumeTargets).toEqual([])
  })
})
