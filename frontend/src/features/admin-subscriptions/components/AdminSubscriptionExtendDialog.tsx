import { useState } from "react"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/components/ui/dialog"

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  extendId: number
  extendDays: string
  setExtendDays: (v: string) => void
  onExtend: (id: number, days: number) => Promise<void>
}

export function AdminSubscriptionExtendDialog({
  open,
  onOpenChange,
  extendId,
  extendDays,
  setExtendDays,
  onExtend,
}: Props) {
  const [loading, setLoading] = useState(false)

  const handleExtend = async () => {
    setLoading(true)
    await onExtend(extendId, parseInt(extendDays))
    setLoading(false)
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>续期订阅</DialogTitle>
          <DialogDescription>输入续期天数，将在当前到期日基础上延长。</DialogDescription>
        </DialogHeader>
        <div className="space-y-4 pt-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">续期天数</label>
            <Input type="number" min="1" value={extendDays}
              onChange={(e) => setExtendDays(e.target.value)} />
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="outline" onClick={() => onOpenChange(false)}>取消</Button>
            <Button onClick={handleExtend} disabled={loading} className="bg-blue-600 hover:bg-blue-700">
              {loading ? "续期中..." : "确认续期"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
