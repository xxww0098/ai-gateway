import { useState, useCallback, useMemo } from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import type { ApiKey } from "./types"
import { fetchUsageLogs, fetchUserApiKeys } from "./api"
import type { UsageLogsParams } from "./api"

// ── Helpers ─────────────────────────────────────────────────────────────────

function todayStr(): string {
  const d = new Date()
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

function daysAgo(n: number): string {
  const d = new Date(Date.now() - n * 86400000)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

// ── Types ───────────────────────────────────────────────────────────────────

type DateRange = 'today' | '7d' | '30d' | 'custom'

interface UseUsageLogsOptions {
  pageSize?: number
}

// ── useUsageLogs Hook ───────────────────────────────────────────────────────

export function useUsageLogs(opts: UseUsageLogsOptions = {}) {
  const { pageSize: defaultPageSize = 20 } = opts
  const qc = useQueryClient()

  // Filter state
  const [filterKeyId, setFilterKeyId] = useState<string>('')
  const [filterModel, setFilterModel] = useState('')
  const [dateRange, setDateRange] = useState<DateRange>('7d')
  const [startDate, setStartDate] = useState(daysAgo(6))
  const [endDate, setEndDate] = useState(todayStr())

  // Pagination state
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(defaultPageSize)

  // Compute effective date range
  const getEffectiveDates = useCallback(() => {
    if (dateRange === 'today') return { startDate: todayStr(), endDate: todayStr() }
    if (dateRange === '7d') return { startDate: daysAgo(6), endDate: todayStr() }
    if (dateRange === '30d') return { startDate: daysAgo(29), endDate: todayStr() }
    return { startDate, endDate }
  }, [dateRange, startDate, endDate])

  const effectiveDates = getEffectiveDates()

  const queryParams: UsageLogsParams = useMemo(() => ({
    page,
    pageSize,
    apiKeyId: filterKeyId || undefined,
    model: filterModel.trim() || undefined,
    startDate: effectiveDates.startDate,
    endDate: effectiveDates.endDate,
  }), [page, pageSize, filterKeyId, filterModel, effectiveDates.startDate, effectiveDates.endDate])

  const usageQuery = useQuery({
    queryKey: queryKeys.usage.logs(queryParams as unknown as Record<string, unknown>),
    queryFn: () => fetchUsageLogs(queryParams),
  })

  const logs = usageQuery.data?.items || []
  const total = usageQuery.data?.total || 0
  const stats = usageQuery.data?.stats || null
  const totalPages = Math.ceil(total / pageSize)
  const loading = usageQuery.isLoading

  const handleFilter = useCallback(() => {
    setPage(1)
  }, [])

  const handlePageChange = useCallback((newPage: number) => {
    setPage(newPage)
  }, [])

  const handlePageSizeChange = useCallback((newSize: number) => {
    setPageSize(newSize)
    setPage(1)
  }, [])

  const handleDateRangeChange = useCallback((range: DateRange) => {
    setDateRange(range)
    setPage(1)
  }, [])

  const refresh = useCallback(() => {
    qc.invalidateQueries({ queryKey: queryKeys.usage.all() })
  }, [qc])

  return {
    // Data
    logs,
    stats,
    total,
    loading,
    page,
    pageSize,
    totalPages,
    // Filters
    filterKeyId,
    setFilterKeyId,
    filterModel,
    setFilterModel,
    dateRange,
    handleDateRangeChange,
    startDate,
    setStartDate,
    endDate,
    setEndDate,
    // Actions
    handleFilter,
    handlePageChange,
    handlePageSizeChange,
    refresh,
    // For export: expose the query params builder
    getEffectiveDates,
  }
}

// ── useUserApiKeys Hook (for filter dropdown) ───────────────────────────────

export function useUserApiKeys() {
  const apiKeysQuery = useQuery({
    queryKey: queryKeys.apiKeys.list(),
    queryFn: fetchUserApiKeys,
    // 这个 key 与「API 密钥」页共用。命中缓存时本 hook 的 queryFn 不会跑，拿到的
    // 是对端写进缓存的原始形态（分页对象 `{ items }` 而非数组）。所以归一化放在
    // `select` 里 —— 它对缓存里的任意形态都会执行，保证消费方永远拿到数组，
    // 否则 UsageFilterBar 的 `apiKeys.map` 会在拿到对象时崩溃。
    select: (res): ApiKey[] => {
      if (Array.isArray(res)) return res
      if (res && typeof res === 'object' && 'items' in res) {
        return (res as { items: ApiKey[] }).items
      }
      return []
    },
  })

  return {
    apiKeys: apiKeysQuery.data ?? [],
    loading: apiKeysQuery.isLoading,
  }
}
