import * as DialogPrimitive from "@radix-ui/react-dialog"
import { Activity } from "lucide-react"
import { ProgressRing } from "@/shared/components/ui/ProgressRing"
import type { ApiKey } from "../types"

interface Props {
  key_: ApiKey
}

export function ApiKeyUsageDialog({ key_ }: Props) {
  return (
    <DialogPrimitive.Root>
      <DialogPrimitive.Trigger asChild>
        <button className="btn btn-secondary btn-sm px-3 py-1.5 text-xs h-auto shadow-none">
          <Activity className="h-3.5 w-3.5" />
          额度分析
        </button>
      </DialogPrimitive.Trigger>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-3xl translate-x-[-50%] translate-y-[-50%] border border-border bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl">
          <DialogPrimitive.Title className="text-lg font-bold">API Key 限额分配 - {key_.name}</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 mt-1 mb-6">
            这个面板展示了该 Key 各个滚动时间窗口的使用进度，如果即将触顶，网关将拦截其请求。
          </DialogPrimitive.Description>

          <div className="grid grid-cols-2 lg:grid-cols-4 gap-6 p-6 rounded-xl bg-gray-50/50 dark:bg-dark-800/30 border border-border">
            <div className="flex flex-col items-center">
              <ProgressRing
                percentage={key_.quota ? (key_.quota_used / key_.quota) * 100 : 0}
                value={`$${(key_.quota_used || 0).toFixed(2)}`}
                label={key_.quota ? `总额 / $${key_.quota}` : "总额度 (无限制)"}
                size={120} strokeWidth={8}
                gradientFrom="#10b981" gradientTo="#047857"
              />
            </div>
            {key_.rate_limit_30d > 0 && (
              <div className="flex flex-col items-center">
                <ProgressRing
                  percentage={(key_.usage_30d / key_.rate_limit_30d) * 100}
                  value={`$${(key_.usage_30d || 0).toFixed(2)}`}
                  label={`月限制 / $${key_.rate_limit_30d}`}
                  size={120} strokeWidth={8}
                  gradientFrom="#f59e0b" gradientTo="#d97706"
                />
              </div>
            )}
            {key_.rate_limit_1d > 0 && (
              <div className="flex flex-col items-center">
                <ProgressRing
                  percentage={(key_.usage_1d / key_.rate_limit_1d) * 100}
                  value={`$${(key_.usage_1d || 0).toFixed(2)}`}
                  label={`日限制 / $${key_.rate_limit_1d}`}
                  size={120} strokeWidth={8}
                  gradientFrom="#3b82f6" gradientTo="#2563eb"
                />
              </div>
            )}
            {key_.rate_limit_5h > 0 && (
              <div className="flex flex-col items-center">
                <ProgressRing
                  percentage={(key_.usage_5h / key_.rate_limit_5h) * 100}
                  value={`$${(key_.usage_5h || 0).toFixed(2)}`}
                  label={`5H限制 / $${key_.rate_limit_5h}`}
                  size={120} strokeWidth={8}
                  gradientFrom="#ec4899" gradientTo="#be185d"
                />
              </div>
            )}
          </div>
          <div className="mt-6 flex justify-end">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-ghost">关闭</button>
            </DialogPrimitive.Close>
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
