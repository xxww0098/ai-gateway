import { docsPath, type DocsSlug } from '@/shared/routes/docs'

/** Panel-issued user API keys start with this prefix. */
export const KEY_PREFIX = 'agw-'

export const PRODUCT_NAME = 'AI-GateWay'

export function gatewayOrigin(): string {
  return window.location.origin
}

export function openaiBaseUrl(origin: string): string {
  return `${origin}/v1`
}

export function anthropicBaseUrl(origin: string): string {
  return origin
}

export function sampleKey(): string {
  return `${KEY_PREFIX}xxxxxxxx`
}

export type DocsNavItem = {
  slug?: DocsSlug
  label: string
  hint?: string
}

export type DocsNavGroup = {
  title: string
  items: DocsNavItem[]
}

export const docsNav: DocsNavGroup[] = [
  {
    title: '开始',
    items: [
      { label: '总览', hint: '接入指南' },
      { slug: 'quickstart', label: '快速接入' },
    ],
  },
  {
    title: '工具',
    items: [
      { slug: 'openai', label: 'OpenAI SDK / curl' },
      { slug: 'claude', label: 'Claude Code' },
      { slug: 'codex', label: 'Codex CLI' },
      { slug: 'cursor', label: 'Cursor' },
    ],
  },
]

export function navHref(item: DocsNavItem): string {
  return docsPath(item.slug)
}

export function firstRequestCurl(origin: string): string {
  const key = sampleKey()
  return [
    `curl ${openaiBaseUrl(origin)}/chat/completions \\`,
    `  -H "Authorization: Bearer ${key}" \\`,
    `  -H "Content-Type: application/json" \\`,
    `  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}'`,
  ].join('\n')
}

export function openaiEnv(origin: string): string {
  return [
    `export OPENAI_BASE_URL="${openaiBaseUrl(origin)}"`,
    `export OPENAI_API_KEY="${sampleKey()}"`,
  ].join('\n')
}

export function anthropicEnv(origin: string): string {
  return [
    `export ANTHROPIC_BASE_URL="${anthropicBaseUrl(origin)}"`,
    `export ANTHROPIC_AUTH_TOKEN="${sampleKey()}"`,
  ].join('\n')
}

export function pythonOpenaiSnippet(origin: string): string {
  return [
    'from openai import OpenAI',
    '',
    'client = OpenAI(',
    `    base_url="${openaiBaseUrl(origin)}",`,
    `    api_key="${sampleKey()}",`,
    ')',
    'resp = client.chat.completions.create(',
    '    model="gpt-4o",',
    '    messages=[{"role": "user", "content": "hi"}],',
    ')',
    'print(resp.choices[0].message.content)',
  ].join('\n')
}
