import { useState, useEffect } from "react"
import * as DialogPrimitive from "@radix-ui/react-dialog"
import { Layers, X } from "lucide-react"
import { errorMessage } from "@/shared/api/errors"
import { toast } from "sonner"
import type { ApiKey, ModelInfo } from "../types"
import { fetchKeyModels } from "../api"

interface Props {
  key_: ApiKey
}

export function ModelListDialog({ key_ }: Props) {
  const [open, setOpen] = useState(false)
  const [models, setModels] = useState<ModelInfo[]>([])
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    if (!open) return
    setLoading(true)
    setModels([])
    fetchKeyModels(key_.id)
      .then(res => setModels(res.models || []))
      .catch((err: unknown) => toast.error(errorMessage(err, "无法加载可用模型")))
      .finally(() => setLoading(false))
  }, [open, key_.id])

  return (
    <DialogPrimitive.Root open={open} onOpenChange={setOpen}>
      <DialogPrimitive.Trigger asChild>
        <button className="btn btn-secondary btn-sm px-3 py-1.5 text-xs h-auto shadow-none text-primary-600 dark:text-primary-400 border-primary-200 dark:border-primary-800 bg-primary-50 dark:bg-primary-900/10 hover:bg-primary-100 dark:hover:bg-primary-900/30">
          <Layers className="h-3.5 w-3.5" />
          路由白名单
        </button>
      </DialogPrimitive.Trigger>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-4xl max-h-[85vh] flex flex-col translate-x-[-50%] translate-y-[-50%] border border-border bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl">
          <div className="mb-4">
            <DialogPrimitive.Title className="text-xl font-bold">已授权模型矩阵 - {key_.name}</DialogPrimitive.Title>
            <DialogPrimitive.Description className="text-sm text-gray-500 mt-1">
              该 API Key 可通行的模型清单。若无相关模型，将返回 404 Error。单价格为预先换算后计费。
            </DialogPrimitive.Description>
          </div>

          <div className="flex-1 overflow-auto rounded-xl border border-border bg-gray-50/30 dark:bg-dark-800/20">
            <table className="table">
              <thead className="sticky top-0 bg-gray-50 dark:bg-dark-800 shadow-sm">
                <tr>
                  <th>模型接口识别码</th>
                  <th className="text-right">输入倍率 / 1M tkns</th>
                  <th className="text-right">输出倍率 / 1M tkns</th>
                </tr>
              </thead>
              <tbody>
                {loading ? (
                  <tr>
                    <td colSpan={3} className="h-32 text-center text-gray-500">加载节点配置中...</td>
                  </tr>
                ) : models.length === 0 ? (
                  <tr>
                    <td colSpan={3} className="h-32 text-center text-gray-500">此分组下没有可用模型。</td>
                  </tr>
                ) : (
                  models.map((m, idx) => (
                    <tr key={idx}>
                      <td className="font-mono text-sm text-gray-900 dark:text-gray-100">{m.id}</td>
                      <td className="text-right text-gray-600 dark:text-gray-400 font-mono">
                        ${(m.input_price_per_1m || 0).toFixed(4)}
                      </td>
                      <td className="text-right text-gray-600 dark:text-gray-400 font-mono">
                        ${(m.output_price_per_1m || 0).toFixed(4)}
                      </td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>

          <div className="mt-6 flex justify-end">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-secondary px-6">关闭窗口</button>
            </DialogPrimitive.Close>
          </div>

          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-sm opacity-70 transition-opacity hover:opacity-100">
            <X className="h-4 w-4" />
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
