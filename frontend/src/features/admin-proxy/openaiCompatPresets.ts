// 内置 OpenAI 兼容平台预设：运营在面板里只需填 API Key，
// 名称 / Base URL / 模型前缀 / 模型列表 URL 由预设写入。
//
// 这里是唯一的事实来源 —— 添加表单（ProviderTab）、渠道表格里的品牌图标、
// 探测弹窗的 provider 归类，以及测试都从这里读，不要在别处重抄这些常量。

import type { ProviderStructuredForm } from './providerConfig'

export interface OpenAiCompatPreset {
  /** 预设标识，同时也是模型目录里的 provider key */
  key: string
  /** 面板上的显示名 */
  label: string
  /** 品牌图标 key（见 lobehubBrandIcons.ts） */
  iconProvider: string
  /** 写进 `openai-compatibility` 的渠道名 */
  name: string
  baseUrl: string
  /** 模型前缀，带斜杠：`bailian/qwen3-max` */
  prefix: string
  modelsUrl: string
  /** 认回一个已存在渠道时按名字匹配的别名，全小写 */
  aliases: string[]
}

/** 阿里云百炼（DashScope）的公网 OpenAI 兼容端点，不是专属实例。 */
export const BAILIAN_PRESET: OpenAiCompatPreset = {
  key: 'bailian',
  label: '百炼',
  iconProvider: 'bailian',
  name: 'bailian',
  baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
  prefix: 'bailian/',
  modelsUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1/models',
  aliases: ['bailian', 'dashscope', '百炼'],
}

export const OPENAI_COMPAT_PRESETS: OpenAiCompatPreset[] = [BAILIAN_PRESET]

/** 预设写进添加表单的字段：除 API Key 之外的一切。 */
export function openAiCompatPresetForm(preset: OpenAiCompatPreset): ProviderStructuredForm {
  return {
    name: preset.name,
    baseUrl: preset.baseUrl,
    prefix: preset.prefix,
    modelsUrl: preset.modelsUrl,
  }
}

/**
 * 认出一个渠道是哪个内置预设：渠道名撞上别名，或 Base URL 落在预设的
 * 兼容端点上（`https://<host>/<第一段>/...`，所以 `/v1` 有没有、末尾斜杠
 * 有没有都认）。都不像就返回 `undefined` —— 自填的通用渠道走原路。
 */
export function matchOpenAiCompatPreset(channel: { name?: string; baseUrl?: string }): OpenAiCompatPreset | undefined {
  const name = channel.name?.trim().toLowerCase() || ''
  const baseUrl = channel.baseUrl?.trim() || ''
  return OPENAI_COMPAT_PRESETS.find(
    preset =>
      (!!name && preset.aliases.some(alias => name.includes(alias))) ||
      (!!baseUrl && baseUrlMatchesPreset(preset, baseUrl)),
  )
}

function baseUrlMatchesPreset(preset: OpenAiCompatPreset, baseUrl: string): boolean {
  const target = endpointRoot(preset.baseUrl)
  const candidate = endpointRoot(baseUrl)
  if (!target || !candidate) return false
  return candidate.host === target.host && candidate.rootPath === target.rootPath
}

/** `https://dashscope.aliyuncs.com/compatible-mode/v1` → host + `/compatible-mode` */
function endpointRoot(url: string): { host: string; rootPath: string } | undefined {
  try {
    const parsed = new URL(url)
    const firstSegment = parsed.pathname.split('/').filter(Boolean)[0] || ''
    return { host: parsed.host.toLowerCase(), rootPath: firstSegment ? `/${firstSegment}` : '' }
  } catch {
    return undefined
  }
}
