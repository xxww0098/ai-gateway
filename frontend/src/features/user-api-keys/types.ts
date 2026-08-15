// Types for user API keys

export interface ApiKey {
  id: number
  key: string
  name: string
  status: string
  display_status: string
  group_id?: number | null
  group_name?: string
  expires_at?: string | null
  last_used_at?: string | null
  quota: number
  quota_used: number
  rate_limit_5h: number
  rate_limit_1d: number
  rate_limit_7d: number
  rate_limit_30d: number
  usage_5h: number
  usage_1d: number
  usage_7d: number
  usage_30d: number
  window_5h_start?: string | null
  window_1d_start?: string | null
  window_7d_start?: string | null
  window_30d_start?: string | null
  rate_limit_5h_reset_at?: string | null
  rate_limit_1d_reset_at?: string | null
  rate_limit_7d_reset_at?: string | null
  rate_limit_30d_reset_at?: string | null
  created_at: string
}

export interface ModelInfo {
  id: string
  input_price_per_1m?: number
  output_price_per_1m?: number
}

export interface CreateKeyForm {
  name: string
  quota: string
  rate_5h: string
  rate_1d: string
  rate_7d: string
  rate_30d: string
  group_id?: number
  expires_in_days?: number | null
}

/** Payload sent to POST /user/api-keys */
export interface CreateKeyPayload {
  name: string
  quota?: number
  rate_limit_5h?: number
  rate_limit_1d?: number
  rate_limit_7d?: number
  rate_limit_30d?: number
  group_id?: number
  expires_in_days?: number | null
}

export interface AvailableGroup {
  id: number
  name: string
  description: string
  subscription_type: 'standard' | 'subscription'
  rate_multiplier: number
  daily_limit_usd: number
  weekly_limit_usd: number
  monthly_limit_usd: number
  default_validity_days: number
}
