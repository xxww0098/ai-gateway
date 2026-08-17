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

export const useAuthStore = create<AuthState>((set, get) => ({
  token: localStorage.getItem('agw_token'),
  user: JSON.parse(localStorage.getItem('agw_user') || 'null'),
  setAuth: (token, user) => {
    localStorage.setItem('agw_token', token)
    localStorage.setItem('agw_user', JSON.stringify(user))
    set({ token, user })
  },
  updateUser: (userUpdate) => {
    const current = get().user
    if (current) {
      const updated = { ...current, ...userUpdate }
      localStorage.setItem('agw_user', JSON.stringify(updated))
      set({ user: updated })
    }
  },
  logout: () => {
    localStorage.removeItem('agw_token')
    localStorage.removeItem('agw_user')
    set({ token: null, user: null })
  },
}))