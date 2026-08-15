/**
 * Shared type definitions for ai-gateway frontend.
 * These types align with the backend models and API response structures.
 */

// ── Generic Response Types ──────────────────────────────────────────────────

/** Standard paginated response from backend */
export interface PaginatedResponse<T> {
  items: T[]
  total: number
  page: number
  page_size: number
}

// ── Entity Types ────────────────────────────────────────────────────────────

/** Usage log from /usage endpoints (matches model.UsageLog in backend) */
export interface UsageLog {
  id: number
  user_id: number
  api_key_id: number
  group_id: number | null
  request_id: string
  model: string
  provider: string
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
  duration_ms: number
  failed: boolean
  created_at: string
}
