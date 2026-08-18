import type { LucideIcon } from 'lucide-react'
import { Link } from 'react-router-dom'
import { cn } from '@/shared/utils/utils'

export type EmptyStateAction = {
  label: string
  to?: string
  onClick?: () => void
}

export type EmptyStateProps = {
  title: string
  description?: string
  icon?: LucideIcon
  tone?: 'default' | 'error' | 'first-use' | 'no-results'
  size?: 'default' | 'compact'
  bordered?: boolean
  className?: string
  action?: EmptyStateAction
}

const toneClass = {
  default: 'text-muted-foreground',
  error: 'text-red-600 dark:text-red-400',
  'first-use': 'text-primary-600 dark:text-primary-400',
  'no-results': 'text-muted-foreground',
} as const

function Action({ action }: { action: EmptyStateAction }) {
  const className =
    'inline-flex min-h-9 items-center justify-center rounded-lg bg-primary px-3 text-sm font-medium text-primary-foreground hover:opacity-90'
  if (action.to) {
    return (
      <Link to={action.to} className={className}>
        {action.label}
      </Link>
    )
  }
  return (
    <button type="button" className={className} onClick={action.onClick}>
      {action.label}
    </button>
  )
}

export function EmptyState({
  title,
  description,
  icon: Icon,
  tone = 'default',
  size = 'default',
  bordered,
  className,
  action,
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center text-center',
        size === 'compact' ? 'gap-2 px-4 py-8' : 'gap-3 px-6 py-14',
        bordered && 'rounded-2xl border border-border bg-card',
        className,
      )}
    >
      {Icon ? <Icon className={cn('h-8 w-8', toneClass[tone])} /> : null}
      <h3 className="text-sm font-semibold text-foreground">{title}</h3>
      {description ? <p className="max-w-md text-sm text-muted-foreground">{description}</p> : null}
      {action ? <Action action={action} /> : null}
    </div>
  )
}

export function EmptyStateRow({
  colSpan,
  ...props
}: EmptyStateProps & { colSpan: number }) {
  return (
    <tr>
      <td colSpan={colSpan} className="p-0">
        <EmptyState size="compact" {...props} />
      </td>
    </tr>
  )
}
