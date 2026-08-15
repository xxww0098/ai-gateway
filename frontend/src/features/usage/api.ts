// API functions for admin usage logs feature
import { apiClient } from "@/shared/api/client"
import type { PaginatedResponse } from "@/shared/types/api"
import type { AdminUsageLog, AdminUsageLogsFilter } from "./types"

/**
 * Fetches paginated admin usage logs with filter parameters.
 */
export function fetchAdminUsageLogs(filter: AdminUsageLogsFilter) {
  const params = new URLSearchParams()
  params.set('page', String(filter.page))
  params.set('page_size', String(filter.pageSize))
  if (filter.model?.trim()) params.set('model', filter.model.trim())
  if (filter.status) params.set('status', filter.status)
  if (filter.startDate) params.set('start_date', filter.startDate)
  if (filter.endDate) params.set('end_date', filter.endDate)
  return apiClient.get<PaginatedResponse<AdminUsageLog>>(`/admin/usage-logs?${params}`)
}
