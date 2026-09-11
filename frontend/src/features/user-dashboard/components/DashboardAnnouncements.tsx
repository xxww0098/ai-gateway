import { Bell, AlertCircle } from 'lucide-react'
import type { DashboardAnnouncementsProps } from '../types'

export function DashboardAnnouncements({ announcements }: DashboardAnnouncementsProps) {
  if (announcements.length === 0) return null

  return (
    <div className="space-y-2.5">
      {announcements.map(a => {
        const isDanger = a.type === 'danger'
        const isWarning = a.type === 'warning'
        const toneClass = isDanger
          ? 'border-destructive/30 bg-destructive/5 text-foreground'
          : isWarning
            ? 'border-amber-500/30 bg-amber-500/5 text-foreground'
            : 'border-primary/20 bg-primary/5 text-foreground'

        const iconClass = isDanger
          ? 'text-destructive'
          : isWarning
            ? 'text-amber-600 dark:text-amber-400'
            : 'text-primary'

        return (
          <div
            key={a.id}
            className={`flex items-center gap-3 rounded-[10px] border px-3.5 py-2.5 text-xs shadow-[0_1px_2px_rgb(0_0_0/0.07)] transition-colors ${toneClass}`}
          >
            {isDanger ? (
              <AlertCircle className={`size-4 shrink-0 ${iconClass}`} />
            ) : (
              <Bell className={`size-4 shrink-0 ${iconClass}`} />
            )}
            <div className="flex-1 min-w-0">
              <span className="font-medium">{a.title}</span>
            </div>
          </div>
        )
      })}
    </div>
  )
}
