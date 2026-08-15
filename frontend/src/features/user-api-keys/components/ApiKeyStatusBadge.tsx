interface Props {
  status: string
}

export function ApiKeyStatusBadge({ status }: Props) {
  if (status === 'active') {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-green-50 dark:bg-green-900/20 px-2.5 py-1 text-xs font-semibold text-green-600 dark:text-green-400 border border-green-200 dark:border-green-900/50">
        <span className="h-1.5 w-1.5 rounded-full bg-green-500"></span>
        正常
      </span>
    )
  }
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full bg-gray-100 dark:bg-dark-800 px-2.5 py-1 text-xs font-semibold text-gray-600 dark:text-gray-400 border border-border">
      <span className="h-1.5 w-1.5 rounded-full bg-gray-500"></span>
      禁用
    </span>
  )
}
