import { useState } from 'react'
import { Link } from 'react-router-dom'
import { CheckCircle2, Circle, Compass, ArrowRight, X } from 'lucide-react'
import { Button } from '@/shared/components/ui/button'
import { adminRoutes } from '@/shared/routes/admin'
import { docsPath } from '@/shared/routes/docs'

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
      id: 1,
      done: true,
      title: '管理员账户就绪',
      desc: '当前已成功登录管理中台，网关核心控制平面与安全凭证加密已初始化。',
      statusText: '已就绪',
    },
    {
      id: 2,
      done: userCount > 1,
      title: '用户注册入驻',
      desc: '等待首位租户在门户注册；普通用户入驻后即可申领独立的 agw- 密钥。',
      statusText: userCount > 1 ? '已完成' : '等待租户',
    },
    {
      id: 3,
      done: hasTraffic,
      title: '跑通首次 API 转发',
      desc: '在渠道管理中接入供应商凭证并配置单价，使用 agw- 密钥完成第一笔调用。',
      statusText: hasTraffic ? '已完成' : '待配置',
    },
  ]
  const doneCount = steps.filter(step => step.done).length
  const progressPercent = Math.round((doneCount / steps.length) * 100)

  const dismiss = () => {
    try {
      localStorage.setItem(DISMISS_KEY, '1')
    } catch {
      /* localStorage 不可用时只关当前会话即可 */
    }
    setDismissed(true)
  }

  return (
    <section className="relative overflow-hidden rounded-[13px] border border-border bg-card p-5 shadow-[0_1px_2px_rgb(0_0_0/0.07)]">
      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div className="flex items-center gap-2.5">
          <div className="flex size-8 shrink-0 items-center justify-center rounded-[8px] border border-primary/20 bg-primary/10 text-primary">
            <Compass className="size-4" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-sm font-semibold tracking-tight text-foreground">网关初始化向导</h3>
              <span className="rounded-md border border-primary/20 bg-primary/10 px-1.5 py-0.5 text-[10px] font-bold text-primary">
                {doneCount} / {steps.length} 就绪 ({progressPercent}%)
              </span>
            </div>
            <p className="mt-0.5 text-xs text-muted-foreground">
              当前网关为初始部署状态。完成以下三步跑通首次转发，网关即可正式对外提供服务。
            </p>
          </div>
        </div>

        <button
          type="button"
          onClick={dismiss}
          aria-label="不再显示"
          className="rounded-[6px] p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          <X className="size-4" />
        </button>
      </div>

      {/* Progress Bar */}
      <div className="mt-3.5 h-1 w-full overflow-hidden rounded-full bg-muted">
        <div
          className="h-full bg-primary transition-all duration-300 ease-out"
          style={{ width: `${progressPercent}%` }}
        />
      </div>

      {/* 3 Step Cards */}
      <div className="my-3.5 grid gap-3 sm:grid-cols-3">
        {steps.map(step => (
          <div
            key={step.id}
            className={`rounded-[10px] border p-3.5 transition-colors ${
              step.done
                ? 'border-border/60 bg-muted/20'
                : 'border-border bg-card'
            }`}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground">
                步骤 0{step.id}
              </span>
              {step.done ? (
                <span className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-600 dark:text-emerald-400">
                  <CheckCircle2 className="size-3.5" />
                  {step.statusText}
                </span>
              ) : (
                <span className="inline-flex items-center gap-1 text-[11px] font-medium text-muted-foreground">
                  <Circle className="size-3" />
                  {step.statusText}
                </span>
              )}
            </div>
            <h4 className={`mt-2 text-xs font-semibold ${step.done ? 'text-foreground' : 'text-foreground'}`}>
              {step.title}
            </h4>
            <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
              {step.desc}
            </p>
          </div>
        ))}
      </div>

      {/* Action Strip */}
      <div className="flex flex-wrap items-center justify-between gap-3 pt-2 border-t border-border/60">
        <div className="text-xs text-muted-foreground">
          首笔转发成功后此引导将自动收起，亦可点击右上角永久关闭。
        </div>
        <div className="flex items-center gap-2">
          <Button asChild variant="outline" size="sm" className="h-8 text-xs rounded-[8px]">
            <Link to={docsPath('quickstart')}>
              查看接入文档
            </Link>
          </Button>
          <Button asChild size="sm" className="h-8 text-xs rounded-[8px] bg-primary text-primary-foreground hover:bg-primary/90">
            <Link to={adminRoutes.channels} className="inline-flex items-center gap-1">
              前往配置上游渠道
              <ArrowRight className="size-3.5" />
            </Link>
          </Button>
        </div>
      </div>
    </section>
  )
}
