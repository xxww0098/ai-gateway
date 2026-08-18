import type { LucideIcon } from 'lucide-react'
import { Inbox } from 'lucide-react'
import { Link } from 'react-router-dom'
import { Button } from '@/shared/components/ui/button'
import { cn } from '@/shared/utils/utils'

/**
 * 空状态的语气。决定图标底色与整体色调：
 * - `default`  —— 中性「暂无数据」。
 * - `error`    —— 加载失败。
 * - `first-use` —— 功能还没被用过，鼓励用户迈出第一步。
 * - `no-results` —— 有数据但当前筛选没命中。
 */
export type EmptyStateTone = 'default' | 'error' | 'first-use' | 'no-results'
export type EmptyStateSize = 'default' | 'compact'

/** 空状态的下一步动作：要么跳路由（`to`），要么回调（`onClick`）。 */
export interface EmptyStateAction {
  label: string
  onClick?: () => void
  /** 目标路由；给了就渲染成 router `Link`，优先于 `onClick`。 */
  to?: string
}

export interface EmptyStateProps {
  /** lucide 图标组件；缺省用 `Inbox`。 */
  icon?: LucideIcon
  title: string
  description?: string
  tone?: EmptyStateTone
  size?: EmptyStateSize
  /** 包一层描边卡片。表格内嵌用法不需要，独立成块时才开。 */
  bordered?: boolean
  action?: EmptyStateAction
  className?: string
}

const TONE_ICON_CLASS: Record<EmptyStateTone, string> = {
  default: 'bg-muted text-muted-foreground',
  error: 'bg-red-50 text-red-500 dark:bg-red-950/40 dark:text-red-400',
  'first-use': 'bg-primary/10 text-primary',
  'no-results': 'bg-muted text-muted-foreground',
}

/**
 * 统一的空/错/首用状态：一个居中的图标徽标 + 标题 + 说明 + 可选的下一步动作。
 *
 * 设计意图（对应「收紧面板空状态」）：一行「暂无 XXX」是死胡同，这个组件强制把
 * 「现在该点哪」讲清楚，所以 `title` 必填、`action` 常配。
 */
export function EmptyState({
  icon: Icon = Inbox,
  title,
  description,
  tone = 'default',
  size = 'default',
  bordered = false,
  action,
  className,
}: EmptyStateProps) {
  const compact = size === 'compact'
  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center text-center',
        compact ? 'gap-2 px-4 py-8' : 'gap-3 px-6 py-16',
        bordered &&
          'rounded-2xl border border-border bg-card dark:border-dark-800 dark:bg-dark-900',
        className,
      )}
    >
      <div
        className={cn(
          'flex items-center justify-center rounded-full',
          compact ? 'h-10 w-10' : 'h-14 w-14',
          TONE_ICON_CLASS[tone],
        )}
      >
        <Icon className={compact ? 'h-5 w-5' : 'h-7 w-7'} aria-hidden="true" />
      </div>
      <div className={cn('space-y-1', compact ? 'max-w-xs' : 'max-w-md')}>
        <p className={cn('font-semibold text-foreground', compact ? 'text-sm' : 'text-base')}>
          {title}
        </p>
        {description && (
          <p className={cn('text-muted-foreground', compact ? 'text-xs' : 'text-sm')}>
            {description}
          </p>
        )}
      </div>
      {action && <EmptyStateActionButton action={action} compact={compact} />}
    </div>
  )
}

function EmptyStateActionButton({
  action,
  compact,
}: {
  action: EmptyStateAction
  compact: boolean
}) {
  const size = compact ? 'sm' : 'default'
  if (action.to) {
    return (
      <Button asChild size={size} variant="outline" className="mt-1">
        <Link to={action.to}>{action.label}</Link>
      </Button>
    )
  }
  return (
    <Button type="button" size={size} variant="outline" className="mt-1" onClick={action.onClick}>
      {action.label}
    </Button>
  )
}

export interface EmptyStateRowProps extends EmptyStateProps {
  /** 内嵌单元格横跨的列数。 */
  colSpan: number
}

/**
 * 表格里的空状态：把 [`EmptyState`] 塞进一整行 `<tr><td colSpan>`，默认 `compact`。
 * 直接放进 `<tbody>` 使用，替代手写「一个横跨整行的提示单元格」。
 */
export function EmptyStateRow({ colSpan, ...props }: EmptyStateRowProps) {
  return (
    <tr>
      <td colSpan={colSpan} className="p-0">
        <EmptyState size="compact" {...props} />
      </td>
    </tr>
  )
}
