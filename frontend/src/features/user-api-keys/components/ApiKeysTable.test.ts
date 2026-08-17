import { describe, it, expect } from 'vitest'
import { maskApiKeyDisplay } from './ApiKeysTable'

describe('maskApiKeyDisplay', () => {
  it('masks agw- and sk-agw- prefixes', () => {
    expect(maskApiKeyDisplay('agw-0123456789abcdef')).toBe('agw-****')
    expect(maskApiKeyDisplay('sk-agw-0123456789abcdef')).toBe('sk-agw-****')
  })

  it('does not keep a cpa- branch', () => {
    expect(maskApiKeyDisplay('cpa-oldkey')).toBe('****')
    expect(maskApiKeyDisplay('sk-cpa-oldkey')).toBe('sk-****')
  })
})
