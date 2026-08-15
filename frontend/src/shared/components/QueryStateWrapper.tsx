import type { ReactNode } from 'react'
import { Button } from '@/shared/components/ui/button'
import { Loader2, AlertCircle, Inbox } from 'lucide-react'

export interface QueryStateWrapperProps {
  /** Whether the query is currently loading */
  isLoading: boolean
  /** Error object from the query (null/undefined means no error) */
  error?: Error | null
  /** Whether the data set is empty (after successful load) */
  isEmpty?: boolean
  /** Callback to retry the failed query */
  onRetry?: () => void
  /** Custom message for the empty state */
  emptyMessage?: string
  /** Custom message for the loading state */
  loadingMessage?: string
  /** Content to render when data is available */
  children: ReactNode
  /** Optional className for the wrapper container */
  className?: string
}

/**
 * Shared component that handles loading, error (with retry), and empty states
 * for react-query powered views. Renders children when data is available.
 *
 * Validates: Requirements 3.4, 3.5, 3.6
 */
export function QueryStateWrapper({
  isLoading,
  error,
  isEmpty,
  onRetry,
  emptyMessage = '暂无数据',
  loadingMessage = '加载中...',
  children,
  className,
}: QueryStateWrapperProps) {
  if (isLoading) {
    return (
      <div className={className ?? 'flex flex-col items-center justify-center py-16 gap-3'}>
        <Loader2 className="h-8 w-8 animate-spin text-primary" />
        <p className="text-sm text-muted-foreground">{loadingMessage}</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className={className ?? 'flex flex-col items-center justify-center py-16 gap-4'}>
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-destructive/10">
          <AlertCircle className="h-6 w-6 text-destructive" />
        </div>
        <div className="text-center space-y-1">
          <p className="text-sm font-medium text-foreground">加载失败</p>
          <p className="text-xs text-muted-foreground max-w-sm">
            {error.message || '请求异常，请稍后重试'}
          </p>
        </div>
        {onRetry && (
          <Button variant="outline" size="sm" onClick={onRetry}>
            重试
          </Button>
        )}
      </div>
    )
  }

  if (isEmpty) {
    return (
      <div className={className ?? 'flex flex-col items-center justify-center py-16 gap-3'}>
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted">
          <Inbox className="h-6 w-6 text-muted-foreground" />
        </div>
        <p className="text-sm text-muted-foreground">{emptyMessage}</p>
      </div>
    )
  }

  return <>{children}</>
}
