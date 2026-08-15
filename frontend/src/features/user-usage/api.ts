// API functions for user usage
import { apiClient } from "@/shared/api/client"
import type { UsageLog, UsageStats, ApiKey } from "./types"

// ── Types ───────────────────────────────────────────────────────────────────

export interface UsageDetailResponse {
  items: UsageLog[]
  total: number
  page: number
  page_size: number
  stats: UsageStats | null
}

export interface UsageLogsParams {
  page: number
  pageSize: number
  apiKeyId?: string
  model?: string
  startDate?: string
  endDate?: string
}

// ── API Functions ───────────────────────────────────────────────────────────

export function fetchUsageLogs(params: UsageLogsParams) {
  const qs = new URLSearchParams()
  qs.set('page', String(params.page))
  qs.set('page_size', String(params.pageSize))
  if (params.apiKeyId) qs.set('api_key_id', params.apiKeyId)
  if (params.model?.trim()) qs.set('model', params.model.trim())
  if (params.startDate) qs.set('start_date', params.startDate)
  if (params.endDate) qs.set('end_date', params.endDate)
  return apiClient.get<UsageDetailResponse>(`/user/usage/detail?${qs.toString()}`)
}

export function fetchUserApiKeys() {
  return apiClient.get<ApiKey[] | { items: ApiKey[] }>('/user/api-keys')
}
