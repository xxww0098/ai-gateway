import { useState } from 'react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { toast } from 'sonner'
import { useDepositUser } from '../hooks'
import type { DepositPayload, UserItem } from '../types'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: UserItem | null
}

export function AdminUserDepositDialog({ open, onOpenChange, user }: Props) {
  const [amount, setAmount] = useState('')
  const [note, setNote] = useState('')

  const deposit = useDepositUser()

  const preview = () => {
    const a = parseFloat(amount)
    return Number.isFinite(a) && a > 0 ? (user?.balance || 0) + a : (user?.balance || 0)
  }

  const handleDeposit = async () => {
    if (!user) return
    const a = parseFloat(amount)
    if (!Number.isFinite(a) || a <= 0) { toast.error('请输入正数金额'); return }

    const payload: DepositPayload = { amount: a }
    if (note.trim()) payload.note = note.trim()

    await deposit.mutateAsync({ id: user.id, payload })
    onOpenChange(false)
    setAmount(''); setNote('')
  }

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-[460px] translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white">充值余额</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            为 {user?.email} 添加 USD 余额
          </DialogPrimitive.Description>

          {user && (
            <div className="space-y-4 py-5">
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="rounded-xl border border-gray-200 dark:border-dark-600 bg-gray-50/80 dark:bg-dark-800/70 px-3 py-3">
                  <p className="text-xs text-gray-500 dark:text-dark-400">当前余额</p>
                  <p className="mt-1 text-lg font-semibold text-gray-900 dark:text-white tabular-nums">
                    ${user.balance.toFixed(4)}
                  </p>
                </div>
                <div className="rounded-xl border border-emerald-200 dark:border-emerald-800/60 bg-emerald-50/80 dark:bg-emerald-950/30 px-3 py-3">
                  <p className="text-xs text-emerald-700 dark:text-emerald-300">充值后余额</p>
                  <p className="mt-1 text-lg font-semibold text-emerald-700 dark:text-emerald-300 tabular-nums">
                    ${preview().toFixed(4)}
                  </p>
                </div>
              </div>
              <div className="space-y-1.5">
                <label className="input-label">充值金额 (USD)</label>
                <input className="input h-10 font-mono" type="number" step="0.01" min="0.01" placeholder="0.00" value={amount} onChange={e => setAmount(e.target.value)} />
              </div>
              <div className="space-y-1.5">
                <label className="input-label">备注 <span className="text-gray-400">(可选)</span></label>
                <input className="input h-10" placeholder="例如：首月赠送额度" value={note} onChange={e => setNote(e.target.value)} />
              </div>
            </div>
          )}

          <div className="flex justify-end gap-3">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-ghost px-4 text-sm">取消</button>
            </DialogPrimitive.Close>
            <button className="btn btn-success px-5 text-sm" onClick={handleDeposit} disabled={deposit.isPending}>
              {deposit.isPending ? '充值中...' : '确认充值'}
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
