import { anthropicBaseUrl, openaiBaseUrl } from '@/pages/docs/guide'

export type GatewaySdkFamily = 'openai' | 'anthropic'

export type GatewayInferenceProtocol = {
  id: 'chat-completions' | 'responses' | 'messages'
  name: string
  method: 'POST'
  path: '/v1/chat/completions' | '/v1/responses' | '/v1/messages'
  sdk: GatewaySdkFamily
  clients: string
}

/** The three inference entries gw-proxy actually serves. */
export const GATEWAY_INFERENCE_PROTOCOLS: readonly GatewayInferenceProtocol[] = [
  {
    id: 'chat-completions',
    name: 'Chat Completions',
    method: 'POST',
    path: '/v1/chat/completions',
    sdk: 'openai',
    clients: 'OpenAI SDK、Cursor',
  },
  {
    id: 'responses',
    name: 'Responses',
    method: 'POST',
    path: '/v1/responses',
    sdk: 'openai',
    clients: 'Codex CLI、OpenAI SDK',
  },
  {
    id: 'messages',
    name: 'Anthropic Messages',
    method: 'POST',
    path: '/v1/messages',
    sdk: 'anthropic',
    clients: 'Claude Code、Anthropic SDK',
  },
]

export type GatewaySdkBase = {
  id: GatewaySdkFamily
  name: string
  hint: string
}

export const GATEWAY_SDK_BASES: readonly GatewaySdkBase[] = [
  {
    id: 'openai',
    name: 'OpenAI 兼容',
    hint: 'Chat Completions / Responses · SDK 填 /v1',
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    hint: 'Messages · SDK 填裸主机，不要加 /v1',
  },
]

export function protocolEndpointUrl(origin: string, path: string): string {
  return `${origin}${path}`
}

export function sdkBaseUrl(origin: string, family: GatewaySdkFamily): string {
  return family === 'openai' ? openaiBaseUrl(origin) : anthropicBaseUrl(origin)
}
