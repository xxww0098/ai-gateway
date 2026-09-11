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

/** DeepSeek（深度求索）开放平台端点 */
export const DEEPSEEK_PRESET: OpenAiCompatPreset = {
  key: 'deepseek',
  label: 'DeepSeek',
  iconProvider: 'deepseek',
  name: 'deepseek',
  baseUrl: 'https://api.deepseek.com/v1',
  prefix: 'deepseek/',
  modelsUrl: 'https://api.deepseek.com/v1/models',
  aliases: ['deepseek', '深度求索'],
}

/** 智谱 AI（BigModel / 智谱清言）开放平台端点 */
export const ZHIPU_PRESET: OpenAiCompatPreset = {
  key: 'zhipu',
  label: '智谱 AI',
  iconProvider: 'zhipu',
  name: 'zhipu',
  baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
  prefix: 'zhipu/',
  modelsUrl: 'https://open.bigmodel.cn/api/paas/v4/models',
  aliases: ['zhipu', 'bigmodel', 'chatglm', '智谱'],
}

/** 月之暗面 Moonshot（Kimi）开放平台端点 */
export const MOONSHOT_PRESET: OpenAiCompatPreset = {
  key: 'moonshot',
  label: '月之暗面',
  iconProvider: 'moonshot',
  name: 'moonshot',
  baseUrl: 'https://api.moonshot.cn/v1',
  prefix: 'moonshot/',
  modelsUrl: 'https://api.moonshot.cn/v1/models',
  aliases: ['moonshot', 'kimi', '月之暗面'],
}

/** MiniMax（稀宇科技）开放平台端点 */
export const MINIMAX_PRESET: OpenAiCompatPreset = {
  key: 'minimax',
  label: 'MiniMax',
  iconProvider: 'minimax',
  name: 'minimax',
  baseUrl: 'https://api.minimaxi.com/v1',
  prefix: 'minimax/',
  modelsUrl: 'https://api.minimaxi.com/v1/models',
  aliases: ['minimax', 'minimaxi', '稀宇'],
}

/** 硅基流动（SiliconFlow）模型推理平台端点 */
export const SILICONFLOW_PRESET: OpenAiCompatPreset = {
  key: 'siliconflow',
  label: '硅基流动',
  iconProvider: 'siliconflow',
  name: 'siliconflow',
  baseUrl: 'https://api.siliconflow.cn/v1',
  prefix: 'siliconflow/',
  modelsUrl: 'https://api.siliconflow.cn/v1/models',
  aliases: ['siliconflow', 'siliconcloud', '硅基流动', '硅基'],
}

/** 火山引擎（方舟大模型 / 豆包）开放平台端点 */
export const VOLCENGINE_PRESET: OpenAiCompatPreset = {
  key: 'volcengine',
  label: '火山引擎',
  iconProvider: 'doubao',
  name: 'volcengine',
  baseUrl: 'https://ark.cn-beijing.volces.com/api/v3',
  prefix: 'volcengine/',
  modelsUrl: 'https://ark.cn-beijing.volces.com/api/v3/models',
  aliases: ['volcengine', 'volces', 'doubao', '火山引擎', '豆包'],
}

/** 零一万物（01.AI）开放平台端点 */
export const LINGYI_PRESET: OpenAiCompatPreset = {
  key: 'lingyi',
  label: '零一万物',
  iconProvider: 'yi',
  name: 'lingyi',
  baseUrl: 'https://api.lingyiwanwu.com/v1',
  prefix: 'lingyi/',
  modelsUrl: 'https://api.lingyiwanwu.com/v1/models',
  aliases: ['lingyi', '01ai', '零一万物', 'lingyiwanwu', 'yi'],
}

/** 阶跃星辰（StepFun）开放平台端点 */
export const STEPFUN_PRESET: OpenAiCompatPreset = {
  key: 'stepfun',
  label: '阶跃星辰',
  iconProvider: 'stepfun',
  name: 'stepfun',
  baseUrl: 'https://api.stepfun.com/v1',
  prefix: 'stepfun/',
  modelsUrl: 'https://api.stepfun.com/v1/models',
  aliases: ['stepfun', '阶跃星辰', 'step'],
}

/** 百川智能（Baichuan）开放平台端点 */
export const BAICHUAN_PRESET: OpenAiCompatPreset = {
  key: 'baichuan',
  label: '百川智能',
  iconProvider: 'baichuan',
  name: 'baichuan',
  baseUrl: 'https://api.baichuan-ai.com/v1',
  prefix: 'baichuan/',
  modelsUrl: 'https://api.baichuan-ai.com/v1/models',
  aliases: ['baichuan', '百川智能', '百川'],
}

/** 小米 MiMo（Xiaomi MiMo）大模型开放平台端点 */
export const MIMO_PRESET: OpenAiCompatPreset = {
  key: 'mimo',
  label: '小米 MiMo',
  iconProvider: 'mimo',
  name: 'mimo',
  baseUrl: 'https://api.xiaomimimo.com/v1',
  prefix: 'mimo/',
  modelsUrl: 'https://api.xiaomimimo.com/v1/models',
  aliases: ['mimo', 'xiaomi', '小米', 'xiaomimimo'],
}

export const OPENAI_COMPAT_PRESETS: OpenAiCompatPreset[] = [
  BAILIAN_PRESET,
  DEEPSEEK_PRESET,
  ZHIPU_PRESET,
  MOONSHOT_PRESET,
  MINIMAX_PRESET,
  SILICONFLOW_PRESET,
  VOLCENGINE_PRESET,
  LINGYI_PRESET,
  STEPFUN_PRESET,
  BAICHUAN_PRESET,
  MIMO_PRESET,
]

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
 * 兼容端点上（忽略结尾斜杠与版本段 /v1、/v3、/v4 等）。都不像就返回 `undefined` —— 自填的通用渠道走原路。
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
  if (candidate.host === target.host && candidate.rootPath === target.rootPath) {
    return true
  }
  // 百炼平台支持专属实例（*.maas.aliyuncs.com）与国际站（dashscope-intl.aliyuncs.com）端点
  if (preset.key === 'bailian' && candidate.rootPath === target.rootPath) {
    if (candidate.host === 'dashscope-intl.aliyuncs.com' || candidate.host.endsWith('.maas.aliyuncs.com')) {
      return true
    }
  }
  return false
}

/**
 * 提取 API 端点的规范根路径（忽略尾随斜杠与末尾版本号段，如 /v1、/v3、/v4）。
 * `https://dashscope.aliyuncs.com/compatible-mode/v1` → host + `/compatible-mode`
 * `https://api.deepseek.com/v1` → host + ``
 */
function endpointRoot(url: string): { host: string; rootPath: string } | undefined {
  try {
    const parsed = new URL(url)
    const normalizedPath = parsed.pathname
      .replace(/\/+$/, '')
      .replace(/\/v\d+$/i, '')
    return { host: parsed.host.toLowerCase(), rootPath: normalizedPath }
  } catch {
    return undefined
  }
}
