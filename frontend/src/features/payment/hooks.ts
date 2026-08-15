import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import { errorMessage } from "@/shared/api/errors"
import { toast } from "sonner"
import {
  createWechatOrder,
  getWechatOrderStatus,
  createAlipayOrder,
  getAlipayOrderStatus,
  getStripeConfig,
  createStripePayment,
} from "./api"

// ── Wechat Pay ──────────────────────────────────────────────────────────────

export function useCreateWechatOrder() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (amount: number) => createWechatOrder(amount),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.payment.all() })
    },
    onError: (err) => toast.error(errorMessage(err, '创建微信支付订单失败')),
  })
}

export function useWechatOrderStatus(orderId: string | null, enabled = true) {
  return useQuery({
    queryKey: queryKeys.payment.wechatStatus(orderId ?? ''),
    queryFn: () => getWechatOrderStatus(orderId!),
    enabled: enabled && !!orderId,
    refetchInterval: (query) => {
      const status = query.state.data?.status
      if (status === 'paid' || status === 'failed') return false
      return 3000
    },
  })
}

// ── Alipay ──────────────────────────────────────────────────────────────────

export function useCreateAlipayOrder() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (amount: number) => createAlipayOrder(amount),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.payment.all() })
    },
    onError: (err) => toast.error(errorMessage(err, '创建支付宝订单失败')),
  })
}

export function useAlipayOrderStatus(orderId: string | null, enabled = true) {
  return useQuery({
    queryKey: queryKeys.payment.alipayStatus(orderId ?? ''),
    queryFn: () => getAlipayOrderStatus(orderId!),
    enabled: enabled && !!orderId,
    refetchInterval: (query) => {
      const status = query.state.data?.status
      if (status === 'paid' || status === 'failed') return false
      return 3000
    },
  })
}

// ── Stripe ──────────────────────────────────────────────────────────────────

export function useStripeConfig() {
  return useQuery({
    queryKey: queryKeys.payment.stripeConfig(),
    queryFn: getStripeConfig,
    staleTime: 1000 * 60 * 10, // 10 minutes — config rarely changes
  })
}

export function useCreateStripePayment() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ amount, currency }: { amount: number; currency?: string }) =>
      createStripePayment(amount, currency),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.payment.all() })
    },
    onError: (err) => toast.error(errorMessage(err, '创建 Stripe 支付失败')),
  })
}
