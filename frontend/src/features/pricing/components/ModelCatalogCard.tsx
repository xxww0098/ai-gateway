import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import type { ComponentPropsWithoutRef, CSSProperties, KeyboardEvent, ReactNode, RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Brain, Check, Copy, Database, DollarSign, Info, Loader2, Pencil, Terminal, Zap } from 'lucide-react'
import { toast } from 'sonner'
import { useUpdatePrice } from '@/features/pricing/hooks'
import {
  getModelDetailMetrics,
  getModelProviderKey,
  getProviderDisplayName,
  getSupportedMethods,
  hasModelDetails,
  type ModelCatalogItem,
} from '@/features/pricing/model_catalog'
import { getProviderStyle, getProviderBrandIcon } from '@/features/pricing/modelCatalogUtils'

export function formatPrice(price?: number): string {
  if (price === undefined || price === null || price === 0) return '免费'
  if (price < 0.01) return `$${price.toFixed(4)}`
  return `$${price.toFixed(2)}`
}

const modelDetailsPanelClass =
  'w-80 max-w-[min(24rem,calc(100vw-2rem))] rounded-xl border border-border bg-popover text-popover-foreground p-3.5 text-left shadow-lg shadow-black/10 dark:shadow-black/40 sm:w-96'

function ModelDetailsTooltip({
  model,
  providerLabel,
  className = '',
  style,
  ...divProps
}: {
  model: ModelCatalogItem
  providerLabel: string
  className?: string
  style?: CSSProperties
} & Omit<ComponentPropsWithoutRef<'div'>, 'children'>) {
  const metrics = getModelDetailMetrics(model)
  const methods = getSupportedMethods(model)
  const methodsLabel = model.supported_parameters?.length
    ? '支持参数'
    : model.supportedGenerationMethods?.length
      ? '生成方法'
      : '模态能力'

  return (
    <div
      role="tooltip"
      className={`${modelDetailsPanelClass} ${className}`.trim()}
      style={style}
      {...divProps}
    >
      <div className="space-y-1">
        <div className="flex items-center gap-2">
          <span className="min-w-0 truncate text-sm font-semibold text-foreground">
            {model.display_name || model.id}
          </span>
          <span className="rounded-md bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
            {providerLabel}
          </span>
        </div>
        <p className="break-all font-mono text-xs text-muted-foreground">{model.id}</p>
        {model.description && (
          <p className="text-xs leading-relaxed text-muted-foreground">{model.description}</p>
        )}
      </div>

      {metrics.length > 0 && (
        <div className="mt-3 grid grid-cols-2 gap-2">
          {metrics.map((metric) => (
            <div key={metric.label} className="rounded-lg bg-muted px-2.5 py-2">
              <p className="text-[10px] font-medium text-muted-foreground">{metric.label}</p>
              <p className="mt-0.5 break-words text-xs font-semibold tabular-nums text-foreground">{metric.value}</p>
            </div>
          ))}
        </div>
      )}

      {methods.length > 0 && (
        <div className="mt-3">
          <p className="text-[10px] font-medium text-muted-foreground">{methodsLabel}</p>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {methods.slice(0, 6).map((method) => (
              <span key={method} className="rounded-md bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary">
                {method}
              </span>
            ))}
            {methods.length > 6 && (
              <span className="rounded-md bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                +{methods.length - 6}
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

export function ModelDetailsTooltipPortal({
  model,
  providerLabel,
  anchorRef,
  ownerId,
  onPointerEnter,
  onPointerLeave,
}: {
  model: ModelCatalogItem
  providerLabel: string
  anchorRef: RefObject<HTMLDivElement | null>
  ownerId: string
  onPointerEnter: () => void
  onPointerLeave: () => void
}) {
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null)

  useLayoutEffect(() => {
    const anchorEl = anchorRef.current
    if (!anchorEl) {
      setPos(null)
      return
    }
    const update = () => {
      const r = anchorEl.getBoundingClientRect()
      setPos({ top: r.bottom + 8, right: window.innerWidth - r.right })
    }
    update()
    const scrollOpts: AddEventListenerOptions = { capture: true, passive: true }
    window.addEventListener('scroll', update, scrollOpts)
    window.addEventListener('resize', update)
    return () => {
      window.removeEventListener('scroll', update, scrollOpts)
      window.removeEventListener('resize', update)
    }
  }, [anchorRef, model.id])

  if (pos === null) return null

  return createPortal(
    <ModelDetailsTooltip
      model={model}
      providerLabel={providerLabel}
      data-model-details-portal-owner={ownerId}
      className="z-[110]"
      style={{ position: 'fixed', top: pos.top, right: pos.right, zIndex: 110 }}
      onPointerEnter={onPointerEnter}
      onPointerLeave={onPointerLeave}
    />,
    document.body
  )
}

export interface EditablePriceCellProps {
  modelId: string
  field: 'input' | 'output' | 'cached_input' | 'reasoning'
  price: number | undefined
  icon: ReactNode
  label: string
  isAdmin: boolean
  onSaved: () => void
  currentPrices: {
    input_price_per_1m?: number
    output_price_per_1m?: number
    cached_input_price_per_1m?: number
    reasoning_price_per_1m?: number
  }
}

export function EditablePriceCell({
  modelId,
  field,
  price,
  icon,
  label,
  isAdmin,
  onSaved,
  currentPrices,
}: EditablePriceCellProps) {
  const [editing, setEditing] = useState(false)
  const [value, setValue] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)
  const updatePrice = useUpdatePrice()

  const startEditing = useCallback(() => {
    if (!isAdmin) return
    const p = price ?? 0
    setValue(p === 0 ? '' : p.toString())
    setEditing(true)
  }, [isAdmin, price])

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus()
      inputRef.current.select()
    }
  }, [editing])

  const cancel = useCallback(() => {
    setEditing(false)
    setValue('')
  }, [])

  const save = useCallback(async () => {
    const newPrice = parseFloat(value) || 0
    const oldPrice = price ?? 0

    if (newPrice === oldPrice) {
      cancel()
      return
    }

    updatePrice.mutate(
      {
        model_id: modelId,
        input_price_per_1m: field === 'input' ? newPrice : (currentPrices.input_price_per_1m ?? 0),
        output_price_per_1m: field === 'output' ? newPrice : (currentPrices.output_price_per_1m ?? 0),
        cached_input_price_per_1m: field === 'cached_input' ? newPrice : (currentPrices.cached_input_price_per_1m ?? 0),
        reasoning_price_per_1m: field === 'reasoning' ? newPrice : (currentPrices.reasoning_price_per_1m ?? 0),
      },
      {
        onSuccess: () => {
          toast.success(`${modelId} ${label}已更新为 $${newPrice.toFixed(4)}/1M`)
          setEditing(false)
          onSaved()
        },
      }
    )
  }, [value, price, modelId, field, label, currentPrices, cancel, onSaved, updatePrice])

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        e.preventDefault()
        save()
      } else if (e.key === 'Escape') {
        cancel()
      }
    },
    [save, cancel]
  )

  if (editing) {
    return (
      <div className="flex items-center gap-1.5 rounded-lg bg-primary/10 ring-2 ring-primary/40 px-2.5 py-1.5 transition-all">
        {icon}
        <div className="min-w-0 flex-1">
          <p className="text-[10px] text-primary leading-none font-medium">{label} /1M</p>
          <div className="flex items-center gap-1 mt-0.5">
            <span className="text-xs text-muted-foreground select-none">$</span>
            <input
              ref={inputRef}
              type="number"
              step="0.0001"
              min="0"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={handleKeyDown}
              onBlur={save}
              disabled={updatePrice.isPending}
              className="w-full bg-transparent text-xs font-semibold tabular-nums text-foreground outline-none placeholder:text-muted-foreground [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none"
              placeholder="0.00"
            />
            {updatePrice.isPending && <Loader2 className="h-3 w-3 animate-spin text-primary flex-shrink-0" />}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div
      className={`flex items-center gap-1.5 rounded-lg bg-muted px-2.5 py-2 transition-all ${
        isAdmin
          ? 'cursor-pointer hover:bg-primary/10 hover:ring-1 hover:ring-primary/40 group/price'
          : ''
      }`}
      onClick={startEditing}
      title={isAdmin ? '点击编辑价格' : undefined}
    >
      {icon}
      <div className="min-w-0 flex-1">
        <p className="text-[10px] text-muted-foreground leading-none">{label} /1M</p>
        <p className="text-xs font-semibold tabular-nums text-foreground mt-0.5">{formatPrice(price)}</p>
      </div>
      {isAdmin && (
        <Pencil className="h-3 w-3 text-muted-foreground opacity-0 group-hover/price:opacity-100 transition-opacity flex-shrink-0" />
      )}
    </div>
  )
}

export interface ModelCatalogCardProps {
  model: ModelCatalogItem
  isAdmin?: boolean
  copied?: boolean
  onCopy?: (modelId: string) => void
  onOpenCode?: (model: ModelCatalogItem) => void
  onPriceSaved?: () => void
  showPricing?: boolean
  showCopy?: boolean
  showInlineMetrics?: boolean
  sourceBadges?: string[]
}

export function ModelCatalogCard({
  model,
  isAdmin = false,
  copied = false,
  onCopy,
  onOpenCode,
  onPriceSaved,
  showPricing = true,
  showCopy = true,
  showInlineMetrics = true,
  sourceBadges = [],
}: ModelCatalogCardProps) {
  const provider = getModelProviderKey(model)
  const providerLabel = getProviderDisplayName(provider)
  const style = getProviderStyle(provider)
  const BrandIcon = getProviderBrandIcon(provider)
  const hasReasoning = isAdmin || (model.reasoning_price_per_1m ?? 0) > 0
  const hasCached = isAdmin || (model.cached_input_price_per_1m ?? 0) > 0
  const showDetails = hasModelDetails(model)
  const inlineMetrics = showInlineMetrics ? getModelDetailMetrics(model).filter((metric) => metric.label !== '类型').slice(0, 3) : []
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

    const handleEscape = (event: globalThis.KeyboardEvent) => {
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

  const handleCopy = useCallback(() => {
    onCopy?.(model.id)
  }, [model.id, onCopy])

  return (
    <div className="group relative flex flex-col justify-between rounded-2xl border border-border/80 bg-card p-5 shadow-2xs transition-all duration-150 hover:border-border hover:shadow-xs">
      <div>
        {/* Header Row: Provider Badge + Actions */}
        <div className="mb-3 flex items-center justify-between gap-2">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className={`inline-flex items-center gap-1.5 rounded-lg border px-2 py-1 text-[11px] font-bold ${style.border} ${style.bg} ${style.text}`}>
              {BrandIcon ? (
                <BrandIcon size={13} className="shrink-0" />
              ) : null}
              <span>{providerLabel}</span>
            </span>
            {sourceBadges.map((source) => (
              <span
                key={source}
                className="inline-flex items-center rounded-lg bg-muted px-2 py-0.5 text-[10px] font-bold text-muted-foreground"
              >
                {source}
              </span>
            ))}
          </div>

          <div className="flex flex-shrink-0 items-center gap-1">
            {onOpenCode && (
              <button
                type="button"
                onClick={() => onOpenCode(model)}
                className="inline-flex h-7 items-center gap-1 rounded-lg border border-border/60 bg-muted/30 px-2 text-[11px] font-semibold text-muted-foreground transition-colors hover:bg-muted hover:text-foreground active:scale-95"
                title="查看调用代码示例"
              >
                <Terminal className="h-3 w-3 text-primary" />
                <span>代码</span>
              </button>
            )}
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
                  className="flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus:bg-muted focus:text-foreground focus:outline-none"
                  aria-label={`查看 ${model.id} 模型详情`}
                  title="模型规格详情"
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
            {showCopy && onCopy && (
              <button
                type="button"
                onClick={handleCopy}
                className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none active:scale-95"
                title="复制模型 ID"
              >
                {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
              </button>
            )}
          </div>
        </div>

        {/* Model Title & ID */}
        <div className="space-y-1">
          {model.display_name && model.display_name !== model.id ? (
            <div>
              <h3 className="truncate text-sm font-bold tracking-tight text-foreground" title={model.display_name}>
                {model.display_name}
              </h3>
              <div className="mt-0.5 flex items-center gap-1.5">
                <code
                  onClick={handleCopy}
                  className="truncate rounded-md bg-muted/70 px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground hover:text-foreground hover:bg-muted cursor-pointer transition"
                  title="点击复制模型 ID"
                >
                  {model.id}
                </code>
              </div>
            </div>
          ) : (
            <h3
              onClick={handleCopy}
              className="truncate font-mono text-sm font-bold tracking-tight text-foreground hover:text-primary cursor-pointer transition"
              title="点击复制模型 ID"
            >
              {model.id}
            </h3>
          )}

          {model.description && (
            <p className="line-clamp-2 text-xs text-muted-foreground leading-relaxed pt-0.5" title={model.description}>
              {model.description}
            </p>
          )}
        </div>

        {/* Specs Pills */}
        {inlineMetrics.length > 0 && (
          <div className="mt-3 flex flex-wrap items-center gap-1.5">
            {inlineMetrics.map((metric) => (
              <span
                key={metric.label}
                className="inline-flex items-center gap-1 rounded-md bg-muted/80 px-2 py-0.5 text-[11px] font-medium text-foreground/80"
              >
                <span className="text-muted-foreground text-[10px]">{metric.label}:</span>
                <span className="tabular-nums font-semibold">{metric.value}</span>
              </span>
            ))}
          </div>
        )}
      </div>

      {/* Pricing Matrix */}
      {showPricing && (
        <div className="mt-4 grid grid-cols-2 gap-2 border-t border-border/40 pt-3">
          <EditablePriceCell
            modelId={model.id}
            field="input"
            price={model.input_price_per_1m}
            icon={<Zap className="h-3 w-3 text-blue-500 flex-shrink-0" />}
            label="输入"
            isAdmin={isAdmin}
            onSaved={onPriceSaved || (() => undefined)}
            currentPrices={{
              input_price_per_1m: model.input_price_per_1m,
              output_price_per_1m: model.output_price_per_1m,
              cached_input_price_per_1m: model.cached_input_price_per_1m,
              reasoning_price_per_1m: model.reasoning_price_per_1m,
            }}
          />
          <EditablePriceCell
            modelId={model.id}
            field="output"
            price={model.output_price_per_1m}
            icon={<DollarSign className="h-3 w-3 text-emerald-500 flex-shrink-0" />}
            label="输出"
            isAdmin={isAdmin}
            onSaved={onPriceSaved || (() => undefined)}
            currentPrices={{
              input_price_per_1m: model.input_price_per_1m,
              output_price_per_1m: model.output_price_per_1m,
              cached_input_price_per_1m: model.cached_input_price_per_1m,
              reasoning_price_per_1m: model.reasoning_price_per_1m,
            }}
          />
          {hasReasoning && (
            <EditablePriceCell
              modelId={model.id}
              field="reasoning"
              price={model.reasoning_price_per_1m}
              icon={<Brain className="h-3 w-3 text-purple-500 flex-shrink-0" />}
              label="推理"
              isAdmin={isAdmin}
              onSaved={onPriceSaved || (() => undefined)}
              currentPrices={{
                input_price_per_1m: model.input_price_per_1m,
                output_price_per_1m: model.output_price_per_1m,
                cached_input_price_per_1m: model.cached_input_price_per_1m,
                reasoning_price_per_1m: model.reasoning_price_per_1m,
              }}
            />
          )}
          {hasCached && (
            <EditablePriceCell
              modelId={model.id}
              field="cached_input"
              price={model.cached_input_price_per_1m}
              icon={<Database className="h-3 w-3 text-amber-500 flex-shrink-0" />}
              label="缓存"
              isAdmin={isAdmin}
              onSaved={onPriceSaved || (() => undefined)}
              currentPrices={{
                input_price_per_1m: model.input_price_per_1m,
                output_price_per_1m: model.output_price_per_1m,
                cached_input_price_per_1m: model.cached_input_price_per_1m,
                reasoning_price_per_1m: model.reasoning_price_per_1m,
              }}
            />
          )}
        </div>
      )}
    </div>
  )
}

