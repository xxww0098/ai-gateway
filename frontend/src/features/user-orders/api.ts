// API functions for user orders feature
import { apiClient } from "@/shared/api/client"
import type { PaginatedResponse } from "@/shared/types/api"
import type { Subscription, RefundRecord, PaymentOrder } from "./types"

// ── Subscriptions ───────────────────────────────────────────────────────────

export function fetchUserSubscriptions() {
  return apiClient.get<Subscription[]>('/user/subscriptions')
}

// ── Refunds ─────────────────────────────────────────────────────────────────

export function fetchRefundList() {
  return apiClient.get<{ items: RefundRecord[] }>('/refund/list')
}

// ── Payment Orders ──────────────────────────────────────────────────────────

export interface FetchOrdersParams {
  page: number
  pageSize: number
  status?: string
}

export function fetchPaymentOrders({ page, pageSize, status }: FetchOrdersParams) {
  const params = new URLSearchParams()
  params.set('page', String(page))
  params.set('page_size', String(pageSize))
  if (status) params.set('status', status)
  return apiClient.get<PaginatedResponse<PaymentOrder>>(`/user/orders?${params}`)
}
