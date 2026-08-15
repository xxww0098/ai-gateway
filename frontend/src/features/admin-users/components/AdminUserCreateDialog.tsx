import { useState } from 'react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { toast } from 'sonner'
import { useCreateUser } from '../hooks'
import type { CreateUserPayload } from '../types'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function AdminUserCreateDialog({ open, onOpenChange }: Props) {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [role, setRole] = useState('user')
  const [username, setUsername] = useState('')
  const [balance, setBalance] = useState('')

  const createUser = useCreateUser()

  const reset = () => {
    setEmail(''); setPassword(''); setConfirm('')
    setRole('user'); setUsername(''); setBalance('')
  }

  const handleCreate = async () => {
    if (!email.trim()) { toast.error('请输入邮箱'); return }
    if (password.length < 8) { toast.error('密码至少8位'); return }
    if (password !== confirm) { toast.error('两次密码不一致'); return }

    const payload: CreateUserPayload = {
      email: email.trim(),
      password,
      role,
    }
    if (username.trim()) payload.username = username.trim()
    if (balance) payload.balance = parseFloat(balance)

    await createUser.mutateAsync(payload)
    onOpenChange(false)
    reset()
  }

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-[460px] translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white">手动注册用户</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            由管理员直接创建一个新的平台账户。
          </DialogPrimitive.Description>

          <div className="space-y-4 py-5">
            <div className="space-y-1.5">
              <label className="input-label">邮箱 *</label>
              <input className="input h-10" type="email" placeholder="user@example.com" value={email} onChange={e => setEmail(e.target.value)} />
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-1.5">
                <label className="input-label">密码 *</label>
                <input className="input h-10" type="password" placeholder="至少8位" value={password} onChange={e => setPassword(e.target.value)} />
              </div>
              <div className="space-y-1.5">
                <label className="input-label">确认密码 *</label>
                <input className="input h-10" type="password" placeholder="再次输入" value={confirm} onChange={e => setConfirm(e.target.value)} />
              </div>
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-1.5">
                <label className="input-label">角色</label>
                <select className="input h-10" value={role} onChange={e => setRole(e.target.value)}>
                  <option value="user">User (开发者)</option>
                  <option value="admin">Admin (管理组)</option>
                </select>
              </div>
              <div className="space-y-1.5">
                <label className="input-label">初始余额 (USD)</label>
                <input className="input h-10 font-mono" type="number" placeholder="0.00" step="0.01" min="0" value={balance} onChange={e => setBalance(e.target.value)} />
              </div>
            </div>
            <div className="space-y-1.5">
              <label className="input-label">用户名 <span className="text-gray-400">(可选)</span></label>
              <input className="input h-10" placeholder="显示名称" value={username} onChange={e => setUsername(e.target.value)} />
            </div>
            <div className="rounded-xl border border-dashed border-gray-200 dark:border-dark-600 bg-gray-50/80 dark:bg-dark-800/70 px-3 py-3 text-xs leading-5 text-gray-500 dark:text-dark-400">
              Admin 角色拥有系统全部管理权限。新用户创建后可通过编辑修改其权限及额度。
            </div>
          </div>

          <div className="flex justify-end gap-3">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-ghost px-4 text-sm">取消</button>
            </DialogPrimitive.Close>
            <button className="btn btn-primary px-5 text-sm" onClick={handleCreate} disabled={createUser.isPending}>
              {createUser.isPending ? '创建中...' : '注册用户'}
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
