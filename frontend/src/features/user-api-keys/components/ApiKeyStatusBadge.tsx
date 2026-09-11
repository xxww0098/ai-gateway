interface Props {
  status: string
}

export function ApiKeyStatusBadge({ status }: Props) {
  if (status === 'active') {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-50 dark:bg-emerald-950/30 px-2.5 py-0.5 text-xs font-medium text-emerald-700 dark:text-emerald-400 border border-emerald-200 dark:border-emerald-800/40">
        <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" aria-hidden="true" />
        正常
      </span>
    )
  }

  if (status === 'expired') {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-red-50 dark:bg-red-950/30 px-2.5 py-0.5 text-xs font-medium text-red-700 dark:text-red-400 border border-red-200 dark:border-red-800/40">
        <span className="h-1.5 w-1.5 rounded-full bg-red-500" aria-hidden="true" />
        已过期
      </span>
    )
  }

  if (status === 'exhausted') {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-50 dark:bg-amber-950/30 px-2.5 py-0.5 text-xs font-medium text-amber-700 dark:text-amber-400 border border-amber-200 dark:border-amber-800/40">
        <span className="h-1.5 w-1.5 rounded-full bg-amber-500" aria-hidden="true" />
        额度已尽
      </span>
    )
  }

  return (
    <span className="inline-flex items-center gap-1.5 rounded-full bg-muted px-2.5 py-0.5 text-xs font-medium text-muted-foreground border border-border">
      <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/60" aria-hidden="true" />
      禁用
    </span>
  )
}
