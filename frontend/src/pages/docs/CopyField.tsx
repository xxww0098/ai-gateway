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
        <span className="text-xs font-semibold uppercase tracking-wider text-gray-500 dark:text-dark-400">
          {label}
        </span>
        <button
          type="button"
          onClick={handleCopy}
          className="inline-flex items-center gap-1.5 rounded-lg px-2 py-1 text-xs font-medium text-gray-500 transition-colors hover:bg-gray-100 hover:text-gray-900 dark:text-dark-400 dark:hover:bg-dark-800 dark:hover:text-white"
        >
          {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
          {copied ? '已复制' : '复制'}
        </button>
      </div>
      <pre
        className={cn(
          'overflow-x-auto rounded-xl border border-border bg-white p-3 font-mono text-sm text-primary-600 shadow-sm dark:bg-dark-900/80 dark:text-primary-400',
          multiline && 'whitespace-pre-wrap leading-relaxed text-gray-600 dark:text-dark-300',
        )}
      >
        {value}
      </pre>
    </div>
  )
}
