import { describe, expect, it } from 'vitest'
import {
  BAILIAN_PRESET,
  OPENAI_COMPAT_PRESETS,
  matchOpenAiCompatPreset,
  openAiCompatPresetForm,
} from './openaiCompatPresets'
import { buildProviderAddArray, normalizeProviderItems } from './providerConfig'

describe('openAiCompatPresetForm', () => {
  it('填满除 API Key 以外的一切：加上 Key 就能落库，不加就只差 Key', () => {
    const form = openAiCompatPresetForm(BAILIAN_PRESET)

    expect(() => buildProviderAddArray('openai', [], form)).toThrow('API Key 不能为空')

    const updated = buildProviderAddArray('openai', [], { ...form, apiKey: 'sk-operator' })
    expect(updated).toHaveLength(1)
    const item = normalizeProviderItems('openai', updated)[0]
    expect(item.name).toBe(BAILIAN_PRESET.name)
    expect(item.baseUrl).toBe(BAILIAN_PRESET.baseUrl)
    expect(item.prefix).toBe(BAILIAN_PRESET.prefix)
    expect(item.modelsUrl).toBe(BAILIAN_PRESET.modelsUrl)
  })

  it('模型列表 URL 就挂在 Base URL 下面，前缀带尾斜杠', () => {
    for (const preset of OPENAI_COMPAT_PRESETS) {
      expect(preset.modelsUrl.startsWith(preset.baseUrl)).toBe(true)
      expect(preset.modelsUrl.endsWith('/models')).toBe(true)
      expect(preset.prefix).toBe(`${preset.key}/`)
      expect(preset.aliases).toContain(preset.name)
    }
  })
})

describe('matchOpenAiCompatPreset', () => {
  it('按渠道名认预设，别名与中文名都算', () => {
    for (const alias of BAILIAN_PRESET.aliases) {
      expect(matchOpenAiCompatPreset({ name: alias })).toBe(BAILIAN_PRESET)
      expect(matchOpenAiCompatPreset({ name: alias.toUpperCase() })).toBe(BAILIAN_PRESET)
    }
  })

  it('按 Base URL 认预设：末尾斜杠、少写版本段都认', () => {
    const root = BAILIAN_PRESET.baseUrl.replace(/\/v1$/, '')
    for (const baseUrl of [BAILIAN_PRESET.baseUrl, `${BAILIAN_PRESET.baseUrl}/`, root]) {
      expect(matchOpenAiCompatPreset({ name: 'my-gateway', baseUrl })).toBe(BAILIAN_PRESET)
    }
  })

  it('自填的通用渠道不认成预设', () => {
    expect(matchOpenAiCompatPreset({ name: 'openrouter', baseUrl: 'https://openrouter.ai/api/v1' })).toBeUndefined()
    expect(matchOpenAiCompatPreset({})).toBeUndefined()
    expect(matchOpenAiCompatPreset({ name: '', baseUrl: 'not a url' })).toBeUndefined()
  })

  it('同一个域名下换了一套 API 就不算这个预设了', () => {
    const { host } = new URL(BAILIAN_PRESET.baseUrl)
    expect(matchOpenAiCompatPreset({ baseUrl: `https://${host}/api/v1` })).toBeUndefined()
  })
})
