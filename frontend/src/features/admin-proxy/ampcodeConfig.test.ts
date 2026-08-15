import { describe, expect, it } from 'vitest'
import {
  buildAmpModelMappingsPutPayload,
  buildAmpUpstreamAPIKeysDeletePayload,
  buildAmpUpstreamAPIKeysPutPayload,
  extractAmpcodeConfig,
  normalizeAmpModelMappings,
  parseAmpUpstreamAPIKeyForm,
  validateAmpModelMappings,
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
})
