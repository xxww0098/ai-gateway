import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import type { ComponentPropsWithoutRef, CSSProperties, KeyboardEvent, ReactNode, RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Brain, Check, Copy, Database, DollarSign, Info, Loader2, Pencil, Zap } from 'lucide-react'
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
import { getProviderStyle } from '@/features/pricing/modelCatalogUtils'

function formatPrice(price?: number): string {
  if (price === undefined || price === null || price === 0) return '免费'
  if (price < 0.01) return `$${price.toFixed(4)}`
  return `$${price.toFixed(2)}`
}

const modelDetailsPanelClass =
  'w-80 max-w-[min(24rem,calc(100vw-2rem))] rounded-xl border border-gray-200 bg-white p-3 text-left shadow-xl shadow-gray-900/10 dark:border-dark-600 dark:bg-dark-900 dark:shadow-black/40 sm:w-96'

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
          <span className="min-w-0 truncate text-sm font-semibold text-gray-900 dark:text-white">
            {model.display_name || model.id}
          </span>
          <span className="rounded-md bg-gray-100 px-1.5 py-0.5 text-[10px] font-medium text-gray-500 dark:bg-dark-800 dark:text-dark-300">
            {providerLabel}
          </span>
        </div>
        <p className="break-all font-mono text-[11px] text-gray-400 dark:text-dark-400">{model.id}</p>
        {model.description && (
          <p className="text-xs leading-5 text-gray-600 dark:text-dark-200">{model.description}</p>
        )}
      </div>

      {metrics.length > 0 && (
        <div className="mt-3 grid grid-cols-2 gap-2">
          {metrics.map((metric) => (
            <div key={metric.label} className="rounded-lg bg-gray-50 px-2.5 py-2 dark:bg-dark-800">
              <p className="text-[10px] font-medium text-gray-400 dark:text-dark-400">{metric.label}</p>
              <p className="mt-0.5 break-words text-xs font-semibold text-gray-800 dark:text-gray-100">{metric.value}</p>
            </div>
          ))}
        </div>
      )}

      {methods.length > 0 && (
        <div className="mt-3">
          <p className="text-[10px] font-medium text-gray-400 dark:text-dark-400">{methodsLabel}</p>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {methods.slice(0, 6).map((method) => (
              <span key={method} className="rounded-md bg-primary-50 px-1.5 py-0.5 text-[10px] font-medium text-primary-700 dark:bg-primary-950/30 dark:text-primary-300">
                {method}
              </span>
            ))}
            {methods.length > 6 && (
              <span className="rounded-md bg-gray-100 px-1.5 py-0.5 text-[10px] font-medium text-gray-500 dark:bg-dark-800 dark:text-dark-300">
                +{methods.length - 6}
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function ModelDetailsTooltipPortal({
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
    window.addEventListener('scroll', update, true)
    window.addEventListener('resize', update)
    return () => {
      window.removeEventListener('scroll', update, true)
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

interface EditablePriceCellProps {
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

function EditablePriceCell({
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
      <div className="flex items-center gap-1.5 rounded-lg bg-primary-50 dark:bg-primary-950/20 ring-2 ring-primary-400/50 px-2.5 py-1.5 transition-all">
        {icon}
        <div className="min-w-0 flex-1">
          <p className="text-[10px] text-primary-500 dark:text-primary-400 leading-none font-medium">{label} /1M</p>
          <div className="flex items-center gap-1 mt-0.5">
            <span className="text-xs text-gray-400 select-none">$</span>
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
              className="w-full bg-transparent text-xs font-semibold text-gray-900 dark:text-white outline-none placeholder:text-gray-300 [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none"
              placeholder="0.00"
            />
            {updatePrice.isPending && <Loader2 className="h-3 w-3 animate-spin text-primary-500 flex-shrink-0" />}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div
      className={`flex items-center gap-1.5 rounded-lg bg-gray-50 dark:bg-dark-800 px-2.5 py-2 transition-all ${
        isAdmin
          ? 'cursor-pointer hover:bg-primary-50/60 dark:hover:bg-primary-950/10 hover:ring-1 hover:ring-primary-300/50 dark:hover:ring-primary-700/40 group/price'
          : ''
      }`}
      onClick={startEditing}
      title={isAdmin ? '点击编辑价格' : undefined}
    >
      {icon}
      <div className="min-w-0 flex-1">
        <p className="text-[10px] text-gray-400 dark:text-dark-500 leading-none">{label} /1M</p>
        <p className="text-xs font-semibold text-gray-700 dark:text-gray-200 mt-0.5">{formatPrice(price)}</p>
      </div>
      {isAdmin && (
        <Pencil className="h-3 w-3 text-gray-300 dark:text-dark-600 opacity-0 group-hover/price:opacity-100 transition-opacity flex-shrink-0" />
      )}
    </div>
  )
}

export interface ModelCatalogCardProps {
  model: ModelCatalogItem
  isAdmin?: boolean
  copied?: boolean
  onCopy?: (modelId: string) => void
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
  onPriceSaved,
  showPricing = true,
  showCopy = true,
  showInlineMetrics = false,
  sourceBadges = [],
}: ModelCatalogCardProps) {
  const provider = getModelProviderKey(model)
  const providerLabel = getProviderDisplayName(provider)
  const style = getProviderStyle(provider)
  const hasReasoning = isAdmin || (model.reasoning_price_per_1m ?? 0) > 0
  const hasCached = isAdmin || (model.cached_input_price_per_1m ?? 0) > 0
  const showDetails = hasModelDetails(model)
  const inlineMetrics = showInlineMetrics ? getModelDetailMetrics(model).filter((metric) => metric.label !== '类型').slice(0, 2) : []
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
    <div className="group relative rounded-xl border border-border bg-card p-4 transition-all hover:border-gray-300 dark:hover:border-dark-600">
      <div className="mb-3 flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="mb-1 flex items-center gap-2">
            <span className={`inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-semibold uppercase ${style.bg} ${style.text}`}>
              {providerLabel}
            </span>
            {sourceBadges.map((source) => (
              <span
                key={source}
                className="inline-flex items-center rounded-md bg-gray-100 px-1.5 py-0.5 text-[10px] font-medium text-gray-500 dark:bg-dark-800 dark:text-dark-300"
              >
                {source}
              </span>
            ))}
          </div>
          <h3 className="truncate text-sm font-semibold text-gray-900 dark:text-white" title={model.id}>
            {model.id}
          </h3>
        </div>
        <div className="flex flex-shrink-0 items-center gap-1">
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
                className="flex h-7 w-7 items-center justify-center rounded-md text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600 focus:bg-gray-100 focus:text-gray-700 focus:outline-none focus:ring-2 focus:ring-primary-400/30 dark:hover:bg-dark-700 dark:hover:text-gray-200 dark:focus:bg-dark-700"
                aria-label={`查看 ${model.id} 模型详情`}
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
              onClick={handleCopy}
              className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-dark-700 dark:hover:text-gray-200"
              title="复制模型 ID"
            >
              {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
            </button>
          )}
        </div>
      </div>

      {inlineMetrics.length > 0 && (
        <div className="mb-3 grid grid-cols-2 gap-2">
          {inlineMetrics.map((metric) => (
            <div key={metric.label} className="rounded-lg bg-gray-50 px-2.5 py-2 dark:bg-dark-800">
              <p className="text-[10px] text-gray-400 dark:text-dark-500 leading-none">{metric.label}</p>
              <p className="mt-0.5 truncate text-xs font-semibold text-gray-700 dark:text-gray-200" title={metric.value}>{metric.value}</p>
            </div>
          ))}
        </div>
      )}

      {showPricing && (
        <div className="grid grid-cols-2 gap-2">
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
