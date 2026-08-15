import { useState } from "react"
import { errorMessage } from "@/shared/api/errors"
import { toast } from "sonner"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/shared/components/ui/select"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/shared/components/ui/dialog"
import { Plus } from "lucide-react"
import { Label } from "@/shared/components/ui/label"
import { SUBSCRIPTION_FUNDING_SOURCE_OPTIONS } from "../constants"
import { assignSubscription } from "../api"
import type { Group, AssignForm } from "../types"

interface Props {
  groups: Group[]
  onAssigned: () => void
}

export function AdminSubscriptionAssignDialog({ groups, onAssigned }: Props) {
  const [open, setOpen] = useState(false)
  const [form, setForm] = useState<AssignForm>({
    user_id: "",
    group_id: "",
    validity_days: "",
    notes: "",
    funding_source: "",
    funding_reference: "",
    price_paid_usd: "",
  })
  const [creating, setCreating] = useState(false)

  const handleAssign = async (e: { preventDefault: () => void }) => {
    e.preventDefault()
    setCreating(true)
    try {
      if (!form.funding_source.trim()) {
        toast.error("请选择资金来源")
        setCreating(false)
        return
      }
      let pricePaidUsd: number | undefined
      if (form.price_paid_usd.trim()) {
        const v = parseFloat(form.price_paid_usd)
        if (!Number.isFinite(v) || v < 0) {
          toast.error("实收金额 USD 格式不正确")
          setCreating(false)
          return
        }
        pricePaidUsd = v
      }

      const body: Record<string, unknown> = {
        user_id: parseInt(form.user_id),
        group_id: parseInt(form.group_id),
        funding_source: form.funding_source.trim(),
      }
      if (form.validity_days) body.validity_days = parseInt(form.validity_days)
      if (form.notes.trim()) body.notes = form.notes.trim()
      if (form.funding_reference.trim()) body.funding_reference = form.funding_reference.trim()
      if (pricePaidUsd !== undefined) body.price_paid_usd = pricePaidUsd

      await assignSubscription(body)
      toast.success("订阅分配成功")
      setOpen(false)
      setForm({
        user_id: "",
        group_id: "",
        validity_days: "",
        notes: "",
        funding_source: "",
        funding_reference: "",
        price_paid_usd: "",
      })
      onAssigned()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "分配失败"))
    } finally {
      setCreating(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button className="gap-2 bg-amber-600 hover:bg-amber-700 text-white shadow-sm">
          <Plus className="h-4 w-4" />
          分配订阅
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>分配订阅</DialogTitle>
          <DialogDescription>
            为用户开通订阅套餐，在有效期内使用量不扣余额。资金来源与实收金额会写入数据库，便于对账与站点营收统计，请如实填写。
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleAssign} className="space-y-4 pt-4">
          <div className="space-y-2">
            <Label className="text-sm font-medium">用户 ID</Label>
            <Input type="number" min="1" value={form.user_id}
              onChange={(e) => setForm({ ...form, user_id: e.target.value })} required />
          </div>
          <div className="space-y-2">
            <Label className="text-sm font-medium">订阅分组</Label>
            <Select value={form.group_id} onValueChange={(v) => setForm({ ...form, group_id: v })}>
              <SelectTrigger><SelectValue placeholder="选择订阅分组" /></SelectTrigger>
              <SelectContent>
                {groups.map(g => (
                  <SelectItem key={g.id} value={String(g.id)}>
                    {g.name} — 月限 ${g.monthly_limit_usd?.toFixed(2) || '∞'}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label className="text-sm font-medium">资金来源（必填）</Label>
            <Select
              value={form.funding_source || undefined}
              onValueChange={(v) => setForm({ ...form, funding_source: v })}
            >
              <SelectTrigger>
                <SelectValue placeholder="选择资金来源" />
              </SelectTrigger>
              <SelectContent>
                {SUBSCRIPTION_FUNDING_SOURCE_OPTIONS.map((o) => (
                  <SelectItem key={o.value} value={o.value}>
                    {o.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label className="text-sm font-medium">对账单号 / 参考信息（选填）</Label>
            <Input
              value={form.funding_reference}
              onChange={(e) => setForm({ ...form, funding_reference: e.target.value })}
              placeholder="如微信转账单号、付款人姓名后四位等"
            />
          </div>
          <div className="space-y-2">
            <Label className="text-sm font-medium">实收金额（USD，选填）</Label>
            <Input
              type="text"
              inputMode="decimal"
              value={form.price_paid_usd}
              onChange={(e) => setForm({ ...form, price_paid_usd: e.target.value })}
              placeholder="线下收款折合美元，不填则为 0"
            />
            <p className="text-xs text-muted-foreground">用于营收统计；与用户余额扣款无关。</p>
          </div>
          <div className="space-y-2">
            <Label className="text-sm font-medium">有效天数（留空使用分组默认值）</Label>
            <Input type="number" min="1" value={form.validity_days}
              onChange={(e) => setForm({ ...form, validity_days: e.target.value })}
              placeholder="30" />
          </div>
          <div className="space-y-2">
            <Label className="text-sm font-medium">备注</Label>
            <Input value={form.notes} onChange={(e) => setForm({ ...form, notes: e.target.value })}
              placeholder="可选" />
          </div>
          <div className="flex justify-end gap-2 pt-4">
            <Button type="button" variant="outline" onClick={() => setOpen(false)}>取消</Button>
            <Button type="submit" disabled={creating || !form.group_id || !form.funding_source}
              className="bg-amber-600 hover:bg-amber-700">
              {creating ? "分配中..." : "确认分配"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
