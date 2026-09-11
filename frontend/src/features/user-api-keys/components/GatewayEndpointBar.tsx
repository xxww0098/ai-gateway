import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Copy, Check, ExternalLink } from 'lucide-react'
import { toast } from 'sonner'
import { Link } from 'react-router-dom'
import { docsPath } from '@/shared/routes/docs'
import { copyTextToClipboard } from '@/shared/utils/clipboard'
import {
  GATEWAY_INFERENCE_PROTOCOLS,
  GATEWAY_SDK_BASES,
  protocolEndpointUrl,
  sdkBaseUrl,
} from '../endpoints'

export function GatewayEndpointBar() {
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const timerRef = useRef<number>(0)
  const origin = typeof window !== 'undefined' ? window.location.origin : ''

  useEffect(() => () => window.clearTimeout(timerRef.current), [])

  const copyValue = async (key: string, value: string, success: string) => {
    const ok = await copyTextToClipboard(value)
    if (!ok) {
      toast.error('复制失败，请手动选择复制')
      return
    }
    window.clearTimeout(timerRef.current)
    setCopiedKey(key)
    toast.success(success)
    timerRef.current = window.setTimeout(() => setCopiedKey(null), 2000)
  }

  return (
    <div className="rounded-xl border border-border bg-card shadow-2xs">
      <div className="grid grid-cols-1 md:grid-cols-3 md:divide-x divide-y md:divide-y-0 divide-border">
        {GATEWAY_INFERENCE_PROTOCOLS.map((protocol) => {
          const url = protocolEndpointUrl(origin, protocol.path)
          const copyKey = `endpoint:${protocol.id}`
          return (
            <div key={protocol.id} className="flex min-w-0 flex-col gap-2 px-3.5 py-3 sm:px-4">
              <div className="space-y-0.5">
                <div className="text-sm font-semibold text-foreground">{protocol.name}</div>
                <p className="text-[11px] text-muted-foreground">{protocol.clients}</p>
              </div>
              <CopyChip
                copyKey={copyKey}
                copiedKey={copiedKey}
                value={url}
                label={`${protocol.name} 端点`}
                title={`点击复制 ${url}`}
                onCopy={copyValue}
              >
                <span className="rounded bg-background px-1 py-px font-sans text-[10px] font-semibold tracking-wide text-foreground">
                  {protocol.method}
                </span>
                <span className="min-w-0 truncate">{protocol.path}</span>
              </CopyChip>
            </div>
          )
        })}
      </div>

      <div className="flex flex-col gap-2 border-t border-border px-3.5 py-2.5 sm:px-4">
        <div className="flex items-center justify-between gap-3">
          <span className="text-xs font-medium text-muted-foreground">客户端 Base URL</span>
          <Link
            to={docsPath('quickstart')}
            className="inline-flex min-h-9 shrink-0 items-center gap-1 text-xs font-medium text-primary hover:underline"
          >
            <span>查看快速接入指南</span>
            <ExternalLink className="h-3 w-3" aria-hidden="true" />
          </Link>
        </div>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {GATEWAY_SDK_BASES.map((base) => {
            const url = sdkBaseUrl(origin, base.id)
            const copyKey = `sdk:${base.id}`
            return (
              <CopyChip
                key={base.id}
                copyKey={copyKey}
                copiedKey={copiedKey}
                value={url}
                label={`${base.name} Base URL`}
                title={`${base.hint} · 点击复制 ${url}`}
                onCopy={copyValue}
                wide
              >
                <span className="shrink-0 font-sans text-[11px] font-medium text-foreground">
                  {base.name}
                </span>
                <span className="min-w-0 truncate sm:hidden">
                  {base.id === 'openai' ? '/v1' : '裸主机'}
                </span>
                <span className="hidden min-w-0 truncate sm:inline">{url}</span>
              </CopyChip>
            )
          })}
        </div>
      </div>
    </div>
  )
}

function CopyChip({
  copyKey,
  copiedKey,
  value,
  label,
  title,
  onCopy,
  children,
  wide = false,
}: {
  copyKey: string
  copiedKey: string | null
  value: string
  label: string
  title: string
  onCopy: (key: string, value: string, success: string) => void
  children: ReactNode
  wide?: boolean
}) {
  const copied = copiedKey === copyKey
  return (
    <button
      type="button"
      onClick={() => void onCopy(copyKey, value, `已复制 ${label}`)}
      aria-label={`复制 ${label}`}
      title={title}
      className={`group inline-flex min-h-11 items-center gap-1.5 rounded-md border border-border bg-muted/60 px-2.5 py-2 text-left font-mono text-[11px] text-foreground transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:min-h-0 sm:text-xs md:py-1.5 ${
        wide ? 'w-full justify-between' : 'w-full justify-between md:w-auto md:justify-start'
      }`}
    >
      <span className="inline-flex min-w-0 items-center gap-1.5">{children}</span>
      {copied ? (
        <Check className="h-4 w-4 shrink-0 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
      ) : (
        <Copy
          className="h-4 w-4 shrink-0 text-foreground/55 transition-colors group-hover:text-foreground"
          aria-hidden="true"
        />
      )}
    </button>
  )
}
