import { useState, useRef, useId, useCallback, useEffect } from 'react'
import { Check, Copy, Terminal, Info, Zap, DollarSign, Brain, Database, Cpu } from 'lucide-react'
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from '@/shared/components/ui/table'
import type { ModelCatalogItem } from '@/features/pricing/model_catalog'
import {
  getModelProviderKey,
  getProviderDisplayName,
  formatTokenLimit,
  hasModelDetails,
} from '@/features/pricing/model_catalog'
import { getProviderStyle, getProviderBrandIcon } from '@/features/pricing/modelCatalogUtils'
import { EditablePriceCell, ModelDetailsTooltipPortal } from './ModelCatalogCard'

interface ModelCatalogTableProps {
  models: ModelCatalogItem[]
  isAdmin: boolean
  copiedId: string | null
  onCopy: (id: string) => void
  onOpenCode: (model: ModelCatalogItem) => void
  onPriceSaved: () => void
}

function TableModelRow({
  model,
  isAdmin,
  copied,
  onCopy,
  onOpenCode,
  onPriceSaved,
}: {
  model: ModelCatalogItem
  isAdmin: boolean
  copied: boolean
  onCopy: (id: string) => void
  onOpenCode: (model: ModelCatalogItem) => void
  onPriceSaved: () => void
}) {
  const provider = getModelProviderKey(model)
  const providerLabel = getProviderDisplayName(provider)
  const style = getProviderStyle(provider)
  const BrandIcon = getProviderBrandIcon(provider)

  const showDetails = hasModelDetails(model)
  const [detailsOpen, setDetailsOpen] = useState(false)
  const detailsAnchorRef = useRef<HTMLDivElement | null>(null)
  const closeTooltipTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const detailsOwnerId = useId()

  const cancelTooltipClose = useCallback(() => {
    if (closeTooltipTimerRef.current) {
      clearTimeout(closeTooltipTimerRef.current)
      closeTooltipTimerRef.current = null
    }
  }, [])

  const scheduleTooltipClose = useCallback(() => {
    cancelTooltipClose()
    closeTooltipTimerRef.current = setTimeout(() => {
      closeTooltipTimerRef.current = null
      setDetailsOpen(false)
    }, 200)
  }, [cancelTooltipClose])

  useEffect(() => {
    if (!detailsOpen) {
      cancelTooltipClose()
      return
    }

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Element | null
      if (!target) return
      if (detailsAnchorRef.current?.contains(target)) return
      const portal = target.closest('[data-model-details-portal-owner]')
      if (portal?.getAttribute('data-model-details-portal-owner') === detailsOwnerId) return
      cancelTooltipClose()
      setDetailsOpen(false)
    }

    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        cancelTooltipClose()
        setDetailsOpen(false)
      }
    }

    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('keydown', handleEscape)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('keydown', handleEscape)
      cancelTooltipClose()
    }
  }, [detailsOpen, detailsOwnerId, cancelTooltipClose])

  const contextLimit = model.context_length ?? model.inputTokenLimit
  const outputLimit = model.max_completion_tokens ?? model.outputTokenLimit

  const isThinking = Boolean(model.thinking || (model.reasoning_price_per_1m ?? 0) > 0)
  const isVision = Boolean(
    model.supportedInputModalities?.includes('image') ||
      model.id.toLowerCase().includes('vision') ||
      model.id.toLowerCase().includes('4o')
  )

  return (
    <TableRow className="group/row transition-colors hover:bg-muted/40">
      {/* Model Name & ID */}
      <TableCell className="py-3.5 pl-4 sm:pl-6">
        <div className="flex items-start gap-3">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 mt-0.5">
            {BrandIcon ? (
              <BrandIcon size={18} className="shrink-0" />
            ) : (
              <Cpu className="h-4 w-4 text-primary" />
            )}
          </div>
          <div className="min-w-0 max-w-xs sm:max-w-sm space-y-1">
            <div className="flex items-center gap-2">
              <span className="font-semibold text-foreground text-xs sm:text-sm truncate" title={model.display_name || model.id}>
                {model.display_name || model.id}
              </span>
            </div>
            <div className="flex items-center gap-1.5">
              <code
                onClick={() => onCopy(model.id)}
                className="inline-block truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground hover:text-foreground hover:bg-muted/80 cursor-pointer transition"
                title="点击复制模型 ID"
              >
                {model.id}
              </code>
              <button
                type="button"
                onClick={() => onCopy(model.id)}
                className="opacity-0 group-hover/row:opacity-100 transition text-muted-foreground hover:text-foreground"
                title="复制模型 ID"
              >
                {copied ? (
                  <Check className="h-3 w-3 text-emerald-500" />
                ) : (
                  <Copy className="h-3 w-3" />
                )}
              </button>
            </div>
            {/* Capability tags */}
            <div className="flex flex-wrap items-center gap-1 pt-0.5">
              {isThinking && (
                <span className="rounded bg-purple-100 dark:bg-purple-950/50 px-1.5 py-0.2 text-[10px] font-medium text-purple-700 dark:text-purple-300">
                  思考
                </span>
              )}
              {isVision && (
                <span className="rounded bg-blue-100 dark:bg-blue-950/50 px-1.5 py-0.2 text-[10px] font-medium text-blue-700 dark:text-blue-300">
                  视觉
                </span>
              )}
            </div>
          </div>
        </div>
      </TableCell>

      {/* Provider Badge */}
      <TableCell className="py-3.5 whitespace-nowrap">
        <span className={`inline-flex items-center rounded-lg border px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide ${style.border} ${style.bg} ${style.text}`}>
          {providerLabel}
        </span>
      </TableCell>

      {/* Context & Limits */}
      <TableCell className="py-3.5 whitespace-nowrap text-xs">
        <div className="space-y-0.5 tabular-nums">
          <div className="font-semibold text-foreground">
            {contextLimit ? `${formatTokenLimit(contextLimit)} tokens` : '-'}
          </div>
          {outputLimit && (
            <div className="text-[11px] text-muted-foreground">
              最大输出 {formatTokenLimit(outputLimit)}
            </div>
          )}
        </div>
      </TableCell>

      {/* Input Price */}
      <TableCell className="py-3.5 whitespace-nowrap">
        <div className="min-w-[110px]">
          <EditablePriceCell
            modelId={model.id}
            field="input"
            price={model.input_price_per_1m}
            icon={<Zap className="h-3 w-3 text-blue-500 flex-shrink-0" />}
            label="输入"
            isAdmin={isAdmin}
            onSaved={onPriceSaved}
            currentPrices={{
              input_price_per_1m: model.input_price_per_1m,
              output_price_per_1m: model.output_price_per_1m,
              cached_input_price_per_1m: model.cached_input_price_per_1m,
              reasoning_price_per_1m: model.reasoning_price_per_1m,
            }}
          />
        </div>
      </TableCell>

      {/* Output Price */}
      <TableCell className="py-3.5 whitespace-nowrap">
        <div className="min-w-[110px]">
          <EditablePriceCell
            modelId={model.id}
            field="output"
            price={model.output_price_per_1m}
            icon={<DollarSign className="h-3 w-3 text-emerald-500 flex-shrink-0" />}
            label="输出"
            isAdmin={isAdmin}
            onSaved={onPriceSaved}
            currentPrices={{
              input_price_per_1m: model.input_price_per_1m,
              output_price_per_1m: model.output_price_per_1m,
              cached_input_price_per_1m: model.cached_input_price_per_1m,
              reasoning_price_per_1m: model.reasoning_price_per_1m,
            }}
          />
        </div>
      </TableCell>

      {/* Cached Price */}
      <TableCell className="py-3.5 whitespace-nowrap">
        <div className="min-w-[110px]">
          {isAdmin || (model.cached_input_price_per_1m ?? 0) > 0 ? (
            <EditablePriceCell
              modelId={model.id}
              field="cached_input"
              price={model.cached_input_price_per_1m}
              icon={<Database className="h-3 w-3 text-amber-500 flex-shrink-0" />}
              label="缓存"
              isAdmin={isAdmin}
              onSaved={onPriceSaved}
              currentPrices={{
                input_price_per_1m: model.input_price_per_1m,
                output_price_per_1m: model.output_price_per_1m,
                cached_input_price_per_1m: model.cached_input_price_per_1m,
                reasoning_price_per_1m: model.reasoning_price_per_1m,
              }}
            />
          ) : (
            <span className="text-xs text-muted-foreground/60 pl-2">-</span>
          )}
        </div>
      </TableCell>

      {/* Reasoning Price */}
      <TableCell className="py-3.5 whitespace-nowrap">
        <div className="min-w-[110px]">
          {isAdmin || (model.reasoning_price_per_1m ?? 0) > 0 ? (
            <EditablePriceCell
              modelId={model.id}
              field="reasoning"
              price={model.reasoning_price_per_1m}
              icon={<Brain className="h-3 w-3 text-purple-500 flex-shrink-0" />}
              label="推理"
              isAdmin={isAdmin}
              onSaved={onPriceSaved}
              currentPrices={{
                input_price_per_1m: model.input_price_per_1m,
                output_price_per_1m: model.output_price_per_1m,
                cached_input_price_per_1m: model.cached_input_price_per_1m,
                reasoning_price_per_1m: model.reasoning_price_per_1m,
              }}
            />
          ) : (
            <span className="text-xs text-muted-foreground/60 pl-2">-</span>
          )}
        </div>
      </TableCell>

      {/* Actions */}
      <TableCell className="py-3.5 pr-4 sm:pr-6 whitespace-nowrap text-right">
        <div className="flex items-center justify-end gap-1">
          <button
            type="button"
            onClick={() => onOpenCode(model)}
            className="inline-flex h-7 items-center gap-1 rounded-lg border border-border/80 bg-background px-2 text-xs font-medium text-muted-foreground shadow-2xs transition hover:bg-muted hover:text-foreground active:scale-95"
            title="查看快速调用代码"
          >
            <Terminal className="h-3 w-3 text-primary" />
            <span>示例</span>
          </button>

          {showDetails && (
            <div
              className="relative"
              ref={detailsAnchorRef}
              onPointerEnter={() => {
                cancelTooltipClose()
                setDetailsOpen(true)
              }}
              onPointerLeave={scheduleTooltipClose}
            >
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation()
                  cancelTooltipClose()
                  setDetailsOpen((current) => !current)
                }}
                onFocus={() => {
                  cancelTooltipClose()
                  setDetailsOpen(true)
                }}
                className="flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground transition hover:bg-muted hover:text-foreground focus:bg-muted focus:text-foreground focus:outline-none"
                title="查看模型参数详情"
              >
                <Info className="h-3.5 w-3.5" />
              </button>
              {detailsOpen && (
                <ModelDetailsTooltipPortal
                  model={model}
                  providerLabel={providerLabel}
                  anchorRef={detailsAnchorRef}
                  ownerId={detailsOwnerId}
                  onPointerEnter={cancelTooltipClose}
                  onPointerLeave={scheduleTooltipClose}
                />
              )}
            </div>
          )}
        </div>
      </TableCell>
    </TableRow>
  )
}

export function ModelCatalogTable({
  models,
  isAdmin,
  copiedId,
  onCopy,
  onOpenCode,
  onPriceSaved,
}: ModelCatalogTableProps) {
  return (
    <div className="rounded-2xl border border-border/80 bg-card overflow-hidden shadow-2xs">
      <div className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow className="border-b border-border/80 bg-muted/40 hover:bg-muted/40">
              <TableHead className="py-3 pl-4 sm:pl-6 text-xs font-bold text-foreground">
                模型与标识
              </TableHead>
              <TableHead className="py-3 text-xs font-bold text-foreground">
                供应商
              </TableHead>
              <TableHead className="py-3 text-xs font-bold text-foreground">
                上下文规格
              </TableHead>
              <TableHead className="py-3 text-xs font-bold text-foreground">
                输入价格 / 1M
              </TableHead>
              <TableHead className="py-3 text-xs font-bold text-foreground">
                输出价格 / 1M
              </TableHead>
              <TableHead className="py-3 text-xs font-bold text-foreground">
                缓存输入 / 1M
              </TableHead>
              <TableHead className="py-3 text-xs font-bold text-foreground">
                推理单价 / 1M
              </TableHead>
              <TableHead className="py-3 pr-4 sm:pr-6 text-right text-xs font-bold text-foreground">
                操作
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {models.map((model) => (
              <TableModelRow
                key={model.id}
                model={model}
                isAdmin={isAdmin}
                copied={copiedId === model.id}
                onCopy={onCopy}
                onOpenCode={onOpenCode}
                onPriceSaved={onPriceSaved}
              />
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  )
}
