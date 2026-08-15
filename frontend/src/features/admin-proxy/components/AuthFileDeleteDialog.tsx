import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import type { AuthFileDeleteDialogProps } from '../types'

export function AuthFileDeleteDialog({
  open,
  onOpenChange,
  onConfirm,
  deleting,
  item,
}: AuthFileDeleteDialogProps) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-[400px] translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white">删除凭证</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            确定要删除 <span className="font-medium text-gray-900 dark:text-white">{item?.label || item?.name}</span> 吗？此操作不可撤销。
          </DialogPrimitive.Description>

          <div className="flex justify-end gap-3 mt-6">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-ghost px-4 text-sm">取消</button>
            </DialogPrimitive.Close>
            <button className="btn btn-danger px-5 text-sm" onClick={onConfirm} disabled={deleting}>
              {deleting ? '删除中...' : '确认删除'}
            </button>
          </div>
          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-md p-1 opacity-70 hover:opacity-100 transition-opacity">
            <X className="h-4 w-4" />
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
