import type { ComponentType } from 'react'
import AlibabaColor from '@lobehub/icons/es/Alibaba/components/Color'
import AlibabaCloudColor from '@lobehub/icons/es/AlibabaCloud/components/Color'
import AntigravityColor from '@lobehub/icons/es/Antigravity/components/Color'
import BailianColor from '@lobehub/icons/es/Bailian/components/Color'
import BaichuanColor from '@lobehub/icons/es/Baichuan/components/Color'
import ChatGLMColor from '@lobehub/icons/es/ChatGLM/components/Color'
import ClaudeColor from '@lobehub/icons/es/Claude/components/Color'
import CodexColor from '@lobehub/icons/es/Codex/components/Color'
import CopilotColor from '@lobehub/icons/es/Copilot/components/Color'
import CursorMono from '@lobehub/icons/es/Cursor/components/Mono'
import DeepSeekColor from '@lobehub/icons/es/DeepSeek/components/Color'
import DoubaoColor from '@lobehub/icons/es/Doubao/components/Color'
import GeminiColor from '@lobehub/icons/es/Gemini/components/Color'
import GeminiCLIColor from '@lobehub/icons/es/GeminiCLI/components/Color'
import IFlyTekCloudColor from '@lobehub/icons/es/IFlyTekCloud/components/Color'
import KimiColor from '@lobehub/icons/es/Kimi/components/Color'
import KiroColor from '@lobehub/icons/es/Kiro/components/Color'
import MetaColor from '@lobehub/icons/es/Meta/components/Color'
import MinimaxColor from '@lobehub/icons/es/Minimax/components/Color'
import MistralColor from '@lobehub/icons/es/Mistral/components/Color'
import MoonshotMono from '@lobehub/icons/es/Moonshot/components/Mono'
import OllamaMono from '@lobehub/icons/es/Ollama/components/Mono'
import OpenAIMono from '@lobehub/icons/es/OpenAI/components/Mono'
import QwenColor from '@lobehub/icons/es/Qwen/components/Color'
import SiliconCloudColor from '@lobehub/icons/es/SiliconCloud/components/Color'
import StepfunColor from '@lobehub/icons/es/Stepfun/components/Color'
import XAIMono from '@lobehub/icons/es/XAI/components/Mono'
import XiaomiMiMoMono from '@lobehub/icons/es/XiaomiMiMo/components/Mono'
import YiColor from '@lobehub/icons/es/Yi/components/Color'
import ZhipuColor from '@lobehub/icons/es/Zhipu/components/Color'

type BrandIconProps = { size?: number; className?: string }

/** Color SVG icons only — avoids package index (Combine → @lobehub/ui). */
export const LOBE_BRAND_ICONS: Record<string, ComponentType<BrandIconProps>> = {
  anthropic: ClaudeColor,
  claude: ClaudeColor,
  codex: CodexColor,
  copilot: CopilotColor,
  cursor: CursorMono,
  openai: OpenAIMono,
  ollama: OllamaMono,
  gemini: GeminiCLIColor,
  'gemini-cli': GeminiCLIColor,
  google: GeminiColor,
  vertex: GeminiColor,
  'google-cloud': GeminiColor,
  antigravity: AntigravityColor,
  kimi: KimiColor,
  kiro: KiroColor,
  moonshot: MoonshotMono,
  xai: XAIMono,
  'x-ai': XAIMono,
  grok: XAIMono,
  qwen: QwenColor,
  // 百炼是平台（DashScope compatible-mode），Qwen 是模型家族 —— 图标各归各家。
  bailian: BailianColor,
  dashscope: BailianColor,
  alibaba: AlibabaColor,
  'alibaba-cloud': AlibabaCloudColor,
  iflow: IFlyTekCloudColor,
  iflytekcloud: IFlyTekCloudColor,
  deepseek: DeepSeekColor,
  mistral: MistralColor,
  minimax: MinimaxColor,
  meta: MetaColor,
  llama: MetaColor,
  glm: ChatGLMColor,
  zhipu: ZhipuColor,
  zai: ZhipuColor,
  bigmodel: ZhipuColor,
  siliconcloud: SiliconCloudColor,
  siliconflow: SiliconCloudColor,
  stepfun: StepfunColor,
  step: StepfunColor,
  yi: YiColor,
  lingyi: YiColor,
  baichuan: BaichuanColor,
  doubao: DoubaoColor,
  volcengine: DoubaoColor,
  volces: DoubaoColor,
  mimo: XiaomiMiMoMono,
  xiaomi: XiaomiMiMoMono,
}

export const LOBE_BRAND_ICON_ALIASES: Record<string, string> = {
  vertex: 'google',
  'google-cloud': 'google',
  iflow: 'iflytekcloud',
  'alibaba-cloud': 'alibaba',
  siliconflow: 'siliconcloud',
  lingyi: 'yi',
  volcengine: 'doubao',
  volces: 'doubao',
  step: 'stepfun',
  xiaomi: 'mimo',
  xiaomimimo: 'mimo',
}
