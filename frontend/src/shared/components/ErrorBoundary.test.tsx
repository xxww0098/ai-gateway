import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { createElement } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { act } from 'react'
import { ErrorBoundary } from './ErrorBoundary'

// Suppress React error boundary console.error noise during tests
let originalConsoleError: typeof console.error

beforeEach(() => {
  originalConsoleError = console.error
  console.error = vi.fn()
})

afterEach(() => {
  console.error = originalConsoleError
})

/** A component that always throws on render */
function ThrowingComponent({ message }: { message: string }): never {
  throw new Error(message)
}

/** A component that renders normally */
function GoodComponent({ text }: { text: string }) {
  return createElement('div', { 'data-testid': 'good' }, text)
}

function renderToContainer(element: React.ReactNode): { container: HTMLDivElement; root: Root } {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  act(() => {
    root.render(element)
  })
  return { container, root }
}

function cleanup(container: HTMLDivElement, root: Root) {
  act(() => {
    root.unmount()
  })
  document.body.removeChild(container)
}

describe('ErrorBoundary', () => {
  it('renders children when no error occurs', () => {
    const { container, root } = renderToContainer(
      createElement(ErrorBoundary, null,
        createElement(GoodComponent, { text: 'Hello' })
      )
    )

    expect(container.textContent).toContain('Hello')
    expect(container.querySelector('[data-testid="good"]')).not.toBeNull()
    cleanup(container, root)
  })

  it('displays user-friendly error message when child throws', () => {
    const { container, root } = renderToContainer(
      createElement(ErrorBoundary, null,
        createElement(ThrowingComponent, { message: 'Test crash' })
      )
    )

    // Should show the error fallback UI
    expect(container.textContent).toContain('页面加载出错')
    expect(container.textContent).toContain('该页面遇到了一个意外错误')
    expect(container.textContent).toContain('Test crash')
    // Should have a retry button
    expect(container.textContent).toContain('重新加载')
    cleanup(container, root)
  })

  it('provides a working retry button that resets the boundary', () => {
    // We'll use a flag to control whether the component throws
    let shouldThrow = true

    function ConditionalThrower() {
      if (shouldThrow) throw new Error('Conditional error')
      return createElement('div', { 'data-testid': 'recovered' }, 'Recovered!')
    }

    const { container, root } = renderToContainer(
      createElement(ErrorBoundary, null,
        createElement(ConditionalThrower)
      )
    )

    // Initially shows error
    expect(container.textContent).toContain('页面加载出错')

    // Fix the component and click retry
    shouldThrow = false
    const retryButton = container.querySelector('button')
    expect(retryButton).not.toBeNull()

    act(() => {
      retryButton!.click()
    })

    // Should now render the recovered component
    expect(container.textContent).toContain('Recovered!')
    expect(container.querySelector('[data-testid="recovered"]')).not.toBeNull()
    cleanup(container, root)
  })

  it('error in one ErrorBoundary does not affect sibling ErrorBoundary', () => {
    // Simulates the App.tsx pattern where each route is wrapped independently
    const { container, root } = renderToContainer(
      createElement('div', null,
        createElement(ErrorBoundary, null,
          createElement(ThrowingComponent, { message: 'Module A crashed' })
        ),
        createElement(ErrorBoundary, null,
          createElement(GoodComponent, { text: 'Module B works' })
        )
      )
    )

    // Module A should show error fallback
    expect(container.textContent).toContain('页面加载出错')
    expect(container.textContent).toContain('Module A crashed')

    // Module B should render normally — error isolation confirmed
    expect(container.textContent).toContain('Module B works')
    expect(container.querySelector('[data-testid="good"]')).not.toBeNull()
    cleanup(container, root)
  })

  it('calls onReset callback when retry is clicked', () => {
    const onReset = vi.fn()
    let shouldThrow = true

    function ConditionalThrower() {
      if (shouldThrow) throw new Error('Reset test')
      return createElement('div', null, 'OK')
    }

    const { container, root } = renderToContainer(
      createElement(ErrorBoundary, { onReset },
        createElement(ConditionalThrower)
      )
    )

    expect(container.textContent).toContain('页面加载出错')

    shouldThrow = false
    const retryButton = container.querySelector('button')
    act(() => {
      retryButton!.click()
    })

    expect(onReset).toHaveBeenCalledTimes(1)
    cleanup(container, root)
  })

  it('renders custom fallback when provided', () => {
    const customFallback = createElement('div', { 'data-testid': 'custom' }, 'Custom Error UI')

    const { container, root } = renderToContainer(
      createElement(ErrorBoundary, { fallback: customFallback },
        createElement(ThrowingComponent, { message: 'ignored' })
      )
    )

    expect(container.querySelector('[data-testid="custom"]')).not.toBeNull()
    expect(container.textContent).toContain('Custom Error UI')
    // Should NOT show default fallback
    expect(container.textContent).not.toContain('页面加载出错')
    cleanup(container, root)
  })
})
