import { TrendingUp, Flame } from 'lucide-react'
import {
  AreaChart, Area, XAxis, YAxis, CartesianGrid, Tooltip as RechartsTooltip,
  ResponsiveContainer, PieChart, Pie
} from 'recharts'
import { EmptyState } from '@/shared/components/EmptyState'
import { userRoutes } from '@/shared/routes/user'
import type { UserDashboardChartsProps, ModelStat } from '../types'

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
    <div className="rounded-lg border border-border bg-popover text-popover-foreground px-3.5 py-2.5 shadow-sm text-xs">
      <p className="font-semibold text-foreground mb-1.5">{label}</p>
      {payload.map((p, i) => (
        <div key={i} className="flex items-center gap-2">
          <div className="w-2 h-2 rounded-full" style={{ backgroundColor: p.dataKey === 'requests' ? '#14b8a6' : '#3b82f6' }} />
          <span className="text-muted-foreground">{p.dataKey === 'requests' ? '请求' : '费用'}</span>
          <span className="font-medium text-foreground tabular-nums ml-auto">
            {p.dataKey === 'cost' ? `$${p.value.toFixed(4)}` : p.value}
          </span>
        </div>
      ))}
    </div>
  )
}

const cellClassName = "transition-opacity duration-150 hover:opacity-80 cursor-pointer"

function CustomDoughnutTooltip({ active, payload }: { active?: boolean; payload?: Array<{ name: string; value: number; payload: ModelStat }> }) {
  if (!active || !payload?.length) return null
  const data = payload[0]
  return (
    <div className="rounded-lg border border-border bg-popover text-popover-foreground px-3.5 py-2.5 shadow-sm text-xs min-w-[180px]">
      <p className="font-semibold text-foreground mb-1.5 truncate max-w-[200px]">{data.name}</p>
      <div className="space-y-1">
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground">请求数</span>
          <span className="font-medium text-foreground tabular-nums">{data.payload.requests} 次</span>
        </div>
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground">Tokens</span>
          <span className="font-medium text-foreground tabular-nums">{(data.payload.tokens / 1000).toFixed(1)}K</span>
        </div>
        <div className="flex justify-between gap-4 border-t border-border pt-1">
          <span className="text-muted-foreground">费用</span>
          <span className="font-semibold text-emerald-600 dark:text-emerald-400 tabular-nums">{fmtCost(data.payload.cost)}</span>
        </div>
      </div>
    </div>
  )
}

export function UserDashboardCharts({
  trendData,
  modelData,
  trendDays,
  onTrendDaysChange,
}: UserDashboardChartsProps) {
  return (
    <div className="grid gap-6 lg:grid-cols-3">
      {/* Usage Trend (takes 2 cols) */}
      <div className="rounded-xl border border-border bg-card overflow-hidden lg:col-span-2">
        <div className="px-5 py-4 border-b border-border flex items-center justify-between">
          <div className="flex items-center gap-2">
            <TrendingUp className="w-4 h-4 text-primary" />
            <h3 className="text-sm font-semibold text-foreground">
              用量趋势
            </h3>
          </div>
          <div className="flex gap-1 p-0.5 bg-muted rounded-lg">
            {([7, 30] as const).map(d => (
              <button
                key={d}
                onClick={() => onTrendDaysChange(d)}
                className={`px-2.5 py-1 rounded-md text-xs font-medium transition-all ${
                  trendDays === d
                    ? 'bg-background text-foreground shadow-xs'
                    : 'text-muted-foreground hover:text-foreground'
                }`}
              >
                {d}天
              </button>
            ))}
          </div>
        </div>
        <div className="p-5">
          {trendData.length === 0 ? (
            <EmptyState
              size="compact"
              tone="no-results"
              icon={TrendingUp}
              title={`最近 ${trendDays} 天暂无调用记录`}
              description="可调整时间范围查看历史数据，或发起 API 请求生成图表。"
              className="h-[200px]"
            />
          ) : (
            <ResponsiveContainer width="100%" height={220}>
              <AreaChart data={trendData} margin={{ top: 5, right: 5, left: -20, bottom: 0 }}>
                <CartesianGrid strokeDasharray="3 3" stroke="currentColor" className="text-border/50" />
                <XAxis dataKey="date" tick={{ fontSize: 11, fill: '#9ca3af' }} axisLine={false} tickLine={false} />
                <YAxis yAxisId="left" tick={{ fontSize: 11, fill: '#9ca3af' }} axisLine={false} tickLine={false} />
                <YAxis yAxisId="right" orientation="right" tick={{ fontSize: 11, fill: '#9ca3af' }} axisLine={false} tickLine={false} tickFormatter={(v: number) => `$${v.toFixed(2)}`} />
                <RechartsTooltip content={<CustomAreaTooltip />} />
                <Area yAxisId="left" type="monotone" dataKey="requests" stroke="#14b8a6" strokeWidth={2} fill="#14b8a6" fillOpacity={0.08} dot={false} activeDot={{ r: 4, strokeWidth: 2 }} />
                <Area yAxisId="right" type="monotone" dataKey="cost" stroke="#3b82f6" strokeWidth={2} fill="#3b82f6" fillOpacity={0.08} dot={false} activeDot={{ r: 4, strokeWidth: 2 }} />
              </AreaChart>
            </ResponsiveContainer>
          )}
          <div className="flex items-center justify-center gap-6 mt-3 text-xs text-muted-foreground">
            <span className="flex items-center gap-1.5">
              <span className="w-2.5 h-2.5 rounded-full bg-[#14b8a6] inline-block" /> 请求数
            </span>
            <span className="flex items-center gap-1.5">
              <span className="w-2.5 h-2.5 rounded-full bg-[#3b82f6] inline-block" /> 费用 ($)
            </span>
          </div>
        </div>
      </div>

      {/* Model Distribution (1 col) */}
      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="px-5 py-4 border-b border-border flex items-center gap-2">
          <Flame className="w-4 h-4 text-amber-500" />
          <h3 className="text-sm font-semibold text-foreground">
            模型分布
          </h3>
        </div>
        <div className="p-4">
          {modelData.length === 0 ? (
            <EmptyState
              size="compact"
              tone="no-results"
              icon={Flame}
              title="暂无模型用量数据"
              description="发起 API 请求后，此处将按模型维度统计调用次数与费用分布。"
              action={{ label: '查看可用模型', to: userRoutes.models }}
              className="h-[260px]"
            />
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
                  <span className="text-lg font-bold tabular-nums text-gray-900 dark:text-white">
                    {fmtCost(modelData.reduce((sum, m) => sum + m.cost, 0))}
                  </span>
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
