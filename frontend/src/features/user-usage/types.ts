// ── Usage types ───────────────────────────────────────────────────────────────

export interface UsageLog {
  id: number
  request_id: string
  model: string
  provider: string
  api_key_name: string
  api_key_id: number
  input_tokens: number
  output_tokens: number
  reasoning_tokens: number
  cached_tokens: number
  input_cost: number
  output_cost: number
  total_cost: number
  actual_cost: number
  rate_multiplier: number
  stream: boolean
  duration_ms: number | null
  failed: boolean
  created_at: string
}

export interface UsageStats {
  total_requests: number
  total_input_tokens: number
  total_output_tokens: number
  total_tokens: number
  total_cost: number
  total_actual_cost: number
  success_count: number
  fail_count: number
  avg_duration_ms: number
}

export interface ApiKey {
  id: number
  name: string
}

export interface TooltipData {
  log: UsageLog
  x: number
  y: number
}

// ── Component prop types ────────────────────────────────────────────────────

export interface UsageStatsCardsProps {
  stats: UsageStats | null
}

export interface UsageFilterBarProps {
  apiKeys: ApiKey[]
  filterKeyId: string
  onFilterKeyIdChange: (id: string) => void
  filterModel: string
  onFilterModelChange: (m: string) => void
  dateRange: 'today' | '7d' | '30d' | 'custom'
  onDateRangeChange: (r: 'today' | '7d' | '30d' | 'custom') => void
  startDate: string
  onStartDateChange: (d: string) => void
  endDate: string
  onEndDateChange: (d: string) => void
  onFilter: () => void
  onRefresh: () => void
  onExport: () => void
  loading: boolean
  exporting: boolean
  total: number
}

export interface UsageTableProps {
  logs: UsageLog[]
  loading: boolean
  total: number
  page: number
  pageSize: number
  totalPages: number
  onPageChange: (p: number) => void
  onPageSizeChange: (s: number) => void
  onCostTooltip: (data: TooltipData | null) => void
  onTokenTooltip: (data: TooltipData | null) => void
}

export interface UsageCostTooltipProps {
  data: TooltipData
  onClose: () => void
}

export interface UsageTokenTooltipProps {
  data: TooltipData
  onClose: () => void
}
