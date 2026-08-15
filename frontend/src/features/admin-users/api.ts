import { apiClient } from '@/shared/api/client'
import type { PaginatedResponse } from '@/shared/types/api'
import type {
  UserItem,
  ManagedApiKey,
  BalanceHistoryEntry,
  CreateUserPayload,
  UpdateUserPayload,
  DepositPayload,
  LoadUsersParams,
} from './types'

export async function loadUsers(params: LoadUsersParams): Promise<PaginatedResponse<UserItem>> {
  const { page, pageSize = 15, search, role, status } = params
  const qs = new URLSearchParams()
  qs.set('page', String(page))
  qs.set('page_size', String(pageSize))
  if (search) qs.set('q', search)
  if (role) qs.set('role', role)
  if (status) qs.set('status', status)

  return apiClient.get<PaginatedResponse<UserItem>>(`/admin/users?${qs.toString()}`)
}

export async function createUser(payload: CreateUserPayload): Promise<void> {
  await apiClient.post('/admin/users', payload)
}

export async function updateUser(id: number, payload: UpdateUserPayload): Promise<void> {
  await apiClient.put(`/admin/users/${id}`, payload)
}

export async function deleteUser(id: number): Promise<void> {
  await apiClient.delete(`/admin/users/${id}`)
}

export async function depositUser(id: number, payload: DepositPayload): Promise<void> {
  await apiClient.post(`/admin/users/${id}/deposit`, payload)
}

export async function getUserApiKeys(id: number): Promise<ManagedApiKey[]> {
  return apiClient.get<ManagedApiKey[]>(`/admin/users/${id}/api-keys`)
}

export async function getUserBalanceHistory(id: number): Promise<BalanceHistoryEntry[]> {
  const res = await apiClient.get<{ entries: BalanceHistoryEntry[] }>(`/admin/users/${id}/balance-history`)
  return res?.entries || []
}
