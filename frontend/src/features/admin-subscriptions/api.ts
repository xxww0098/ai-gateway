// API functions for admin subscriptions management
import { apiClient } from "@/shared/api/client"
import type { PaginatedResponse } from "@/shared/types/api"
import type { Subscription, Group } from "./types"

// ── Subscriptions ───────────────────────────────────────────────────────────

export function fetchSubscriptions(page: number, pageSize: number) {
  return apiClient.get<PaginatedResponse<Subscription>>(
    `/admin/subscriptions?page=${page}&page_size=${pageSize}`
  )
}

export function extendSubscription(id: number, days: number) {
  return apiClient.put(`/admin/subscriptions/${id}/extend`, { days })
}

export function revokeSubscription(id: number) {
  return apiClient.delete(`/admin/subscriptions/${id}`)
}

export function reactivateSubscription(id: number) {
  return apiClient.post(`/admin/subscriptions/${id}/reactivate`)
}

export function reactivateSubscriptionFallback(id: number) {
  return apiClient.put(`/admin/subscriptions/${id}/reactivate`)
}

export function resetSubscriptionQuota(id: number) {
  return apiClient.post(`/admin/subscriptions/${id}/reset-quota`)
}

export function assignSubscription(body: Record<string, unknown>) {
  return apiClient.post(`/admin/subscriptions`, body)
}

// ── Groups ──────────────────────────────────────────────────────────────────

export function fetchGroups() {
  return apiClient.get<Group[]>(`/admin/groups`)
}

export function createGroup(body: Record<string, unknown>) {
  return apiClient.post(`/admin/groups`, body)
}

export function updateGroup(id: number, body: Record<string, unknown>) {
  return apiClient.put(`/admin/groups/${id}`, body)
}

export function deleteGroup(id: number) {
  return apiClient.delete(`/admin/groups/${id}`)
}
