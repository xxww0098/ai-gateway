import { describe, it, expect, afterEach } from 'vitest'
import { createElement } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { act } from 'react'
import { QuickIntegrationPanel } from './QuickIntegrationPanel'
import { anthropicBaseUrl, openaiBaseUrl } from '@/pages/docs/guide'
import type { IntegrationTab } from '../types'

function renderPanel(tab: IntegrationTab): { container: HTMLDivElement; root: Root } {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() => {
    root.render(
      createElement(QuickIntegrationPanel, {
        apiKeyCount: 1,
        totalRequests: 0,
        integrationTab: tab,
        onIntegrationTabChange: () => {},
      }),
    )
  })
  return { container, root }
}

describe('QuickIntegrationPanel', () => {
  const mounted: Array<{ container: HTMLDivElement; root: Root }> = []

  afterEach(() => {
    for (const item of mounted.splice(0)) {
      act(() => {
        item.root.unmount()
      })
      document.body.removeChild(item.container)
    }
  })

  function mount(tab: IntegrationTab) {
    const result = renderPanel(tab)
    mounted.push(result)
    return result.container
  }

  it('prints OpenAI base as {origin}/v1 with Bearer', () => {
    const text = mount('openai').textContent || ''
    expect(text).toContain(openaiBaseUrl(window.location.origin))
    expect(text).toContain('Authorization: Bearer')
    expect(text).toContain('AI-GateWay')
    expect(text).not.toContain('/v1beta')
    expect(text).not.toContain('x-api-key')
    expect(text).not.toContain('cpa-')
    expect(text).not.toContain('CPA Gateway')
  })

  it('prints Anthropic base as bare origin with Bearer, matching /docs', () => {
    const origin = window.location.origin
    const text = mount('anthropic').textContent || ''
    expect(text).toContain(anthropicBaseUrl(origin))
    expect(text).toContain('Authorization: Bearer')
    expect(text).toContain('anthropic-version')
    expect(text).not.toContain(`${origin}/v1beta`)
    expect(text).not.toContain('x-api-key')
    expect(text).not.toContain('cpa-')
  })
})
