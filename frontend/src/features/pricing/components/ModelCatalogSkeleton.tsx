import type { ViewMode } from './ModelCatalogFilters'

export function ModelCatalogSkeleton({ viewMode = 'grid' }: { viewMode?: ViewMode }) {
  if (viewMode === 'table') {
    return (
      <div className="rounded-2xl border border-border/80 bg-card p-4 shadow-2xs">
        <div className="space-y-4 animate-pulse">
          <div className="h-9 w-full rounded-lg bg-muted/60" />
          {Array.from({ length: 5 }).map((_, i) => (
            <div key={i} className="flex items-center gap-4 py-2 border-b border-border/40">
              <div className="h-8 w-8 rounded-lg bg-muted" />
              <div className="space-y-1.5 flex-1">
                <div className="h-4 w-32 rounded bg-muted" />
                <div className="h-3 w-20 rounded bg-muted/60" />
              </div>
              <div className="h-6 w-20 rounded-lg bg-muted" />
              <div className="h-6 w-24 rounded-lg bg-muted" />
              <div className="h-8 w-24 rounded-lg bg-muted" />
              <div className="h-8 w-24 rounded-lg bg-muted" />
            </div>
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <div
          key={i}
          className="rounded-2xl border border-border/80 bg-card p-5 shadow-2xs space-y-4 animate-pulse"
        >
          <div className="flex items-center justify-between">
            <div className="h-5 w-20 rounded-lg bg-muted" />
            <div className="flex gap-1">
              <div className="h-7 w-7 rounded-lg bg-muted" />
              <div className="h-7 w-7 rounded-lg bg-muted" />
            </div>
          </div>

          <div className="space-y-2">
            <div className="h-5 w-40 rounded bg-muted" />
            <div className="h-3 w-24 rounded bg-muted/60" />
          </div>

          <div className="flex gap-2">
            <div className="h-5 w-16 rounded bg-muted/60" />
            <div className="h-5 w-16 rounded bg-muted/60" />
          </div>

          <div className="grid grid-cols-2 gap-2 pt-2 border-t border-border/40">
            <div className="h-10 rounded-lg bg-muted/60" />
            <div className="h-10 rounded-lg bg-muted/60" />
          </div>
        </div>
      ))}
    </div>
  )
}
