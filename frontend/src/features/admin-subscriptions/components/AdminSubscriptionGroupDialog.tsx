import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/components/ui/dialog"
import { Settings2 } from "lucide-react"
import type { GroupForm } from "../types"

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  editingGroupId: number | null
  groupForm: GroupForm
  setGroupForm: (f: GroupForm) => void
  savingGroup: boolean
  onSave: (e: React.FormEvent) => Promise<void>
}

export function AdminSubscriptionGroupDialog({
  open,
  onOpenChange,
  editingGroupId,
  groupForm,
  setGroupForm,
  savingGroup,
  onSave,
}: Props) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Settings2 className="w-5 h-5 text-amber-500" />
            {editingGroupId ? "编辑订阅套餐" : "创建订阅套餐"}
          </DialogTitle>
          <DialogDescription>
            订阅套餐按5h/周/月限额计费，用户在有效期内使用不扣余额。留空的限额表示不限制。
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={onSave} className="space-y-4 pt-2">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">套餐名称</label>
              <Input
                value={groupForm.name}
                onChange={(e) => setGroupForm({ ...groupForm, name: e.target.value })}
                placeholder="例如: 月卡VIP"
                required
                disabled={!!editingGroupId}
              />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">计费倍率</label>
              <Input
                type="number" step="0.01" min="0.01"
                value={groupForm.rate_multiplier}
                onChange={(e) => setGroupForm({ ...groupForm, rate_multiplier: e.target.value })}
                placeholder="1.0"
              />
            </div>
          </div>

          <div className="space-y-3 bg-gray-50 dark:bg-dark-900 rounded-xl p-4">
            <h4 className="text-sm font-semibold text-gray-700 dark:text-gray-300 flex items-center gap-1.5">
              用量限额 <span className="text-xs font-normal text-gray-400">(USD，留空 = 不限制)</span>
            </h4>
            <div className="grid grid-cols-3 gap-3">
              <div className="space-y-1">
                <label className="text-xs text-gray-500">每5h额度</label>
                <Input
                  type="number" step="0.01" min="0"
                  value={groupForm.daily_limit_usd}
                  onChange={(e) => setGroupForm({ ...groupForm, daily_limit_usd: e.target.value })}
                  placeholder="∞"
                  className="text-sm"
                />
              </div>
              <div className="space-y-1">
                <label className="text-xs text-gray-500">每周限额</label>
                <Input
                  type="number" step="0.01" min="0"
                  value={groupForm.weekly_limit_usd}
                  onChange={(e) => setGroupForm({ ...groupForm, weekly_limit_usd: e.target.value })}
                  placeholder="∞"
                  className="text-sm"
                />
              </div>
              <div className="space-y-1">
                <label className="text-xs text-gray-500">每月限额</label>
                <Input
                  type="number" step="0.01" min="0"
                  value={groupForm.monthly_limit_usd}
                  onChange={(e) => setGroupForm({ ...groupForm, monthly_limit_usd: e.target.value })}
                  placeholder="∞"
                  className="text-sm"
                />
              </div>
            </div>
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium">默认有效天数</label>
            <Input
              type="number" min="1"
              value={groupForm.default_validity_days}
              onChange={(e) => setGroupForm({ ...groupForm, default_validity_days: e.target.value })}
              placeholder="30"
            />
            <p className="text-xs text-gray-400">开通或分配订阅时若不指定天数，将使用此默认值。</p>
          </div>

          <div className="space-y-2 rounded-xl border border-amber-200/60 dark:border-amber-900/40 bg-amber-50/40 dark:bg-amber-950/20 p-4">
            <label className="text-sm font-medium text-amber-900 dark:text-amber-200">余额自助开通价 (USD)</label>
            <Input
              type="number"
              step="0.01"
              min="0"
              value={groupForm.subscription_price_usd}
              onChange={(e) => setGroupForm({ ...groupForm, subscription_price_usd: e.target.value })}
              placeholder="0 表示仅管理员分配"
            />
            <p className="text-xs text-amber-800/80 dark:text-amber-300/80">
              大于 0 时，用户可在「我的订阅」用账户余额自助购买该套餐；为 0 或留空则关闭自助，仅管理员分配。
            </p>
          </div>

          <div className="flex justify-end gap-2 pt-4">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>取消</Button>
            <Button type="submit" disabled={savingGroup} className="bg-amber-600 hover:bg-amber-700 text-white">
              {savingGroup ? "保存中..." : editingGroupId ? "更新套餐" : "创建套餐"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
