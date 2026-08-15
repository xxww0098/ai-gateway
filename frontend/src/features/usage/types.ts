// Types for admin usage logs feature

import type { UsageLog } from "@/shared/types/api"

/** Extended usage log with optional user/key display fields from admin endpoint */
export interface AdminUsageLog extends UsageLog {
  user_email?: string
  api_key_name?: string
}

/** Filter parameters for admin usage logs query */
export interface AdminUsageLogsFilter {
  page: number
  pageSize: number
  model?: string
  status?: string // 'success' | 'failed' | ''
  startDate?: string
  endDate?: string
}

/** Date range preset for usage log filtering */
export type DateRangePreset = 'today' | '7d' | '30d'
