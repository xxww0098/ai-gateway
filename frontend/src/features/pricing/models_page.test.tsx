import { describe, expect, it } from 'vitest'
import { createElement, act } from 'react'
import { createRoot } from 'react-dom/client'
import { MemoryRouter } from 'react-router-dom'
import { ModelEmptyState } from './components/ModelEmptyState'
import { ModelCatalogHeader } from './components/ModelCatalogHeader'
import { formatPrice } from './components/ModelCatalogCard'
import { getProviderBrandIcon } from './modelCatalogUtils'
import { ModelQuickCodeDialog } from './components/ModelQuickCodeDialog'
import type { ModelCatalogItem } from './model_catalog'

function renderWithRouter(element: React.ReactElement) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() => {
    root.render(createElement(MemoryRouter, null, element))
  })
  return {
    container,
    cleanup: () => {
      act(() => {
        root.unmount()
      })
      container.remove()
    },
  }
}

describe('Models UI components', () => {
  describe('formatPrice', () => {
    it('formats 0 or undefined as 免费', () => {
      expect(formatPrice(0)).toBe('免费')
      expect(formatPrice(undefined)).toBe('免费')
    })

    it('formats small prices with 4 decimal digits', () => {
      expect(formatPrice(0.0015)).toBe('$0.0015')
      expect(formatPrice(0.0004)).toBe('$0.0004')
    })

    it('formats normal prices with 2 decimal digits', () => {
      expect(formatPrice(1.5)).toBe('$1.50')
      expect(formatPrice(15)).toBe('$15.00')
    })
  })

  describe('getProviderBrandIcon', () => {
    it('resolves icons for major providers and aliases', () => {
      expect(getProviderBrandIcon('anthropic')).not.toBeNull()
      expect(getProviderBrandIcon('claude')).not.toBeNull()
      expect(getProviderBrandIcon('openai')).not.toBeNull()
      expect(getProviderBrandIcon('google')).not.toBeNull()
      expect(getProviderBrandIcon('vertex')).not.toBeNull()
      expect(getProviderBrandIcon('deepseek')).not.toBeNull()
      expect(getProviderBrandIcon('unknown_custom_provider')).toBeNull()
    })
  })

  describe('ModelEmptyState', () => {
    it('renders 3-step guided architecture for admin', () => {
      const { container, cleanup } = renderWithRouter(<ModelEmptyState isAdmin={true} />)

      expect(container.textContent).toContain('还没有可用模型')
      expect(container.textContent).toContain('接入上游渠道')
      expect(container.textContent).toContain('开放与映射模型')
      expect(container.textContent).toContain('精算定价与调用')
      expect(container.textContent).toContain('前往渠道管理配置')
      cleanup()
    })

    it('renders ticket guidance for regular user', () => {
      const { container, cleanup } = renderWithRouter(<ModelEmptyState isAdmin={false} />)

      expect(container.textContent).toContain('提交工单咨询管理员')
      cleanup()
    })

    it('renders filter-miss state with reset button', () => {
      let resetCalled = false
      const { container, cleanup } = renderWithRouter(
        <ModelEmptyState
          isAdmin={false}
          isFilterMiss={true}
          onClearFilter={() => {
            resetCalled = true
          }}
        />
      )

      expect(container.textContent).toContain('未找到匹配的模型')
      const button = container.querySelector('button')
      expect(button).not.toBeNull()
      button?.click()
      expect(resetCalled).toBe(true)
      cleanup()
    })
  })

  describe('ModelCatalogHeader', () => {
    it('renders total models, provider count and rate multiplier badge', () => {
      const { container, cleanup } = renderWithRouter(
        <ModelCatalogHeader totalModels={42} totalProviders={8} rateMultiplier={1.2} />
      )

      expect(container.textContent).toContain('模型广场与定价')
      expect(container.textContent).toContain('42')
      expect(container.textContent).toContain('8')
      expect(container.textContent).toContain('全局费率 1.2x')
      cleanup()
    })
  })

  describe('ModelQuickCodeDialog', () => {
    it('renders integration snippets with correct model id', () => {
      const mockModel: ModelCatalogItem = {
        id: 'claude-3-7-sonnet',
        display_name: 'Claude 3.7 Sonnet',
        owned_by: 'anthropic',
        context_length: 200000,
      }

      const { cleanup } = renderWithRouter(
        <ModelQuickCodeDialog model={mockModel} open={true} onOpenChange={() => {}} />
      )

      expect(document.body.textContent).toContain('Claude 3.7 Sonnet')
      expect(document.body.textContent).toContain('claude-3-7-sonnet')
      expect(document.body.textContent).toContain('cURL')
      expect(document.body.textContent).toContain('Python')
      expect(document.body.textContent).toContain('Node.js')
      cleanup()
    })
  })
})
