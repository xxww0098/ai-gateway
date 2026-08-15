import { useState, useEffect } from 'react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { useUpdateUser } from '../hooks'
import type { UserItem } from '../types'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: UserItem | null
}

export function AdminUserEditDialog({ open, onOpenChange, user }: Props) {
  const [role, setRole] = useState('user')
  const [balance, setBalance] = useState('')
  const [concurrency, setConcurrency] = useState('')
  const [username, setUsername] = useState('')
  const [status, setStatus] = useState('active')
  const [password, setPassword] = useState('')

  const updateUser = useUpdateUser()

  useEffect(() => {
    if (user && open) {
      setRole(user.role)
      setBalance(user.balance.toString())
      setConcurrency(user.concurrency.toString())
      setUsername(user.username || '')
      setStatus(user.status)
      setPassword('')
    }
  }, [user, open])

  const handleSave = async () => {
    if (!user) return
    const payload = {
      role,
      balance: parseFloat(balance),
      concurrency: parseInt(concurrency) || 5,
      status,
      username: username || null,
      password: password || undefined,
    }
    await updateUser.mutateAsync({ id: user.id, payload })
    onOpenChange(false)
  }

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-[420px] translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white">编辑用户</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            修改 {user?.email || ''} 的账户配置
          </DialogPrimitive.Description>

          {user && (
            <div className="space-y-4 py-5">
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <label className="input-label">角色</label>
                  <select className="input h-10" value={role} onChange={e => setRole(e.target.value)}>
                    <option value="user">User</option>
                    <option value="admin">Admin</option>
                  </select>
                </div>
                <div className="space-y-1.5">
                  <label className="input-label">余额 (USD)</label>
                  <input className="input h-10 font-mono" type="number" step="0.01" value={balance} onChange={e => setBalance(e.target.value)} />
                </div>
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <label className="input-label">并发上限</label>
                  <input className="input h-10 font-mono" type="number" min="1" max="100" value={concurrency} onChange={e => setConcurrency(e.target.value)} />
                </div>
                <div className="space-y-1.5">
                  <label className="input-label">用户名</label>
                  <input className="input h-10" placeholder="可选" value={username} onChange={e => setUsername(e.target.value)} />
                </div>
              </div>
              <div className="flex items-center justify-between rounded-xl border border-gray-200 dark:border-dark-600 px-3 py-3">
                <div className="space-y-0.5">
                  <label className="text-sm font-medium text-gray-900 dark:text-white">账户状态</label>
                  <p className="text-xs text-gray-500 dark:text-dark-400">禁用后该用户将无法登录或调用 API</p>
                </div>
                <button
                  onClick={() => setStatus(s => s === 'active' ? 'disabled' : 'active')}
                  className={`relative h-6 w-11 rounded-full transition-colors ${status === 'active' ? 'bg-primary-500' : 'bg-gray-300 dark:bg-dark-600'}`}
                >
                  <span className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${status === 'active' ? 'translate-x-5' : ''}`} />
                </button>
              </div>
              <div className="space-y-1.5">
                <label className="input-label">重置密码 <span className="text-gray-400">(留空不修改)</span></label>
                <input className="input h-10" type="password" placeholder="输入新密码" value={password} onChange={e => setPassword(e.target.value)} />
              </div>
            </div>
          )}

          <div className="flex justify-end gap-3">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-ghost px-4 text-sm">取消</button>
            </DialogPrimitive.Close>
            <button className="btn btn-primary px-5 text-sm" onClick={handleSave} disabled={updateUser.isPending}>
              {updateUser.isPending ? '保存中...' : '保存变更'}
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
