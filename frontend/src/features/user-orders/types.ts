// Types for user orders

export interface Subscription {
  id: number
  group_name?: string
  status: string
  starts_at: string
  expires_at: string
  price_paid: number
  daily_usage_usd: number
  weekly_usage_usd: number
  monthly_usage_usd: number
  daily_limit_usd?: number | null
  weekly_limit_usd?: number | null
  monthly_limit_usd?: number | null
}

type RefundStatus = 'pending' | 'approved' | 'rejected'

export interface RefundRecord {
  id: number
  subscription_id: number
  amount: number
  status: RefundStatus
  created_at: string
}

export interface PaymentOrder {
  id: number
  user_id: number
  provider: string
  amount_usd: number
  amount_local: number
  currency: string
  status: string
  transaction_id: string | null
  metadata: string | null
  paid_at: string | null
  created_at: string
  updated_at: string
}
