import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { useUserApiKeys } from '../hooks'
import type { UserItem } from '../types'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: UserItem | null
}

function fmtDate(s: string): string {
  return new Date(s).toLocaleString('zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

export function AdminUserApiKeysDialog({ open, onOpenChange, user }: Props) {
  const { data: keys = [], isLoading: loading } = useUserApiKeys(user?.id ?? null, open && user !== null)

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-3xl translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white">API Keys</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            {user?.email} 名下的所有调用凭证
          </DialogPrimitive.Description>

          <div className="py-4">
            {loading ? (
              <div className="space-y-3">
                {[1, 2, 3].map(i => (
                  <div key={i} className="h-12 w-full rounded-lg bg-gray-100 dark:bg-dark-800 animate-pulse" />
                ))}
              </div>
            ) : keys.length === 0 ? (
              <div className="rounded-xl border border-dashed border-gray-200 dark:border-dark-600 px-6 py-10 text-center text-sm text-gray-500 dark:text-dark-400">
                该用户尚未创建任何 API Key
              </div>
            ) : (
              <div className="overflow-x-auto rounded-xl border border-gray-200 dark:border-dark-600">
                <table className="table">
                  <thead>
                    <tr>
                      <th>名称</th>
                      <th>Key 前缀</th>
                      <th className="text-right">额度使用</th>
                      <th>状态</th>
                      <th className="text-right">创建时间</th>
                    </tr>
                  </thead>
                  <tbody>
                    {keys.map(k => (
                      <tr key={k.id}>
                        <td className="font-medium text-gray-900 dark:text-white">{k.name}</td>
                        <td><code className="rounded bg-gray-100 dark:bg-dark-800 px-2 py-1 text-xs font-mono">{k.prefix}</code></td>
                        <td className="text-right font-mono text-[13px] tabular-nums text-gray-600 dark:text-gray-400">
                          ${k.quota_used.toFixed(2)}{k.quota > 0 ? ` / $${k.quota.toFixed(2)}` : ' / ∞'}
                        </td>
                        <td>
                          <span className={`inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-medium ${
                            k.status === 'active'
                              ? 'bg-green-50 text-green-700 dark:bg-green-900/20 dark:text-green-400'
                              : 'bg-gray-100 text-gray-600 dark:bg-dark-700 dark:text-gray-400'
                          }`}>
                            {k.status === 'active' ? '正常' : '禁用'}
                          </span>
                        </td>
                        <td className="text-right text-[13px] text-gray-500 tabular-nums">{fmtDate(k.created_at)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>

          <div className="flex justify-end">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-secondary px-5 text-sm">关闭</button>
            </DialogPrimitive.Close>
          </div>
          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-md p-1 opacity-70 hover:opacity-100 transition-opacity">
            <X className="h-4 w-4" />
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
