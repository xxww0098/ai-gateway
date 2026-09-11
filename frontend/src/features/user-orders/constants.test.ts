import { describe, it, expect } from "vitest"
import {
  getProviderInfo,
  getStatusInfo,
  fmtAmount,
  fmtLocalAmount,
  fmtDateTime,
} from "./constants"

describe("user-orders constants & formatters", () => {
  it("resolves provider info correctly", () => {
    expect(getProviderInfo("alipay").label).toBe("支付宝")
    expect(getProviderInfo("wechat").label).toBe("微信支付")
    expect(getProviderInfo("stripe").label).toBe("Stripe")
    expect(getProviderInfo("unknown").label).toBe("unknown")
  })

  it("resolves status info correctly", () => {
    expect(getStatusInfo("paid").label).toBe("已支付")
    expect(getStatusInfo("pending").label).toBe("待支付")
    expect(getStatusInfo("failed").label).toBe("失败")
    expect(getStatusInfo("refunded").label).toBe("已退款")
    expect(getStatusInfo("unknown").label).toBe("unknown")
  })

  it("formats amounts properly", () => {
    expect(fmtAmount(50)).toBe("$50.00")
    expect(fmtAmount(0)).toBe("$0.00")
    expect(fmtLocalAmount(360, "CNY")).toBe("360.00 CNY")
  })

  it("formats date-times with pad", () => {
    const formatted = fmtDateTime("2026-05-09T08:05:00Z")
    expect(formatted).toMatch(/^2026-\d{2}-\d{2} \d{2}:\d{2}$/)
  })
})
