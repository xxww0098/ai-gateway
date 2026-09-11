import { useState } from 'react'
import { Copy, Check, Sparkles, Layers, Terminal } from 'lucide-react'
import { toast } from 'sonner'

interface ModelCatalogHeaderProps {
  totalModels: number
  totalProviders: number
  rateMultiplier: number
}

export function ModelCatalogHeader({
  totalModels,
  totalProviders,
  rateMultiplier,
}: ModelCatalogHeaderProps) {
  const [copiedBaseUrl, setCopiedBaseUrl] = useState(false)

  const origin = typeof window !== 'undefined' ? window.location.origin : 'https://api.ai-gateway.com'
  const baseUrl = `${origin}/v1`

  const handleCopyBaseUrl = () => {
    navigator.clipboard.writeText(baseUrl)
    setCopiedBaseUrl(true)
    toast.success('API Base URL 已复制')
    setTimeout(() => setCopiedBaseUrl(false), 2000)
  }

  return (
    <div className="flex flex-col gap-4 border-b border-border/60 pb-6 sm:flex-row sm:items-end sm:justify-between">
      <div className="space-y-1.5">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl">
            模型广场与定价
          </h1>
          {rateMultiplier !== 1 && (
            <span className="rounded-full border border-primary/30 bg-primary/10 px-2.5 py-0.5 text-xs font-semibold text-primary">
              全局费率 {rateMultiplier}x
            </span>
          )}
        </div>
        <p className="text-sm text-muted-foreground max-w-2xl leading-relaxed">
          查看网关已接入的上游模型、上下文规格、模态能力与实时 Token 精算费率。输入、输出、缓存与推理按真实用量毫厘精算。
        </p>
      </div>

      {/* Overview Stat Badges */}
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <div className="flex items-center gap-1.5 rounded-xl border border-border/80 bg-card px-3 py-2 text-muted-foreground shadow-2xs">
          <Layers className="h-3.5 w-3.5 text-primary" />
          <span>可用模型</span>
          <span className="font-bold tabular-nums text-foreground">{totalModels}</span>
        </div>

        <div className="flex items-center gap-1.5 rounded-xl border border-border/80 bg-card px-3 py-2 text-muted-foreground shadow-2xs">
          <Sparkles className="h-3.5 w-3.5 text-amber-500" />
          <span>支持厂商</span>
          <span className="font-bold tabular-nums text-foreground">{totalProviders}</span>
        </div>

        <button
          type="button"
          onClick={handleCopyBaseUrl}
          className="inline-flex items-center gap-1.5 rounded-xl border border-border/80 bg-card px-3 py-2 font-mono text-xs text-muted-foreground shadow-2xs transition hover:border-primary/40 hover:text-foreground active:scale-[0.98]"
          title="点击复制 API Base URL"
        >
          <Terminal className="h-3.5 w-3.5 text-primary" />
          <span className="text-foreground/80 font-medium">Base URL:</span>
          <span className="max-w-[140px] truncate sm:max-w-none">{baseUrl}</span>
          {copiedBaseUrl ? (
            <Check className="h-3.5 w-3.5 text-emerald-500 shrink-0" />
          ) : (
            <Copy className="h-3.5 w-3.5 shrink-0 opacity-60" />
          )}
        </button>
      </div>
    </div>
  )
}
