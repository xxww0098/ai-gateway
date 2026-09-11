import { describe, expect, it } from 'vitest'
import {
  buildAmpModelMappingsPutPayload,
  buildAmpUpstreamAPIKeysDeletePayload,
  buildAmpUpstreamAPIKeysPutPayload,
  extractAmpcodeConfig,
  isRegexValid,
  maskApiKey,
  normalizeAmpModelMappings,
  parseAmpUpstreamAPIKeyForm,
  simulateModelMapping,
  validateAmpModelMappings,
  validateUpstreamUrl,
} from './ampcodeConfig'

describe('ampcodeConfig', () => {
  it('extracts the nested SDK ampcode response shape', () => {
    expect(extractAmpcodeConfig({
      ampcode: {
        'upstream-url': 'https://ampcode.com',
        'model-mappings': [{ from: 'amp-model', to: 'local-model' }],
      },
    })).toEqual({
      'upstream-url': 'https://ampcode.com',
      'model-mappings': [{ from: 'amp-model', to: 'local-model' }],
    })
  })

  it('builds SDK upstream-api-keys value payloads', () => {
    const entry = parseAmpUpstreamAPIKeyForm({
      upstreamApiKey: ' upstream-key ',
      apiKeysText: 'client-a\nclient-b, client-a',
    })

    expect(buildAmpUpstreamAPIKeysPutPayload([entry])).toEqual({
      value: [{
        'upstream-api-key': 'upstream-key',
        'api-keys': ['client-a', 'client-b'],
      }],
    })
  })

  it('builds SDK delete payloads with value array', () => {
    expect(buildAmpUpstreamAPIKeysDeletePayload([' upstream-key ', ''])).toEqual({
      value: ['upstream-key'],
    })
  })

  it('builds SDK model-mappings value arrays', () => {
    expect(buildAmpModelMappingsPutPayload([
      { from: ' claude-opus-4-5-20251101 ', to: ' gemini-claude-sonnet-4-5 ' },
      { from: '^gpt-.*', to: 'gpt-4o', regex: true },
      { from: '', to: '' },
    ])).toEqual({
      value: [
        { from: 'claude-opus-4-5-20251101', to: 'gemini-claude-sonnet-4-5' },
        { from: '^gpt-.*', to: 'gpt-4o', regex: true },
      ],
    })
  })

  it('validates empty and duplicate model mappings before saving', () => {
    expect(validateAmpModelMappings([
      { from: '', to: 'target' },
      { from: 'source', to: '' },
      { from: 'SOURCE', to: 'other' },
    ])).toEqual([
      '第 1 行缺少 from 模型',
      '第 2 行缺少 to 模型',
      '第 3 行 from 模型重复: SOURCE',
    ])

    expect(() => buildAmpModelMappingsPutPayload([
      { from: 'source', to: 'target' },
      { from: 'SOURCE', to: 'other' },
    ])).toThrow('from 模型重复')
  })

  it('normalizes malformed model mapping values from management responses', () => {
    expect(normalizeAmpModelMappings([
      { from: 1, to: 'target' },
      'invalid-json-shape',
      { from: 'source', to: ' target ', regex: 'true' },
    ])).toEqual([
      { from: '', to: 'target', regex: false },
      { from: '', to: '', regex: false },
      { from: 'source', to: 'target', regex: false },
    ])
  })

  describe('isRegexValid', () => {
    it('returns valid true for valid regex patterns', () => {
      expect(isRegexValid('^gpt-.*$')).toEqual({ valid: true })
      expect(isRegexValid('claude-[0-9]+')).toEqual({ valid: true })
      expect(isRegexValid('')).toEqual({ valid: true })
    })

    it('returns valid false with error for invalid regex patterns', () => {
      const result = isRegexValid('[')
      expect(result.valid).toBe(false)
      expect(result.error).toBeDefined()

      const result2 = isRegexValid('*invalid')
      expect(result2.valid).toBe(false)
      expect(result2.error).toBeDefined()
    })
  })

  describe('simulateModelMapping', () => {
    it('returns exact match when regex is false (case-insensitive and trimmed)', () => {
      const mappings = [
        { from: 'claude-3-5-sonnet', to: 'anthropic-sonnet', regex: false },
        { from: 'gpt-4o', to: 'openai-gpt-4o', regex: false },
      ]

      const result = simulateModelMapping('  CLAUDE-3-5-SONNET  ', mappings)
      expect(result).toEqual({
        matched: true,
        targetModel: 'anthropic-sonnet',
        ruleIndex: 0,
        matchedRule: mappings[0],
      })
    })

    it('returns original model when no mapping matches', () => {
      const mappings = [
        { from: 'gpt-4o', to: 'openai-gpt-4o', regex: false },
      ]

      const result = simulateModelMapping('claude-3-opus', mappings)
      expect(result).toEqual({
        matched: false,
        targetModel: 'claude-3-opus',
        ruleIndex: -1,
      })
    })

    it('replaces model with regex capture group when regex is true', () => {
      const mappings = [
        { from: '^gpt-(.*)$', to: 'amp-$1', regex: true },
      ]

      const result = simulateModelMapping('gpt-4o-mini', mappings)
      expect(result).toEqual({
        matched: true,
        targetModel: 'amp-4o-mini',
        ruleIndex: 0,
        matchedRule: mappings[0],
      })
    })

    it('replaces model without capture group when regex matches', () => {
      const mappings = [
        { from: '^claude-.*', to: 'gemini-claude', regex: true },
      ]

      const result = simulateModelMapping('claude-3-5-sonnet', mappings)
      expect(result).toEqual({
        matched: true,
        targetModel: 'gemini-claude',
        ruleIndex: 0,
        matchedRule: mappings[0],
      })
    })

    it('safely skips invalid regex and continues to next rule', () => {
      const mappings = [
        { from: '[invalid-regex', to: 'never-reached', regex: true },
        { from: 'gpt-4o', to: 'target-gpt-4o', regex: false },
      ]

      const result = simulateModelMapping('gpt-4o', mappings)
      expect(result).toEqual({
        matched: true,
        targetModel: 'target-gpt-4o',
        ruleIndex: 1,
        matchedRule: mappings[1],
      })
    })

    it('returns the first matching rule and correct index when multiple rules exist', () => {
      const mappings = [
        { from: 'other-model', to: 'other-target', regex: false },
        { from: '^gpt-(.*)$', to: 'first-$1', regex: true },
        { from: '^gpt-4o$', to: 'second-target', regex: true },
      ]

      const result = simulateModelMapping('gpt-4o', mappings)
      expect(result.ruleIndex).toBe(1)
      expect(result.targetModel).toBe('first-4o')
    })
  })

  describe('maskApiKey', () => {
    it('returns empty string for empty input', () => {
      expect(maskApiKey('')).toBe('')
    })

    it('masks completely when length <= 8', () => {
      expect(maskApiKey('1234')).toBe('••••••••')
      expect(maskApiKey('12345678')).toBe('••••••••')
    })

    it('keeps first 6 and last 4 chars, masking middle with 8 bullets when length > 8', () => {
      expect(maskApiKey('sgamp_1234567890abcdef')).toBe('sgamp_••••••••cdef')
      expect(maskApiKey('sk-ant-api03-abcdefg1234')).toBe('sk-ant••••••••1234')
      expect(maskApiKey('123456789')).toBe('123456••••••••6789')
    })
  })

  describe('validateUpstreamUrl', () => {
    it('validates empty URL', () => {
      expect(validateUpstreamUrl('')).toEqual({
        valid: false,
        message: '请输入上游地址',
      })
      expect(validateUpstreamUrl('   ')).toEqual({
        valid: false,
        message: '请输入上游地址',
      })
    })

    it('suggests https scheme when user provides url without scheme', () => {
      expect(validateUpstreamUrl('ampcode.com')).toEqual({
        valid: false,
        message: '上游地址必须以 http:// 或 https:// 开头',
        suggestedUrl: 'https://ampcode.com',
      })
      expect(validateUpstreamUrl('ampcode.com/v1')).toEqual({
        valid: false,
        message: '上游地址必须以 http:// 或 https:// 开头',
        suggestedUrl: 'https://ampcode.com/v1',
      })
    })

    it('returns valid true for valid http and https URLs', () => {
      expect(validateUpstreamUrl('https://ampcode.com')).toEqual({ valid: true })
      expect(validateUpstreamUrl('http://localhost:8080')).toEqual({ valid: true })
    })

    it('returns valid false for malformed URLs with http prefix', () => {
      expect(validateUpstreamUrl('http://')).toEqual({
        valid: false,
        message: '上游地址格式不正确',
      })
    })
  })
})

