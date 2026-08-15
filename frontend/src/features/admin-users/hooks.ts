import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { queryKeys } from '@/shared/api/query-keys'
import { errorMessage } from '@/shared/api/errors'
import {
  loadUsers,
  createUser,
  updateUser,
  deleteUser,
  depositUser,
  getUserApiKeys,
  getUserBalanceHistory,
} from './api'
import type {
  LoadUsersParams,
  CreateUserPayload,
  UpdateUserPayload,
  DepositPayload,
} from './types'

/**
 * Fetches a paginated list of users with optional search/filter params.
 */
export function useUsers(params: LoadUsersParams) {
  return useQuery({
    queryKey: queryKeys.users.list({ page: params.page, pageSize: params.pageSize ?? 15 }),
    queryFn: () => loadUsers(params),
  })
}

/**
 * Creates a new user. Invalidates the users list on success.
 */
export function useCreateUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (payload: CreateUserPayload) => createUser(payload),
    onSuccess: () => {
      toast.success('用户创建成功')
      qc.invalidateQueries({ queryKey: queryKeys.users.all() })
    },
    onError: (err) => toast.error(errorMessage(err, '创建失败')),
  })
}

/**
 * Updates an existing user. Invalidates the users list on success.
 */
export function useUpdateUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, payload }: { id: number; payload: UpdateUserPayload }) =>
      updateUser(id, payload),
    onSuccess: () => {
      toast.success('用户信息已更新')
      qc.invalidateQueries({ queryKey: queryKeys.users.all() })
    },
    onError: (err) => toast.error(errorMessage(err, '保存失败')),
  })
}

/**
 * Deletes a user. Invalidates the users list on success.
 */
export function useDeleteUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => deleteUser(id),
    onSuccess: () => {
      toast.success('用户已删除')
      qc.invalidateQueries({ queryKey: queryKeys.users.all() })
    },
    onError: (err) => toast.error(errorMessage(err, '删除失败')),
  })
}

/**
 * Deposits balance to a user. Invalidates the users list on success.
 */
export function useDepositUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, payload }: { id: number; payload: DepositPayload }) =>
      depositUser(id, payload),
    onSuccess: () => {
      toast.success('充值成功')
      qc.invalidateQueries({ queryKey: queryKeys.users.all() })
    },
    onError: (err) => toast.error(errorMessage(err, '充值失败')),
  })
}

/**
 * Fetches API keys for a specific user.
 */
export function useUserApiKeys(userId: number | null, enabled = true) {
  return useQuery({
    queryKey: queryKeys.users.detail(userId ?? 0),
    queryFn: () => getUserApiKeys(userId!),
    enabled: enabled && userId !== null,
  })
}

/**
 * Fetches balance history for a specific user.
 */
export function useUserBalanceHistory(userId: number | null, enabled = true) {
  return useQuery({
    queryKey: [...queryKeys.users.detail(userId ?? 0), 'balance-history'] as const,
    queryFn: () => getUserBalanceHistory(userId!),
    enabled: enabled && userId !== null,
  })
}
