import { create } from 'zustand'

interface User {
  id: number
  email: string
  role: string
  balance?: number
}

interface AuthState {
  token: string | null
  user: User | null
  setAuth: (token: string, user: User) => void
  updateUser: (user: Partial<User>) => void
  logout: () => void
}

// 会话持久化（v1 版本化键）：token 与 user 一并落 localStorage。
// 刷新后由这里恢复 token，UserLayout 的 useProfile 静默校验；
// token 失效时 client.ts 收到 401 统一 logout，无需额外的过期刷新逻辑。
const TOKEN_KEY = 'agw_token:v1'
const USER_KEY = 'agw_user:v1'

function readCachedToken(): string | null {
  try {
    const raw = localStorage.getItem(TOKEN_KEY)
    return raw && raw.length > 0 ? raw : null
  } catch {
    // private mode / storage blocked
    return null
  }
}

function readCachedUser(): User | null {
  try {
    const raw = localStorage.getItem(USER_KEY)
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object') return null
    const row = parsed as Partial<User>
    if (typeof row.id !== 'number' || typeof row.email !== 'string' || typeof row.role !== 'string') {
      return null
    }
    return {
      id: row.id,
      email: row.email,
      role: row.role,
      balance: typeof row.balance === 'number' ? row.balance : undefined,
    }
  } catch {
    try {
      localStorage.removeItem(USER_KEY)
    } catch {
      // private mode / storage blocked
    }
    return null
  }
}

// 历史遗留键（cpa-* 品牌与未版本化的 agw 裸键），一律清掉。
function dropObsoleteAuthKeys() {
  try {
    localStorage.removeItem('cpa_token')
    localStorage.removeItem('cpa_user')
    localStorage.removeItem('agw_token')
    localStorage.removeItem('agw_user')
  } catch {
    // private mode / storage blocked
  }
}

export const useAuthStore = create<AuthState>((set, get) => ({
  token: readCachedToken(),
  user: readCachedUser(),
  setAuth: (token, user) => {
    try {
      localStorage.setItem(TOKEN_KEY, token)
      localStorage.setItem(USER_KEY, JSON.stringify(user))
    } catch {
      // private mode / storage blocked
    }
    dropObsoleteAuthKeys()
    set({ token, user })
  },
  updateUser: (userUpdate) => {
    const current = get().user
    if (!current) return
    const updated = { ...current, ...userUpdate }
    if (
      updated.id === current.id &&
      updated.email === current.email &&
      updated.role === current.role &&
      updated.balance === current.balance
    ) {
      return
    }
    try {
      localStorage.setItem(USER_KEY, JSON.stringify(updated))
    } catch {
      // private mode / storage blocked
    }
    set({ user: updated })
  },
  logout: () => {
    try {
      localStorage.removeItem(TOKEN_KEY)
      localStorage.removeItem(USER_KEY)
    } catch {
      // private mode / storage blocked
    }
    dropObsoleteAuthKeys()
    set({ token: null, user: null })
  },
}))
