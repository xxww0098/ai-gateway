import * as React from "react"
import * as DialogPrimitive from "@radix-ui/react-dialog"
import { useCallback, useEffect, useState } from "react"
import { AlertTriangle, CheckCircle2, X } from "lucide-react"
import { cn } from "@/shared/utils/utils"
import { setConfirmModalHandler } from "./registry"
import type { ConfirmModalOptions } from "./types"

type Queued = {
  id: number
  options: ConfirmModalOptions
  resolve: (value: boolean) => void
}

let idSeq = 0

function defaultTitle() {
  return "确认操作"
}

/**
 * 提供全局 `confirmModal()` 的 React 根包装，放在 `BrowserRouter` 内（与 `App` 并列或包住 `Routes`）即可。
 */
export function ConfirmModalProvider({
  children,
}: {
  children: React.ReactNode
}) {
  const [queue, setQueue] = useState<Queued[]>([])

  const head = queue[0]

  const enqueue = useCallback((options: ConfirmModalOptions) => {
    return new Promise<boolean>((resolve) => {
      const id = ++idSeq
      setQueue((q) => [...q, { id, options, resolve }])
    })
  }, [])

  const settle = useCallback((result: boolean) => {
    setQueue((q) => {
      const [first, ...rest] = q
      if (first) first.resolve(result)
      return rest
    })
  }, [])

  useEffect(() => {
    setConfirmModalHandler(enqueue)
    return () => setConfirmModalHandler(null)
  }, [enqueue])

  const onOpenChange = (open: boolean) => {
    if (!open) settle(false)
  }

  const opts = head?.options
  const title = opts?.title ?? defaultTitle()
  const message = opts?.message ?? ""
  const confirmText = opts?.confirmText ?? "确定"
  const cancelText = opts?.cancelText ?? "取消"
  const variant = opts?.variant ?? "default"
  const Icon = variant === "default" ? CheckCircle2 : AlertTriangle
  const iconClassName =
    variant === "danger"
      ? "bg-red-50 text-red-600 ring-red-100 dark:bg-red-950/40 dark:text-red-300 dark:ring-red-900/70"
      : variant === "warning"
        ? "bg-amber-50 text-amber-600 ring-amber-100 dark:bg-amber-950/40 dark:text-amber-300 dark:ring-amber-900/70"
        : "bg-primary-50 text-primary-600 ring-primary-100 dark:bg-primary-950/40 dark:text-primary-300 dark:ring-primary-900/70"
  const confirmClassName =
    variant === "danger"
      ? "bg-red-600 text-white shadow-sm hover:bg-red-700 focus-visible:ring-red-500 dark:bg-red-500 dark:hover:bg-red-400"
      : variant === "warning"
        ? "bg-amber-600 text-white shadow-sm hover:bg-amber-700 focus-visible:ring-amber-500 dark:bg-amber-500 dark:text-amber-950 dark:hover:bg-amber-400"
        : "bg-primary text-primary-foreground shadow-sm hover:bg-primary/90 focus-visible:ring-ring"
  const dismiss =
    opts?.dismissViaOverlayOrEscape !== undefined
      ? opts.dismissViaOverlayOrEscape
      : true

  return (
    <>
      {children}
      <DialogPrimitive.Root
        open={queue.length > 0}
        onOpenChange={onOpenChange}
      >
        <DialogPrimitive.Portal>
          <DialogPrimitive.Overlay
            className={cn(
              "fixed inset-0 z-[200] bg-slate-950/45 backdrop-blur-[2px] data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0"
            )}
          />
          <DialogPrimitive.Content
            key={head?.id}
            onPointerDownOutside={(e) => {
              if (!dismiss) e.preventDefault()
            }}
            onEscapeKeyDown={(e) => {
              if (!dismiss) e.preventDefault()
            }}
            className={cn(
              /* 必须高于 Overlay：二者曾同为 z-200 时，动画/transform 下易出现内容被蒙层盖住 */
              "fixed left-1/2 top-1/2 z-[210] w-[min(calc(100vw-2rem),28rem)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border/70 bg-card p-0 text-card-foreground shadow-2xl duration-200",
              "data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95"
            )}
          >
            <div className="flex items-start gap-3 border-b border-border/70 px-5 py-4">
              <span className={cn("mt-0.5 inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg ring-1", iconClassName)}>
                <Icon className="h-5 w-5" aria-hidden />
              </span>
              <div className="min-w-0 flex-1">
                <DialogPrimitive.Title className="break-words text-base font-semibold leading-6 text-foreground">
                  {title}
                </DialogPrimitive.Title>
                <DialogPrimitive.Description className="mt-1.5 whitespace-pre-line break-words text-sm leading-6 text-muted-foreground">
                  {message}
                </DialogPrimitive.Description>
              </div>
              {dismiss && (
                <DialogPrimitive.Close asChild>
                  <button
                    type="button"
                    className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    aria-label="关闭"
                  >
                    <X className="h-4 w-4" aria-hidden />
                  </button>
                </DialogPrimitive.Close>
              )}
            </div>
            <div className="flex flex-wrap items-center justify-end gap-2 px-5 py-4">
              <button
                type="button"
                className={cn(
                  "inline-flex h-9 min-w-[4.5rem] items-center justify-center rounded-md border border-border bg-background px-4 text-sm font-medium text-foreground",
                  "transition-colors hover:bg-muted disabled:pointer-events-none disabled:opacity-50",
                  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                )}
                onClick={() => settle(false)}
              >
                {cancelText}
              </button>
              <button
                type="button"
                className={cn(
                  "inline-flex h-9 min-w-[4.5rem] items-center justify-center rounded-md px-4 text-sm font-medium transition-colors",
                  "focus-visible:outline-none focus-visible:ring-2",
                  confirmClassName
                )}
                onClick={() => settle(true)}
              >
                {confirmText}
              </button>
            </div>
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>
    </>
  )
}
