import { describe, expect, it } from 'vitest'
import { BAILIAN_PRESET, OPENAI_COMPAT_PRESETS } from '../openaiCompatPresets'
import { LOBE_BRAND_ICON_ALIASES, LOBE_BRAND_ICONS } from './lobehubBrandIcons'

describe('LOBE_BRAND_ICONS', () => {
  it('每个内置平台预设的图标 key 都能查到组件', () => {
    for (const preset of OPENAI_COMPAT_PRESETS) {
      const resolved = LOBE_BRAND_ICON_ALIASES[preset.iconProvider] || preset.iconProvider
      expect(LOBE_BRAND_ICONS[resolved], `missing icon: ${preset.iconProvider}`).toBeDefined()
    }
  })

  it('bailian 与 dashscope 指向同一个组件', () => {
    expect(LOBE_BRAND_ICONS.bailian).toBeDefined()
    expect(LOBE_BRAND_ICONS.dashscope).toBe(LOBE_BRAND_ICONS.bailian)
    expect(LOBE_BRAND_ICONS[BAILIAN_PRESET.iconProvider]).toBe(LOBE_BRAND_ICONS.bailian)
  })

  it('qwen 仍是模型家族自己的图标，没被并进百炼', () => {
    expect(LOBE_BRAND_ICONS.qwen).toBeDefined()
    expect(LOBE_BRAND_ICONS.qwen).not.toBe(LOBE_BRAND_ICONS.bailian)
  })

  it('国内各平台的图标别名正确映射', () => {
    expect(LOBE_BRAND_ICONS.siliconflow).toBe(LOBE_BRAND_ICONS.siliconcloud)
    expect(LOBE_BRAND_ICONS.lingyi).toBe(LOBE_BRAND_ICONS.yi)
    expect(LOBE_BRAND_ICONS.volcengine).toBe(LOBE_BRAND_ICONS.doubao)
    expect(LOBE_BRAND_ICONS.step).toBe(LOBE_BRAND_ICONS.stepfun)
    expect(LOBE_BRAND_ICONS.xiaomi).toBe(LOBE_BRAND_ICONS.mimo)
  })
})
