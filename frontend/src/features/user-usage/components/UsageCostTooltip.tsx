import type { UsageCostTooltipProps } from '../types'

function fmtCost(n: number): string {
  if (n === 0) return '$0.00'
  if (n < 0.0001) return `$${n.toFixed(6)}`
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(4)}`
}

export function UsageCostTooltip({ data, onClose }: UsageCostTooltipProps) {
  const { log } = data
  return (
    <div
      className="fixed z-[9999] pointer-events-none"
      style={{ left: data.x + 12, top: data.y - 20 }}
      onClick={onClose}
    >
      <div className="whitespace-nowrap rounded-xl border border-gray-700/80 bg-gray-900/95 backdrop-blur-xl px-4 py-3 text-xs text-white shadow-2xl min-w-[240px]" onClick={onClose}>
        <div className="text-[10px] font-bold uppercase tracking-wider text-gray-400 mb-2">费用明细</div>
        <div className="space-y-1.5">
          {log.input_cost > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">输入费用</span>
              <span className="font-medium text-emerald-300 tabular-nums">{fmtCost(log.input_cost)}</span>
            </div>
          )}
          {log.output_cost > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">输出费用</span>
              <span className="font-medium text-violet-300 tabular-nums">{fmtCost(log.output_cost)}</span>
            </div>
          )}
          <div className="border-t border-gray-700 pt-1.5">
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">倍率</span>
              <span className="font-semibold text-blue-400 tabular-nums">{log.rate_multiplier}x</span>
            </div>
            <div className="flex justify-between gap-6 mt-1">
              <span className="text-gray-400">标准费用</span>
              <span className="font-medium text-white tabular-nums">{fmtCost(log.total_cost)}</span>
            </div>
            <div className="flex justify-between gap-6 mt-1 border-t border-gray-700 pt-1.5">
              <span className="text-gray-400">实际扣费</span>
              <span className="font-bold text-green-400 tabular-nums">{fmtCost(log.actual_cost)}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
