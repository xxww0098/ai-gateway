# 003 — Fire Alipay/Wechat success once, not from an unstable effect

- **Status**: DONE
- **Commit**: 34d21b3
- **Severity**: HIGH
- **Category**: Bugs & correctness
- **Rule**: react-doctor/no-prop-callback-in-effect
- **Estimated scope**: 2 files

## Problem

Paid/failed handling lives in `useEffect` and calls the `onSuccess` prop. `onSuccess` is in the dependency list, so a parent re-render with a new callback re-toasts and re-runs the success path while `status` is still `"paid"`.

    // src/features/payment/components/AlipayPayment.tsx:45 — current
    useEffect(() => {
      if (status === "paid") {
        setPolling(false)
        toast.success(`支付宝支付成功！充值 $${statusQuery.data!.amount.toFixed(2)}`)
        onSuccess?.()
      } else if (status === "failed") {
        setPolling(false)
        toast.error("支付宝支付失败")
      }
    }, [status, statusQuery.data, onSuccess])

    // src/features/payment/components/WechatPaySection.tsx:25 — current
    useEffect(() => {
      if (status === "paid") {
        setPolling(false)
        toast.success(`微信支付成功！已充值 $${statusQuery.data!.amount.toFixed(2)}`)
        onSuccess?.()
      } else if (status === "failed") {
        setPolling(false)
        toast.error("支付失败，请重试")
      }
    }, [status, statusQuery.data, onSuccess])

Canonical rule: do not notify the parent by calling a prop callback from an effect that also depends on local state. Keep one source of truth; fire the parent from a guarded transition.

This is the top-up path. Duplicate toasts are the visible bug; a parent `onSuccess` that refetches balance twice is acceptable, double-submitting a follow-up mutation would not be.

## Target

    // src/features/payment/components/AlipayPayment.tsx — target
    const onSuccessRef = useRef(onSuccess)
    onSuccessRef.current = onSuccess
    const handledOrderRef = useRef<string | null>(null)

    useEffect(() => {
      if (!activeOrderId) return
      if (status !== "paid" && status !== "failed") return
      if (handledOrderRef.current === activeOrderId) return
      handledOrderRef.current = activeOrderId
      setPolling(false)
      if (status === "paid") {
        const amount = statusQuery.data?.amount
        if (typeof amount === "number") {
          toast.success(`支付宝支付成功！充值 $${amount.toFixed(2)}`)
        }
        onSuccessRef.current?.()
      } else {
        toast.error("支付宝支付失败")
      }
    }, [status, activeOrderId, statusQuery.data?.amount])

Apply the same pattern in `WechatPaySection.tsx` (use `order?.order_id` as the handled key). Do not put `onSuccess` in the effect deps.

## Repo conventions to follow

- Hooks stay in `src/features/payment/hooks.ts` (`useAlipayOrderStatus` / `useWechatOrderStatus`).
- Toasts use `sonner` like the rest of payment.

## Steps

1. Add `useRef` to the Alipay import list. Guard the effect as in Target.
2. Repeat in `WechatPaySection.tsx`.
3. Do not change create-order mutations or poll interval.

## Boundaries

- Do NOT change payment API contracts or amounts.
- Do NOT add dependencies.
- STOP if these components no longer call `onSuccess` from an effect.

## Verification

- **Mechanical**: `npx react-doctor@latest --scope changed` clears `no-prop-callback-in-effect` and `no-adjust-state-on-prop-change` on these two files if they were only from this effect.
- **Behavior check**: On `/finance` (or the top-up tab), create a mock/paid Alipay order and confirm one success toast. Re-render the parent (switch theme or resize) and confirm no second toast. Same for Wechat.
- **Done when**: one toast per order id, diagnostic gone, poll still stops on paid/failed.
