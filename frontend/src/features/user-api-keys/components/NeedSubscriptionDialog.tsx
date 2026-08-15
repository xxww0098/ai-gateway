import { useNavigate } from "react-router-dom"
import { Crown } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/components/ui/dialog"
import { Button } from "@/shared/components/ui/button"

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 触发校验的订阅分组名称（可选） */
  groupName?: string
}

export function NeedSubscriptionDialog({ open, onOpenChange, groupName }: Props) {
  const navigate = useNavigate()

  const goSubscriptions = () => {
    onOpenChange(false)
    navigate("/subscriptions")
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md border-amber-200/80 dark:border-amber-900/40 bg-gradient-to-b from-amber-50/40 to-background dark:from-amber-950/20">
        <DialogHeader className="space-y-3">
          <div className="mx-auto sm:mx-0 flex h-12 w-12 items-center justify-center rounded-full bg-amber-100 dark:bg-amber-900/40">
            <Crown className="h-6 w-6 text-amber-600 dark:text-amber-400" />
          </div>
          <DialogTitle className="text-center sm:text-left">开通订阅后即可使用该分组</DialogTitle>
          <DialogDescription className="text-center sm:text-left text-gray-600 dark:text-dark-300 space-y-2">
            {groupName ? (
              <p>
                分组「<span className="font-medium text-foreground">{groupName}</span>」为订阅类型，需要您在该分组下拥有
                <span className="font-medium">有效订阅</span>
                才能将 API Key 绑定到该分组。
              </p>
            ) : (
              <p>
                所选分组为订阅类型，需要您在该分组下拥有<span className="font-medium">有效订阅</span>
                后才能绑定 API Key。
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              若暂无订阅，请到「我的订阅」在可用套餐中使用账户余额自行开通。
            </p>
          </DialogDescription>
        </DialogHeader>
        <DialogFooter className="gap-2 sm:gap-0">
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            稍后
          </Button>
          <Button type="button" className="bg-amber-600 hover:bg-amber-700 text-white" onClick={goSubscriptions}>
            前往我的订阅
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
