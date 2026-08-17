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

const USER_KEY = 'agw_user:v1'

function readCachedUser(): User | null {
  const raw = localStorage.getItem(USER_KEY)
  if (!raw) return null
  try {
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
    localStorage.removeItem(USER_KEY)
    return null
  }
}

export const useAuthStore = create<AuthState>((set, get) => ({
  token: null,
  user: readCachedUser(),
  setAuth: (token, user) => {
    localStorage.setItem(USER_KEY, JSON.stringify(user))
    localStorage.removeItem('cpa_token')
    localStorage.removeItem('cpa_user')
    set({ token, user })
  },
  updateUser: (userUpdate) => {
    const current = get().user
    if (!current) return
    const updated = { ...current, ...userUpdate }
    localStorage.setItem(USER_KEY, JSON.stringify(updated))
    set({ user: updated })
  },
  logout: () => {
    localStorage.removeItem(USER_KEY)
    localStorage.removeItem('cpa_token')
    localStorage.removeItem('cpa_user')
    set({ token: null, user: null })
  },
}))
