/** OpenAI-compatible chat/completions request + SSE → Harness StreamChunks. */

export interface OpenAiMessage {
  role: 'system' | 'user' | 'assistant' | 'tool'
  content: unknown
  tool_call_id?: string
  tool_calls?: unknown
}

export function imageDataUrl(attachment: unknown): string | undefined {
  if (attachment === null || typeof attachment !== 'object') return undefined
  const rec = attachment as Record<string, unknown>
  if (typeof rec.url === 'string' && rec.url.length > 0) return rec.url
  if (typeof rec.data === 'string' && rec.data.length > 0) {
    if (rec.data.startsWith('data:')) return rec.data
    const mime = typeof rec.mediaType === 'string'
      ? rec.mediaType
      : typeof rec.mimeType === 'string' ? rec.mimeType : 'image/png'
    return `data:${mime};base64,${rec.data}`
  }
  return undefined
}

export function contentToOpenAi(blocks: readonly { type: string, [k: string]: unknown }[]): unknown {
  const parts: unknown[] = []
  for (const block of blocks) {
    if (block.type === 'text' && typeof block.text === 'string') {
      parts.push({ type: 'text', text: block.text })
    } else if (block.type === 'image') {
      const url = imageDataUrl(block.attachment)
      if (url !== undefined) parts.push({ type: 'image_url', image_url: { url } })
    } else if (block.type === 'tool-result') {
      const inner = Array.isArray(block.content) ? block.content as { type?: string, text?: string }[] : []
      const text = inner.filter(b => b.type === 'text').map(b => b.text ?? '').join('\n')
      parts.push({ type: 'text', text })
    }
  }
  if (parts.length === 1 && (parts[0] as { type?: string }).type === 'text') {
    return (parts[0] as { text: string }).text
  }
  return parts
}

export function messagesToOpenAi(options: {
  system?: string
  messages: readonly { role: string, content: readonly { type: string, [k: string]: unknown }[] }[]
}): OpenAiMessage[] {
  const out: OpenAiMessage[] = []
  if (options.system !== undefined && options.system.length > 0) {
    out.push({ role: 'system', content: options.system })
  }
  for (const message of options.messages) {
    const role = message.role === 'assistant' ? 'assistant' : 'user'
    out.push({ role, content: contentToOpenAi(message.content) })
  }
  return out
}

export async function* parseOpenAiSse(body: ReadableStream<Uint8Array>): AsyncGenerator<Record<string, unknown>> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })
    const chunks = buffer.split('\n\n')
    buffer = chunks.pop() ?? ''
    for (const chunk of chunks) {
      for (const line of chunk.split('\n')) {
        const trimmed = line.startsWith('data:') ? line.slice(5).trim() : ''
        if (trimmed.length === 0 || trimmed === '[DONE]') continue
        try {
          yield JSON.parse(trimmed) as Record<string, unknown>
        } catch {
          // ignore keep-alives / comments
        }
      }
    }
  }
}
