// Types for admin subscriptions management

export interface Subscription {
  id: number
  user_id: number
  group_id: number
  email?: string
  username?: string
  group_name?: string
  status: string
  starts_at: string
  expires_at: string
  daily_usage_usd: number
  weekly_usage_usd: number
  monthly_usage_usd: number
  daily_limit_usd?: number | null
  weekly_limit_usd?: number | null
  monthly_limit_usd?: number | null
  created_at: string
  /** 与后端 funding_source 一致 */
  funding_source?: string
  funding_reference?: string
  price_paid?: number
  notes?: string | null
}

export interface Group {
  id: number
  name: string
  subscription_type: string
  rate_multiplier: number
  daily_limit_usd?: number | null
  weekly_limit_usd?: number | null
  monthly_limit_usd?: number | null
  default_validity_days: number
  /** 余额自助开通价 (USD)，0 表示不允许自助开通 */
  subscription_price_usd?: number
}

export interface AssignForm {
  user_id: string
  group_id: string
  validity_days: string
  notes: string
  funding_source: string
  funding_reference: string
  /** 线下实收折合 USD，可选 */
  price_paid_usd: string
}

export interface GroupForm {
  name: string
  rate_multiplier: string
  daily_limit_usd: string
  weekly_limit_usd: string
  monthly_limit_usd: string
  default_validity_days: string
  /** 自助开通价格，留空或 0 表示不允许用户用余额自助购买 */
  subscription_price_usd: string
}
