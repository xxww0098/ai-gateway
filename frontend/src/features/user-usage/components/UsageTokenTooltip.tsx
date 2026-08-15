import type { UsageTokenTooltipProps } from '../types'

export function UsageTokenTooltip({ data, onClose }: UsageTokenTooltipProps) {
  const { log } = data
  const total = log.input_tokens + log.output_tokens + log.reasoning_tokens + log.cached_tokens

  return (
    <div
      className="fixed z-[9999] pointer-events-none"
      style={{ left: data.x + 12, top: data.y - 20 }}
      onClick={onClose}
    >
      <div className="whitespace-nowrap rounded-xl border border-gray-700/80 bg-gray-900/95 backdrop-blur-xl px-4 py-3 text-xs text-white shadow-2xl min-w-[220px]" onClick={onClose}>
        <div className="text-[10px] font-bold uppercase tracking-wider text-gray-400 mb-2">Token 明细</div>
        <div className="space-y-1.5">
          <div className="flex justify-between gap-6">
            <span className="text-gray-400">输入 Tokens</span>
            <span className="font-medium text-emerald-300 tabular-nums">{log.input_tokens.toLocaleString()}</span>
          </div>
          <div className="flex justify-between gap-6">
            <span className="text-gray-400">输出 Tokens</span>
            <span className="font-medium text-violet-300 tabular-nums">{log.output_tokens.toLocaleString()}</span>
          </div>
          {log.reasoning_tokens > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">推理 Tokens</span>
              <span className="font-medium text-amber-300 tabular-nums">{log.reasoning_tokens.toLocaleString()}</span>
            </div>
          )}
          {log.cached_tokens > 0 && (
            <div className="flex justify-between gap-6">
              <span className="text-gray-400">缓存 Tokens</span>
              <span className="font-medium text-sky-300 tabular-nums">{log.cached_tokens.toLocaleString()}</span>
            </div>
          )}
          <div className="flex justify-between gap-6 border-t border-gray-700 pt-1.5">
            <span className="text-gray-400">总计</span>
            <span className="font-bold text-blue-400 tabular-nums">{total.toLocaleString()}</span>
          </div>
        </div>
      </div>
    </div>
  )
}
