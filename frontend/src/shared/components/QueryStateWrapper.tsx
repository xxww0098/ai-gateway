import type { ReactNode } from 'react'
import { Loader2, Inbox, AlertCircle } from 'lucide-react'
import { EmptyState, type EmptyStateProps } from '@/shared/components/EmptyState'

export interface QueryStateWrapperProps {
  /** Whether the query is currently loading */
  isLoading: boolean
  /** Error object from the query (null/undefined means no error) */
  error?: Error | null
  /** Whether the data set is empty (after successful load) */
  isEmpty?: boolean
  /** Callback to retry the failed query */
  onRetry?: () => void
  /**
   * 完整的空状态描述：这里将出现什么、为什么值得填满它、现在该点哪。
   * 优先于 `emptyMessage`。新代码一律用这个。
   */
  empty?: EmptyStateProps
  /**
   * 只给一行标题的简写。
   * @deprecated 一行「暂无 XXX」是死胡同，用 `empty` 把下一步写清楚。
   */
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
  empty,
  emptyMessage,
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
      <EmptyState
        className={className}
        tone="error"
        icon={AlertCircle}
        title="没能加载出来"
        description={error.message || '请求异常。检查网络后重试，若持续失败请提交工单。'}
        action={onRetry ? { label: '重试', onClick: onRetry } : undefined}
      />
    )
  }

  if (isEmpty) {
    return (
      <EmptyState
        className={className}
        icon={Inbox}
        title={emptyMessage ?? '这里还是空的'}
        {...empty}
      />
    )
  }

  return <>{children}</>
}
