import { describe, it, expect, afterEach } from 'vitest'
import { createElement } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { act } from 'react'
import { MemoryRouter } from 'react-router-dom'
import { GatewayEndpointBar } from './GatewayEndpointBar'
import { GATEWAY_INFERENCE_PROTOCOLS, sdkBaseUrl } from '../endpoints'

function renderBar(): { container: HTMLDivElement; root: Root } {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() => {
    root.render(createElement(MemoryRouter, null, createElement(GatewayEndpointBar)))
  })
  return { container, root }
}

describe('GatewayEndpointBar', () => {
  const mounted: Array<{ container: HTMLDivElement; root: Root }> = []

  afterEach(() => {
    for (const item of mounted.splice(0)) {
      act(() => {
        item.root.unmount()
      })
      document.body.removeChild(item.container)
    }
  })

  it('lists the three inference POST paths and both SDK bases', () => {
    const result = renderBar()
    mounted.push(result)
    const text = result.container.textContent || ''
    const origin = window.location.origin

    for (const protocol of GATEWAY_INFERENCE_PROTOCOLS) {
      expect(text).toContain(protocol.name)
      expect(text).toContain(protocol.path)
    }

    expect(text).toContain(sdkBaseUrl(origin, 'openai'))
    expect(text).toContain(sdkBaseUrl(origin, 'anthropic'))
    expect(text).toContain('查看快速接入指南')
    expect(text).not.toContain('OpenAI Base URL')
    expect(text).not.toContain('Claude 端点')
    expect(text).not.toContain('/v1beta')

    const openaiBase = result.container.querySelector('[aria-label="复制 OpenAI 兼容 Base URL"]')
    const anthropicBase = result.container.querySelector('[aria-label="复制 Anthropic Base URL"]')
    expect(openaiBase?.getAttribute('title')).toContain(sdkBaseUrl(origin, 'openai'))
    expect(anthropicBase?.getAttribute('title')).toContain(sdkBaseUrl(origin, 'anthropic'))

    const chatCopy = result.container.querySelector('[aria-label="复制 Chat Completions 端点"]')
    expect(chatCopy?.getAttribute('title')).toContain(`${origin}/v1/chat/completions`)
  })
})
