import { Bell, AlertCircle } from 'lucide-react'
import type { DashboardAnnouncementsProps } from '../types'

export function DashboardAnnouncements({ announcements }: DashboardAnnouncementsProps) {
  if (announcements.length === 0) return null

  return (
    <div className="space-y-3">
      {announcements.map(a => (
        <div key={a.id} className={`flex items-start gap-4 p-4 rounded-2xl border backdrop-blur-md transition-all ${
          a.type === 'danger' ? 'bg-red-500/10 border-red-500/20 text-red-600 dark:text-red-400' :
          a.type === 'warning' ? 'bg-amber-500/10 border-amber-500/20 text-amber-600 dark:text-amber-400' :
          'bg-primary-500/10 border-primary-500/20 text-primary-700 dark:text-primary-400'
        }`}>
          {a.type === 'danger' ? <AlertCircle className="h-5 w-5 mt-0.5 shrink-0" /> : <Bell className="h-5 w-5 mt-0.5 shrink-0" />}
          <div>
            <h4 className="font-semibold tracking-tight">{a.title}</h4>
          </div>
        </div>
      ))}
    </div>
  )
}
