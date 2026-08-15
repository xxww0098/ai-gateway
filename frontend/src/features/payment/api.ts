// API functions for payment feature module
import { apiClient } from "@/shared/api/client"
import type {
  WechatCreateResponse,
  WechatStatusResponse,
  AlipayCreateResponse,
  AlipayStatusResponse,
  StripeConfig,
  StripeCreateResponse,
} from "./types"

// ── Wechat Pay ──────────────────────────────────────────────────────────────

export function createWechatOrder(amount: number) {
  return apiClient.post<WechatCreateResponse>('/payment/wechat/create', { amount })
}

export function getWechatOrderStatus(orderId: string) {
  return apiClient.get<WechatStatusResponse>(
    `/payment/wechat/status?order_id=${encodeURIComponent(orderId)}`
  )
}

// ── Alipay ──────────────────────────────────────────────────────────────────

export function createAlipayOrder(amount: number) {
  return apiClient.post<AlipayCreateResponse>('/payment/alipay/create', { amount })
}

export function getAlipayOrderStatus(orderId: string) {
  return apiClient.get<AlipayStatusResponse>(
    `/payment/alipay/status?order_id=${encodeURIComponent(orderId)}`
  )
}

// ── Stripe ──────────────────────────────────────────────────────────────────

export function getStripeConfig() {
  return apiClient.get<StripeConfig>('/payment/stripe/config')
}

export function createStripePayment(amount: number, currency = 'USD') {
  return apiClient.post<StripeCreateResponse>('/payment/stripe/create', { amount, currency })
}
