import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { useAuthStore as AuthStore } from './auth_store'

const user = { id: 1, email: 'admin@example.com', role: 'user' }

// 测试环境不提供 localStorage（同 client.property.test.ts 的处理思路），
// 这里用与 Web Storage API 对齐的内存替身，保证 store 的持久化契约可测。
function memoryStorage(): Storage {
  const map = new Map<string, string>()
  return {
    get length() {
      return map.size
    },
    clear: () => map.clear(),
    getItem: (k) => (map.has(k) ? (map.get(k) as string) : null),
    key: (i) => [...map.keys()][i] ?? null,
    removeItem: (k) => {
      map.delete(k)
    },
    setItem: (k, v) => {
      map.set(k, String(v))
    },
  }
}

// 每个用例都重新加载模块：store 的恢复行为发生在模块初始化时，
// 只有 resetModules + 动态 import 才能真正测到「刷新后恢复」。
async function freshStore() {
  vi.resetModules()
  const mod = await import('./auth_store')
  return mod.useAuthStore as typeof AuthStore
}

beforeEach(() => {
  vi.stubGlobal('localStorage', memoryStorage())
})

describe('auth_store session persistence', () => {
  it('setAuth persists token+user and a fresh store restores them', async () => {
    let store = await freshStore()
    expect(store.getState().token).toBeNull()
    store.getState().setAuth('jwt-token-abc', user)
    expect(localStorage.getItem('agw_token:v1')).toBe('jwt-token-abc')

    store = await freshStore()
    expect(store.getState().token).toBe('jwt-token-abc')
    expect(store.getState().user?.email).toBe('admin@example.com')
  })

  it('logout clears the persisted session', async () => {
    let store = await freshStore()
    store.getState().setAuth('jwt-token-abc', user)
    store.getState().logout()
    expect(localStorage.getItem('agw_token:v1')).toBeNull()
    expect(localStorage.getItem('agw_user:v1')).toBeNull()

    store = await freshStore()
    expect(store.getState().token).toBeNull()
    expect(store.getState().user).toBeNull()
  })

  it('ignores a corrupt cached user but keeps the token', async () => {
    localStorage.setItem('agw_user:v1', '{not json')
    localStorage.setItem('agw_token:v1', 'jwt-token-abc')
    const store = await freshStore()
    expect(store.getState().user).toBeNull()
    expect(store.getState().token).toBe('jwt-token-abc')
  })

  it('legacy bare agw_* keys are dropped on next setAuth', async () => {
    localStorage.setItem('agw_token', 'stale')
    localStorage.setItem('cpa_user', 'stale')
    const store = await freshStore()
    store.getState().setAuth('jwt-token-abc', user)
    expect(localStorage.getItem('agw_token')).toBeNull()
    expect(localStorage.getItem('cpa_user')).toBeNull()
    expect(localStorage.getItem('agw_token:v1')).toBe('jwt-token-abc')
  })
})
