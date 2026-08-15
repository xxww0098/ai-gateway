// React-query hooks for admin usage logs feature
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import { errorMessage } from "@/shared/api/errors"
import { toast } from "sonner"
import { useCallback } from "react"
import { fetchAdminUsageLogs } from "./api"
import type { AdminUsageLogsFilter } from "./types"

/**
 * Hook for fetching admin usage logs with filter parameters.
 * Uses react-query for caching, loading states, and error handling.
 */
export function useAdminUsageLogs(filter: AdminUsageLogsFilter) {
  const queryParams: Record<string, unknown> = {
    page: filter.page,
    pageSize: filter.pageSize,
    model: filter.model || undefined,
    status: filter.status || undefined,
    startDate: filter.startDate || undefined,
    endDate: filter.endDate || undefined,
  }

  const query = useQuery({
    queryKey: queryKeys.usage.logs(queryParams),
    queryFn: () => fetchAdminUsageLogs(filter),
  })

  if (query.error) {
    toast.error(errorMessage(query.error, '加载使用日志失败'))
  }

  return {
    logs: query.data?.items ?? [],
    total: query.data?.total ?? 0,
    currentPage: query.data?.page ?? filter.page,
    loading: query.isLoading,
    error: query.error,
    refetch: query.refetch,
  }
}

/**
 * Hook for invalidating usage logs cache.
 * Useful for manual refresh actions.
 */
export function useInvalidateUsageLogs() {
  const qc = useQueryClient()
  return useCallback(() => {
    qc.invalidateQueries({ queryKey: queryKeys.usage.all() })
  }, [qc])
}
