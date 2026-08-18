import { useState } from 'react'
import { Link } from 'react-router-dom'
import { CheckCircle2, Circle, Rocket, X } from 'lucide-react'
import { Button } from '@/shared/components/ui/button'
import { cn } from '@/shared/utils/utils'
import { adminRoutes } from '@/shared/routes/admin'

interface Props {
  /** 全站用户数（含管理员自己）。>1 视为已有普通用户注册。 */
  userCount: number
  /** 统计窗口内是否有过任何转发。 */
  hasTraffic: boolean
}

/** 关掉后不再出现（换机器/清缓存会重置，够用）。新键，不复用旧 localStorage 命名。 */
const DISMISS_KEY = 'agw-admin-setup-dismissed'

/**
 * 新装网关的初始化清单：只有「除管理员外没人、窗口内也没有转发」时才由 Dashboard 挂出。
 * 三步走通就会因为条件不再成立而自然消失；也可手动永久关掉。
 */
export function AdminSetupChecklist({ userCount, hasTraffic }: Props) {
  const [dismissed, setDismissed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(DISMISS_KEY) === '1'
    } catch {
      return false
    }
  })

  if (dismissed) return null

  const steps = [
    {
      done: true,
      label: '创建管理员账户',
      hint: '你已登录，网关控制台已就绪。',
    },
    {
      done: userCount > 1,
      label: '等待或邀请首个用户注册',
      hint: '普通用户注册后即可创建 agw- 密钥调用网关。',
    },
    {
      done: hasTraffic,
      label: '完成首次 API 转发',
      hint: '配置上游渠道后，用任意 agw- 密钥发起一次 /v1 调用。',
    },
  ]
  const doneCount = steps.filter(step => step.done).length

  const dismiss = () => {
    try {
      localStorage.setItem(DISMISS_KEY, '1')
    } catch {
      /* localStorage 不可用时只关当前会话即可 */
    }
    setDismissed(true)
  }

  return (
    <section className="relative rounded-2xl border border-primary/20 bg-primary/5 p-6 dark:border-primary/30">
      <button
        type="button"
        onClick={dismiss}
        aria-label="不再显示"
        className="absolute right-4 top-4 text-muted-foreground transition-colors hover:text-foreground"
      >
        <X className="h-4 w-4" />
      </button>

      <div className="flex items-start gap-3">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
          <Rocket className="h-5 w-5" />
        </div>
        <div className="space-y-1">
          <h3 className="text-base font-semibold text-foreground">网关初始化清单</h3>
          <p className="text-sm text-muted-foreground">
            这台网关看起来刚部署（{doneCount}/{steps.length} 完成）。按下面几步跑通第一次转发。
          </p>
        </div>
      </div>

      <ul className="mt-4 space-y-3">
        {steps.map(step => (
          <li key={step.label} className="flex items-start gap-3">
            {step.done ? (
              <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-primary" />
            ) : (
              <Circle className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground" />
            )}
            <div className="space-y-0.5">
              <p
                className={cn(
                  'text-sm font-medium',
                  step.done ? 'text-muted-foreground line-through' : 'text-foreground',
                )}
              >
                {step.label}
              </p>
              <p className="text-xs text-muted-foreground">{step.hint}</p>
            </div>
          </li>
        ))}
      </ul>

      <div className="mt-4">
        <Button asChild size="sm">
          <Link to={adminRoutes.channels}>前往配置上游渠道</Link>
        </Button>
      </div>
    </section>
  )
}
