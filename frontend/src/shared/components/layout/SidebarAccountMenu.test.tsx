import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { createElement, act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SidebarAccountMenu } from './SidebarAccountMenu'
import { useAuthStore } from '@/features/auth/auth_store'
import { userRoutes } from '@/shared/routes/user'

const serverLogout = vi.fn(() => Promise.resolve({ ok: true }))

vi.mock('@/features/auth/api', () => ({
  logout: () => serverLogout(),
}))

const fixtureUser = {
  id: 17,
  email: 'casey@example.test',
  role: 'user' as const,
}

function PathProbe() {
  const location = useLocation()
  return createElement('div', { 'data-testid': 'path' }, location.pathname)
}

function renderMenu(): { container: HTMLDivElement; root: Root } {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  act(() => {
    root.render(
      createElement(
        QueryClientProvider,
        { client },
        createElement(
          MemoryRouter,
          { initialEntries: [userRoutes.dashboard] },
          createElement(
            Routes,
            null,
            createElement(Route, {
              path: '*',
              element: createElement(
                'div',
                null,
                createElement(SidebarAccountMenu, { collapsed: false, onNavigate: () => {} }),
                createElement(PathProbe),
              ),
            }),
          ),
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

function triggerOf(container: HTMLElement): HTMLButtonElement {
  const btn = container.querySelector<HTMLButtonElement>('button[aria-label="个人中心"]')
  if (!btn) throw new Error('account trigger missing')
  return btn
}

function openMenu(trigger: HTMLElement) {
  act(() => {
    trigger.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true, cancelable: true, pointerType: 'mouse', button: 0 }),
    )
    trigger.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
  })
}

function menuItems(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[role="menuitem"]'))
}

describe('SidebarAccountMenu', () => {
  const mounted: Array<{ container: HTMLDivElement; root: Root }> = []

  beforeEach(() => {
    serverLogout.mockClear()
    useAuthStore.setState({ token: 'tok', user: { ...fixtureUser } })
  })

  afterEach(() => {
    for (const item of mounted.splice(0)) {
      cleanup(item.container, item.root)
    }
  })

  function mount() {
    const result = renderMenu()
    mounted.push(result)
    return result.container
  }

  it('shows the signed-in identity on the trigger and keeps actions out of the closed footer', () => {
    const container = mount()
    expect(triggerOf(container).textContent).toContain(fixtureUser.email)
    expect(document.querySelector('[role="menu"]')).toBeNull()
    expect(menuItems()).toHaveLength(0)
  })

  it('opens a popup whose destinations include finance, tickets, keys, and docs', () => {
    const container = mount()
    openMenu(triggerOf(container))

    const menu = document.querySelector('[role="menu"]')
    expect(menu).not.toBeNull()
    expect(menu?.textContent).toContain(fixtureUser.email)

    const labels = menuItems().map((el) => el.textContent || '')
    expect(labels.some((t) => t.includes('财务'))).toBe(true)
    expect(labels.some((t) => t.includes('工单'))).toBe(true)
    expect(labels.some((t) => t.includes('密钥'))).toBe(true)
    expect(labels.some((t) => t.includes('接入'))).toBe(true)
  })

  it('navigates to the finance route from the popup', () => {
    const container = mount()
    openMenu(triggerOf(container))
    const finance = menuItems().find((el) => (el.textContent || '').includes('财务'))
    expect(finance).toBeTruthy()
    act(() => {
      finance?.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, cancelable: true, button: 0 }))
      finance?.click()
    })
    expect(container.querySelector('[data-testid="path"]')?.textContent).toBe(userRoutes.finance)
  })

  it('signs out through the popup and lands on login', async () => {
    const container = mount()
    openMenu(triggerOf(container))
    const logoutItem = menuItems().find((el) => (el.textContent || '').includes('退出'))
    expect(logoutItem).toBeTruthy()
    await act(async () => {
      logoutItem?.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, cancelable: true, button: 0 }))
      logoutItem?.click()
      await Promise.resolve()
    })
    expect(serverLogout).toHaveBeenCalledTimes(1)
    expect(useAuthStore.getState().user).toBeNull()
    expect(useAuthStore.getState().token).toBeNull()
    expect(container.querySelector('[data-testid="path"]')?.textContent).toBe('/login')
  })
})
