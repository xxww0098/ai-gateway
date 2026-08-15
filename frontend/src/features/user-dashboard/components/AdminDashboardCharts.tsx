import {
  TrendingUp, Flame
} from 'lucide-react'
import {
  AreaChart, Area, XAxis, YAxis, CartesianGrid, Tooltip as RechartsTooltip,
  ResponsiveContainer, PieChart, Pie
} from 'recharts'
import type { AdminDashboardChartsProps, ModelStat } from '../types'

const CHART_COLORS = [
  '#14b8a6', '#3b82f6', '#8b5cf6', '#f59e0b', '#ef4444',
  '#ec4899', '#06b6d4', '#84cc16', '#f97316', '#6366f1'
]

function fmtCost(n: number): string {
  if (n === 0) return '$0.00'
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(2)}`
}

function CustomAreaTooltip({ active, payload, label }: { active?: boolean; payload?: Array<{ value: number; dataKey: string }>; label?: string }) {
  if (!active || !payload?.length) return null
  return (
    <div className="rounded-xl border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 px-4 py-3 shadow-xl text-xs">
      <p className="font-semibold text-gray-900 dark:text-white mb-1.5">{label}</p>
      {payload.map((p, i) => (
        <div key={i} className="flex items-center gap-2">
          <div className="w-2 h-2 rounded-full" style={{ backgroundColor: p.dataKey === 'requests' ? '#14b8a6' : '#3b82f6' }} />
          <span className="text-gray-500">{p.dataKey === 'requests' ? '请求' : '费用'}</span>
          <span className="font-medium text-gray-900 dark:text-white tabular-nums ml-auto">
            {p.dataKey === 'cost' ? `$${p.value.toFixed(4)}` : p.value}
          </span>
        </div>
      ))}
    </div>
  )
}

const cellClassName = "transition-opacity duration-200 hover:opacity-80 cursor-pointer"

function CustomDoughnutTooltip({ active, payload }: { active?: boolean; payload?: Array<{ name: string; value: number; payload: ModelStat }> }) {
  if (!active || !payload?.length) return null
  const data = payload[0]
  return (
    <div className="rounded-xl border border-gray-700/80 bg-gray-900/95 backdrop-blur-xl px-4 py-3 shadow-2xl text-xs text-white min-w-[180px]">
      <p className="font-semibold text-white mb-1.5 truncate max-w-[200px]">{data.name}</p>
      <div className="space-y-1">
        <div className="flex justify-between gap-4">
          <span className="text-gray-400">请求数</span>
          <span className="font-medium text-white tabular-nums">{data.payload.requests} 次</span>
        </div>
        <div className="flex justify-between gap-4">
          <span className="text-gray-400">Tokens</span>
          <span className="font-medium text-white tabular-nums">{(data.payload.tokens / 1000).toFixed(1)}K</span>
        </div>
        <div className="flex justify-between gap-4 border-t border-gray-700 pt-1">
          <span className="text-gray-400">费用</span>
          <span className="font-semibold text-green-400 tabular-nums">{fmtCost(data.payload.cost)}</span>
        </div>
      </div>
    </div>
  )
}

export function AdminDashboardCharts({
  trendData,
  modelData,
  trendDays,
  onTrendDaysChange,
}: AdminDashboardChartsProps) {
  return (
    <div className="grid gap-6 lg:grid-cols-3 mt-6">
      {/* Trend Chart (2 cols) */}
      <div className="glass-card overflow-hidden lg:col-span-2">
        <div className="px-6 py-5 border-b border-border/50 flex items-center justify-between bg-gray-50/50 dark:bg-dark-800/50">
          <div className="flex items-center gap-2">
            <TrendingUp className="w-5 h-5 text-primary-500" />
            <h3 className="text-sm font-bold uppercase tracking-wider text-gray-900 dark:text-white">
              使用趋势
            </h3>
          </div>
          <div className="flex gap-1">
            {([7, 30] as const).map(d => (
              <button key={d} onClick={() => onTrendDaysChange(d)}
                className={`px-3 py-1 rounded-full text-xs font-medium transition-all ${
                  trendDays === d
                    ? 'bg-primary-500 text-white shadow-sm'
                    : 'text-gray-500 dark:text-dark-400 hover:bg-gray-100 dark:hover:bg-dark-800'
                }`}>
                {d}天
              </button>
            ))}
          </div>
        </div>
        <div className="p-4">
          {trendData.length === 0 ? (
            <div className="flex items-center justify-center h-[220px] text-gray-400 dark:text-dark-500 text-sm">暂无趋势数据</div>
          ) : (
            <ResponsiveContainer width="100%" height={220}>
              <AreaChart data={trendData} margin={{ top: 5, right: 10, left: 0, bottom: 0 }}>
                <defs>
                  <linearGradient id="adminReqGrad" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="#14b8a6" stopOpacity={0.3} />
                    <stop offset="100%" stopColor="#14b8a6" stopOpacity={0} />
                  </linearGradient>
                  <linearGradient id="adminCostGrad" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="#3b82f6" stopOpacity={0.3} />
                    <stop offset="100%" stopColor="#3b82f6" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" opacity={0.3} />
                <XAxis dataKey="date" tick={{ fontSize: 11, fill: '#9ca3af' }} axisLine={false} tickLine={false} />
                <YAxis yAxisId="left" tick={{ fontSize: 11, fill: '#9ca3af' }} axisLine={false} tickLine={false} width={40} />
                <YAxis yAxisId="right" orientation="right" tick={{ fontSize: 11, fill: '#9ca3af' }} axisLine={false} tickLine={false} width={50} tickFormatter={(v: number) => `$${v.toFixed(2)}`} />
                <RechartsTooltip content={<CustomAreaTooltip />} />
                <Area yAxisId="left" type="monotone" dataKey="requests" stroke="#14b8a6" fill="url(#adminReqGrad)" strokeWidth={2} dot={false} activeDot={{ r: 4, strokeWidth: 0 }} />
                <Area yAxisId="right" type="monotone" dataKey="cost" stroke="#3b82f6" fill="url(#adminCostGrad)" strokeWidth={2} dot={false} activeDot={{ r: 4, strokeWidth: 0 }} />
              </AreaChart>
            </ResponsiveContainer>
          )}
          <div className="flex items-center justify-center gap-6 mt-3 text-xs text-gray-500 dark:text-dark-400">
            <span className="flex items-center gap-1.5"><span className="w-3 h-0.5 rounded-full bg-primary-500 inline-block" /> 请求数</span>
            <span className="flex items-center gap-1.5"><span className="w-3 h-0.5 rounded-full bg-blue-500 inline-block" /> 费用 ($)</span>
          </div>
        </div>
      </div>

      {/* Model Distribution Doughnut (1 col) */}
      <div className="glass-card overflow-hidden">
        <div className="px-6 py-5 border-b border-border/50 flex items-center gap-2 bg-gray-50/50 dark:bg-dark-800/50">
          <Flame className="w-5 h-5 text-amber-500" />
          <h3 className="text-sm font-bold uppercase tracking-wider text-gray-900 dark:text-white">模型分布</h3>
        </div>
        <div className="p-4">
          {modelData.length === 0 ? (
            <div className="flex items-center justify-center h-[260px] text-gray-400 dark:text-dark-500 text-sm">暂无模型数据</div>
          ) : (
            <div className="flex flex-col items-center gap-4">
              <div className="relative w-[180px] h-[180px]">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie
                      data={modelData.slice(0, 8).map((entry, index) => ({
                        ...entry,
                        fill: CHART_COLORS[index % CHART_COLORS.length],
                        className: cellClassName,
                      }))}
                      dataKey="cost"
                      nameKey="model"
                      cx="50%"
                      cy="50%"
                      innerRadius={52}
                      outerRadius={80}
                      paddingAngle={2}
                      strokeWidth={0}
                    />
                    <RechartsTooltip content={<CustomDoughnutTooltip />} />
                  </PieChart>
                </ResponsiveContainer>
                <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
                  <span className="text-lg font-bold tabular-nums text-gray-900 dark:text-white">{fmtCost(modelData.reduce((sum, m) => sum + m.cost, 0))}</span>
                  <span className="text-[10px] text-gray-400 dark:text-dark-500 mt-0.5">总费用</span>
                </div>
              </div>
              <div className="w-full max-h-[120px] overflow-y-auto space-y-1 px-1">
                {modelData.slice(0, 8).map((m, i) => (
                  <div key={m.model} className="flex items-center justify-between text-[11px] group hover:bg-gray-50 dark:hover:bg-dark-800/50 rounded-md px-1.5 py-0.5 transition-colors">
                    <div className="flex items-center gap-1.5 min-w-0 flex-1">
                      <div className="w-2.5 h-2.5 rounded-full flex-shrink-0 ring-1 ring-white/20" style={{ backgroundColor: CHART_COLORS[i % CHART_COLORS.length] }} />
                      <span className="text-gray-600 dark:text-gray-400 truncate" title={m.model}>{m.model}</span>
                    </div>
                    <div className="flex items-center gap-3 flex-shrink-0 ml-2">
                      <span className="text-gray-400 dark:text-dark-500 tabular-nums">{m.requests}次</span>
                      <span className="text-gray-900 dark:text-white font-medium tabular-nums w-16 text-right">{fmtCost(m.cost)}</span>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
