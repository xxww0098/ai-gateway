import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { toast } from 'sonner'
import { cn } from '@/shared/utils/utils'

type CopyFieldProps = {
  label: string
  value: string
  multiline?: boolean
  className?: string
}

export function CopyField({ label, value, multiline = false, className }: CopyFieldProps) {
  const [copied, setCopied] = useState(false)

  function handleCopy() {
    void navigator.clipboard.writeText(value).then(
      () => {
        setCopied(true)
        toast.success('已复制')
        window.setTimeout(() => {
          setCopied(false)
        }, 1600)
      },
      () => {
        toast.error('复制失败')
      },
    )
  }

  return (
    <div className={cn('space-y-2', className)}>
      <div className="flex items-center justify-between gap-3">
        <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {label}
        </span>
        <button
          type="button"
          onClick={handleCopy}
          className="inline-flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
          {copied ? '已复制' : '复制'}
        </button>
      </div>
      <pre
        className={cn(
          'overflow-x-auto rounded-xl border border-border bg-muted/40 p-3 font-mono text-xs sm:text-sm tracking-tight text-foreground shadow-xs',
          multiline && 'whitespace-pre-wrap leading-relaxed text-foreground',
        )}
      >
        {value}
      </pre>
    </div>
  )
}
