import { LlmAdapter, LlmError, attributionHeaders } from '@deepseek-ai/dsh-llm'
import type { GenerateOptions, StreamChunk } from '@deepseek-ai/dsh-llm'
import { PROVIDER, toListModel, toResolvedModel, type GatewayModel } from './catalog.js'
import { contentToOpenAi, messagesToOpenAi, parseOpenAiSse } from './stream.js'

export interface AdapterAuth {
  origin: string
  apiKey: string
}

export class AgwAdapter extends LlmAdapter {
  constructor(
    private readonly auth: () => AdapterAuth | undefined,
    private readonly models: () => GatewayModel[],
  ) {
    super()
  }

  override providerInfo(provider: string) {
    return { id: provider, name: 'AI-GateWay' }
  }

  override listModels(provider: string) {
    return Promise.resolve(this.models().map(model => toListModel(provider, model)))
  }

  override resolveModel(provider: string, model: string, _signal?: AbortSignal) {
    const found = this.models().find(entry => entry.id === model)
    if (found === undefined) {
      return Promise.reject(new LlmError(`AI-GateWay has no model "${model}"`, 'UNKNOWN_MODEL'))
    }
    return Promise.resolve(toResolvedModel(provider, found))
  }

  async *stream(options: GenerateOptions): AsyncIterable<StreamChunk> {
    const auth = this.auth()
    if (auth === undefined) {
      throw new LlmError('AI-GateWay is not logged in; run /agw login', 'MISSING_CREDENTIAL')
    }
    const model = this.models().find(entry => entry.id === options.model)
    if (model !== undefined && options.reasoningEffort !== undefined) {
      const allowed = new Set(model.reasoning?.efforts.map(e => e.id) ?? [])
      if (allowed.size === 0 || !allowed.has(options.reasoningEffort)) {
        throw new LlmError(
          `AI-GateWay model "${options.model}" does not support reasoning effort "${options.reasoningEffort}"`,
          'UNSUPPORTED',
        )
      }
    }
    const body: Record<string, unknown> = {
      model: options.model,
      stream: true,
      messages: messagesToOpenAi(options),
    }
    if (options.temperature !== undefined) body.temperature = options.temperature
    if (options.maxTokens !== undefined) body.max_tokens = options.maxTokens
    if (options.stop !== undefined) body.stop = options.stop
    if (options.tools !== undefined) body.tools = options.tools
    if (options.reasoningEffort !== undefined) body.reasoning_effort = options.reasoningEffort

    const response = await fetch(`${auth.origin}/v1/chat/completions`, {
      method: 'POST',
      headers: {
        authorization: `Bearer ${auth.apiKey}`,
        'content-type': 'application/json',
        accept: 'text/event-stream',
        ...attributionHeaders(),
      },
      body: JSON.stringify(body),
      ...options.signal === undefined ? {} : { signal: options.signal },
    })
    if (!response.ok) {
      throw new LlmError(`AI-GateWay HTTP ${response.status}`, 'PROVIDER_HTTP_ERROR', {
        status: response.status,
      })
    }
    if (response.body === null) {
      throw new LlmError('AI-GateWay returned an empty body', 'EMPTY_RESPONSE')
    }

    let textIndex: number | undefined
    let text = ''
    let usage: { inputTokens: number, outputTokens: number } | undefined
    let finish: string | undefined
    for await (const event of parseOpenAiSse(response.body)) {
      const choice = Array.isArray(event.choices) ? event.choices[0] as Record<string, unknown> | undefined : undefined
      const delta = choice?.delta as Record<string, unknown> | undefined
      const piece = typeof delta?.content === 'string' ? delta.content : ''
      if (piece.length > 0) {
        if (textIndex === undefined) {
          textIndex = 0
          yield { type: 'block-start', index: 0, blockType: 'text' }
        }
        text += piece
        yield { type: 'text-delta', index: 0, text: piece }
      }
      const usageRaw = event.usage as { prompt_tokens?: number, completion_tokens?: number } | undefined
      if (usageRaw !== undefined) {
        usage = {
          inputTokens: usageRaw.prompt_tokens ?? 0,
          outputTokens: usageRaw.completion_tokens ?? 0,
        }
      }
      if (typeof choice?.finish_reason === 'string') finish = choice.finish_reason
    }
    if (textIndex !== undefined) {
      yield { type: 'block-end', index: 0, block: { type: 'text', text } }
    }
    if (usage !== undefined) yield { type: 'usage', usage }
    yield {
      type: 'finish',
      reason: { kind: finish === 'tool_calls' ? 'tool-calls' : finish === 'length' ? 'max-tokens' : 'stop' },
    }
  }
}

export { PROVIDER, contentToOpenAi }
