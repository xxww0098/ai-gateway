interface Props {
  used: number
  total: number
  label?: string
}

export function QuotaProgressBar({ used, total, label }: Props) {
  const pct = total > 0 ? Math.min((used / total) * 100, 100) : 0
  const color =
    pct > 85
      ? "bg-red-500"
      : pct > 60
        ? "bg-amber-500"
        : "bg-emerald-500"
  const trackColor =
    pct > 85
      ? "bg-red-100 dark:bg-red-900/20"
      : pct > 60
        ? "bg-amber-100 dark:bg-amber-900/20"
        : "bg-emerald-100 dark:bg-emerald-900/20"

  return (
    <div className="min-w-[120px]">
      {label && (
        <span className="text-[10px] font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider mb-1 block">
          {label}
        </span>
      )}
      <div className={`h-1.5 rounded-full ${trackColor} overflow-hidden`}>
        <div
          className={`h-full rounded-full ${color} transition-all duration-500 ease-out`}
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="text-[11px] text-gray-500 dark:text-gray-400 font-mono mt-0.5 block">
        ${used.toFixed(2)} / ${total.toFixed(2)}
      </span>
    </div>
  )
}
