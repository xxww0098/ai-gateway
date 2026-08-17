import { describe, it, expect, afterEach } from 'vitest'
import { createElement } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { act } from 'react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import DocsPage from './DocsPage'
import { docsRoutes, docsPath, isDocsSlug } from '@/shared/routes/docs'
import { KEY_PREFIX, PRODUCT_NAME, openaiBaseUrl, anthropicBaseUrl } from './guide'

function renderAt(path: string): { container: HTMLDivElement; root: Root } {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() => {
    root.render(
      createElement(
        MemoryRouter,
        { initialEntries: [path] },
        createElement(
          Routes,
          null,
          createElement(Route, { path: '/docs', element: createElement(DocsPage) }),
          createElement(Route, { path: '/docs/:slug', element: createElement(DocsPage) }),
        ),
      ),
    )
  })
  return { container, root }
}

function cleanup(container: HTMLDivElement, root: Root) {
  act(() => {
    root.unmount()
  })
  document.body.removeChild(container)
}

function textOf(container: HTMLElement): string {
  return container.textContent || ''
}

describe('docsRoutes', () => {
  it('exposes public /docs paths', () => {
    expect(docsRoutes.root).toBe('/docs')
    expect(docsRoutes.quickstart).toBe('/docs/quickstart')
    expect(docsPath('openai')).toBe('/docs/openai')
    expect(isDocsSlug('cursor')).toBe(true)
    expect(isDocsSlug('gemini')).toBe(false)
  })
})

describe('DocsPage', () => {
  const mounted: Array<{ container: HTMLDivElement; root: Root }> = []

  afterEach(() => {
    for (const item of mounted.splice(0)) {
      cleanup(item.container, item.root)
    }
  })

  function mount(path: string) {
    const result = renderAt(path)
    mounted.push(result)
    return result.container
  }

  it('renders 接入指南 overview with gateway cards and three steps', () => {
    const container = mount('/docs')
    const text = textOf(container)
    expect(text).toContain(PRODUCT_NAME)
    expect(text).toContain('接入指南')
    expect(text).toContain('客户端接入')
    expect(text).toContain('网关地址')
    expect(text).toContain('三步接入')
    expect(text).toContain(openaiBaseUrl(window.location.origin))
    expect(text).toContain(anthropicBaseUrl(window.location.origin))
    expect(text).toContain(KEY_PREFIX)
    expect(text).toContain('/v1/chat/completions')
    expect(text).not.toContain('CPA Gateway')
    expect(text).not.toContain('cpa-')
    expect(text).not.toContain('/v1beta')
    expect(text).not.toContain('/claude-code')
    expect(text).not.toContain('/v1/cursor')
  })

  it('renders 快速接入 env examples', () => {
    const container = mount('/docs/quickstart')
    const text = textOf(container)
    expect(text).toContain('快速接入')
    expect(text).toContain('OPENAI_BASE_URL')
    expect(text).toContain('ANTHROPIC_BASE_URL')
    expect(text).toContain('ANTHROPIC_AUTH_TOKEN')
    expect(text).toContain(KEY_PREFIX)
    expect(text).not.toContain('cpa-')
  })

  it('renders honest client pages and skips unknown slugs', () => {
    expect(textOf(mount('/docs/openai'))).toContain('OpenAI SDK / curl')
    expect(textOf(mount('/docs/claude'))).toContain('Claude Code')
    expect(textOf(mount('/docs/claude'))).toContain('/v1/messages')
    expect(textOf(mount('/docs/codex'))).toContain('Codex CLI')
    expect(textOf(mount('/docs/cursor'))).toContain('Cursor')
    expect(textOf(mount('/docs/cursor'))).toContain('GET /v1/models')
  })
})
