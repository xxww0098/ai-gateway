import { Component, type ReactNode, type ErrorInfo } from 'react'

interface ErrorBoundaryProps {
  children?: ReactNode
  fallback?: ReactNode
  onReset?: () => void
}

interface ErrorBoundaryState {
  hasError: boolean
  error: Error | null
}

/**
 * Catches runtime errors in its child subtree and renders a fallback UI
 * instead of crashing the entire application.
 *
 * The retry button resets the boundary state, allowing children to re-render.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props)
    this.state = { hasError: false, error: null }
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    // Log to console in development; production can extend via onReset or external hook.
    console.error('[ErrorBoundary] Caught error:', error, errorInfo)
  }

  private handleReset = (): void => {
    this.setState({ hasError: false, error: null })
    this.props.onReset?.()
  }

  render(): ReactNode {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback
      }
      return (
        <DefaultErrorFallback
          error={this.state.error}
          onRetry={this.handleReset}
        />
      )
    }
    return this.props.children
  }
}

/* ------------------------------------------------------------------ */
/*  Default fallback — imported inline to avoid circular deps          */
/* ------------------------------------------------------------------ */

import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/shared/components/ui/card'
import { Button } from '@/shared/components/ui/button'
import { AlertTriangle } from 'lucide-react'

interface DefaultErrorFallbackProps {
  error: Error | null
  onRetry: () => void
}

function DefaultErrorFallback({ error, onRetry }: DefaultErrorFallbackProps) {
  return (
    <div className="flex min-h-[320px] items-center justify-center p-6">
      <Card className="max-w-md w-full border-destructive/30 bg-destructive/5">
        <CardHeader className="text-center">
          <div className="mx-auto mb-3 flex h-12 w-12 items-center justify-center rounded-full bg-destructive/10">
            <AlertTriangle className="h-6 w-6 text-destructive" />
          </div>
          <CardTitle className="text-lg">页面加载出错</CardTitle>
          <CardDescription>
            该页面遇到了一个意外错误，请尝试重新加载。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col items-center gap-4">
          {error?.message && (
            <p className="w-full rounded-md bg-muted/60 px-3 py-2 text-xs text-muted-foreground font-mono break-all">
              {error.message}
            </p>
          )}
          <Button onClick={onRetry} className="w-full">
            重新加载
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
