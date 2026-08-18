import { useState } from 'react'
import { Link } from 'react-router-dom'
import { Check, Circle, X } from 'lucide-react'
import { adminRoutes } from '@/shared/routes/admin'

const DISMISS_KEY = 'agw_admin_setup_dismissed:v1'

export function AdminSetupChecklist({
  userCount,
  hasTraffic,
}: {
  userCount: number
  hasTraffic: boolean
}) {
  const [dismissed, setDismissed] = useState(() => localStorage.getItem(DISMISS_KEY) === '1')
  if (dismissed) return null

  const hasUsers = userCount > 1
  const steps = [
    { done: hasUsers, label: '创建第二个用户', to: adminRoutes.users },
    { done: hasTraffic, label: '接通上游并产生一次调用', to: adminRoutes.channels },
  ]

  return (
    <section className="rounded-2xl border border-primary-200 bg-primary-50/60 p-5 dark:border-primary-900/40 dark:bg-primary-950/20">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold text-foreground">开张清单</h3>
          <p className="mt-1 text-sm text-muted-foreground">
            现在只有管理员、也还没有转发。做完这两步，总览才会有真实数据。
          </p>
        </div>
        <button
          type="button"
          className="rounded-lg p-1.5 text-muted-foreground hover:bg-background hover:text-foreground"
          aria-label="不再显示开张清单"
          onClick={() => {
            localStorage.setItem(DISMISS_KEY, '1')
            setDismissed(true)
          }}
        >
          <X className="h-4 w-4" />
        </button>
      </div>
      <ol className="space-y-2">
        {steps.map((step) => (
          <li key={step.to}>
            <Link
              to={step.to}
              className="flex items-center gap-2 rounded-xl border border-border bg-card px-3 py-2 text-sm hover:border-primary/40"
            >
              {step.done ? (
                <Check className="h-4 w-4 text-emerald-600" />
              ) : (
                <Circle className="h-4 w-4 text-muted-foreground" />
              )}
              <span className={step.done ? 'text-muted-foreground line-through' : 'text-foreground'}>
                {step.label}
              </span>
            </Link>
          </li>
        ))}
      </ol>
    </section>
  )
}
