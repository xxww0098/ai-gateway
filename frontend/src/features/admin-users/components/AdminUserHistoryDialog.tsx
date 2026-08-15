import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { useUserBalanceHistory } from '../hooks'
import type { UserItem } from '../types'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: UserItem | null
}

function fmtDate(s: string): string {
  return new Date(s).toLocaleString('zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

const KIND_LABELS: Record<string, string> = {
  initial: '初始设置',
  deposit: '管理员充值',
  adjustment: '余额调整',
  redeem: '兑换码',
  usage: 'API 消费',
}

export function AdminUserHistoryDialog({ open, onOpenChange, user }: Props) {
  const { data: entries = [], isLoading: loading } = useUserBalanceHistory(user?.id ?? null, open && user !== null)

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-4xl max-h-[85vh] flex flex-col translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white">余额变动记录</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            {user?.email} 的全部余额变动历史
          </DialogPrimitive.Description>

          <div className="py-4 flex-1 overflow-auto">
            {loading ? (
              <div className="space-y-3">
                {[1, 2, 3, 4].map(i => (
                  <div key={i} className="h-12 w-full rounded-lg bg-gray-100 dark:bg-dark-800 animate-pulse" />
                ))}
              </div>
            ) : entries.length === 0 ? (
              <div className="rounded-xl border border-dashed border-gray-200 dark:border-dark-600 px-6 py-10 text-center text-sm text-gray-500 dark:text-dark-400">
                暂无余额变动记录
              </div>
            ) : (
              <div className="overflow-x-auto rounded-xl border border-gray-200 dark:border-dark-600">
                <table className="table">
                  <thead>
                    <tr>
                      <th>时间</th>
                      <th>类型</th>
                      <th className="text-right">变动金额</th>
                      <th className="text-right">变动前</th>
                      <th className="text-right">变动后</th>
                      <th>操作人</th>
                      <th>备注</th>
                    </tr>
                  </thead>
                  <tbody>
                    {entries.map(e => (
                      <tr key={e.id}>
                        <td className="text-[13px] text-gray-500 dark:text-gray-400 tabular-nums whitespace-nowrap">{fmtDate(e.created_at)}</td>
                        <td>
                          <span className="inline-flex rounded-md bg-gray-100 dark:bg-dark-700 px-2 py-0.5 text-[11px] font-medium text-gray-700 dark:text-gray-300">
                            {KIND_LABELS[e.kind] || e.kind}
                          </span>
                        </td>
                        <td className={`text-right font-mono font-medium tabular-nums text-[13px] ${e.amount >= 0 ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-600 dark:text-red-400'}`}>
                          {e.amount >= 0 ? '+' : ''}{e.amount.toFixed(4)}
                        </td>
                        <td className="text-right font-mono tabular-nums text-[13px] text-gray-500 dark:text-gray-400">${e.balance_before.toFixed(4)}</td>
                        <td className="text-right font-mono tabular-nums text-[13px] text-gray-500 dark:text-gray-400">${e.balance_after.toFixed(4)}</td>
                        <td className="text-[13px] text-gray-500 dark:text-gray-400">{e.operator_email || '—'}</td>
                        <td className="text-[13px] text-gray-500 dark:text-gray-400 max-w-[200px] truncate">{e.note || '—'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>

          <div className="flex justify-end pt-2">
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
