/**
 * Auth feature module react-query hooks.
 * Provides useProfile, useLogin, and useRegister for components.
 */

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/shared/api/query-keys'
import { errorMessage } from '@/shared/api/errors'
import { toast } from 'sonner'
import { useAuthStore } from './auth_store'
import * as authApi from './api'
import type { LoginRequest, RegisterRequest, ProfileResponse } from './types'

/**
 * Fetches the current user's profile.
 * Only enabled when a token is present in the auth store.
 *
 * Profile drives the realtime balance displayed in the layout Header, so we
 * override the global default (5min staleTime / no focus refetch) with a
 * tighter cadence: 30s stale window, refetch on window focus, and refetch on
 * remount so that switching routes always picks up the latest balance.
 */
export function useProfile() {
  const token = useAuthStore((s) => s.token)
  return useQuery<ProfileResponse>({
    queryKey: queryKeys.auth.profile(),
    queryFn: authApi.getProfile,
    enabled: !!token,
    staleTime: 30 * 1000,
    refetchOnWindowFocus: true,
    refetchOnMount: true,
  })
}

/**
 * Login mutation. On success, stores token + user in auth store.
 * Returns the mutation object for use in form components.
 */
export function useLogin() {
  const setAuth = useAuthStore((s) => s.setAuth)
  const qc = useQueryClient()

  return useMutation({
    mutationFn: (data: LoginRequest) => authApi.login(data),
    onSuccess: (res) => {
      setAuth(res.token, res.user)
      qc.invalidateQueries({ queryKey: queryKeys.auth.profile() })
    },
    onError: (err) => {
      toast.error(errorMessage(err, '登录失败'))
    },
  })
}

/**
 * Register mutation. On success, shows a toast notification.
 * Does NOT auto-login — the component should navigate to /login.
 */
export function useRegister() {
  const setAuth = useAuthStore((s) => s.setAuth)
  const qc = useQueryClient()

  return useMutation({
    mutationFn: (data: RegisterRequest) => authApi.register(data),
    // Registration returns a token + user, so log the user straight in (mirrors
    // useLogin) rather than forcing a redundant trip through the login screen.
    onSuccess: (res) => {
      setAuth(res.token, res.user)
      qc.invalidateQueries({ queryKey: queryKeys.auth.profile() })
      toast.success('注册成功')
    },
    onError: (err) => {
      toast.error(errorMessage(err, '注册失败'))
    },
  })
}
