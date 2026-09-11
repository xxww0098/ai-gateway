import { TrendingUp, Flame, Cpu } from 'lucide-react'
import {
  AreaChart, Area, XAxis, YAxis, CartesianGrid, Tooltip as RechartsTooltip,
  ResponsiveContainer, PieChart, Pie
} from 'recharts'
import { Button } from '@/shared/components/ui/button'
import { userRoutes } from '@/shared/routes/user'
import type { UserDashboardChartsProps, ModelStat } from '../types'

const CHART_COLORS = [
  '#0d9488', '#716dff', '#0284c7', '#d97706', '#e11d48',
  '#059669', '#7c3aed', '#ea580c', '#0891b2', '#4f46e5',
]

function fmtCost(n: number): string {
  if (n === 0) return '$0.00'
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(2)}`
}

function CustomAreaTooltip({ active, payload, label }: { active?: boolean; payload?: Array<{ value: number; dataKey: string }>; label?: string }) {
  if (!active || !payload?.length) return null
  return (
    <div className="rounded-[8px] border border-border bg-popover px-3 py-2.5 shadow-md text-xs text-popover-foreground min-w-[140px]">
      <p className="font-semibold text-foreground mb-1.5">{label}</p>
      <div className="space-y-1">
        {payload.map((p, i) => (
          <div key={i} className="flex items-center justify-between gap-3">
            <span className="flex items-center gap-1.5 text-muted-foreground">
              <span className="size-1.5 rounded-[2px]" style={{ backgroundColor: p.dataKey === 'requests' ? '#0d9488' : '#716dff' }} />
              {p.dataKey === 'requests' ? '请求数' : '费用'}
            </span>
            <span className="font-medium text-foreground tabular-nums">
              {p.dataKey === 'cost' ? `$${p.value.toFixed(4)}` : p.value.toLocaleString()}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}

function CustomDoughnutTooltip({ active, payload }: { active?: boolean; payload?: Array<{ name: string; value: number; payload: ModelStat }> }) {
  if (!active || !payload?.length) return null
  const data = payload[0]
  return (
    <div className="rounded-[8px] border border-border bg-popover px-3.5 py-2.5 shadow-md text-xs text-popover-foreground min-w-[180px]">
      <p className="font-semibold text-foreground mb-1.5 truncate max-w-[200px] font-mono text-[11px]">{data.name}</p>
      <div className="space-y-1">
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground">请求数</span>
          <span className="font-medium text-foreground tabular-nums">{data.payload.requests.toLocaleString()} 次</span>
        </div>
        <div className="flex justify-between gap-4">
          <span className="text-muted-foreground">Tokens</span>
          <span className="font-medium text-foreground tabular-nums">{(data.payload.tokens / 1000).toFixed(1)}K</span>
        </div>
        <div className="flex justify-between gap-4 border-t border-border pt-1">
          <span className="text-muted-foreground">费用</span>
          <span className="font-semibold text-foreground tabular-nums">{fmtCost(data.payload.cost)}</span>
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
  const totalModelCost = modelData.reduce((sum, m) => sum + m.cost, 0)

  return (
    <div className="grid gap-4 lg:grid-cols-3">
      {/* Usage Trend (2 cols) */}
      <div className="overflow-hidden rounded-[13px] border border-border bg-card shadow-[0_1px_2px_rgb(0_0_0/0.07)] lg:col-span-2 flex flex-col justify-between">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-3">
          <div className="flex items-center gap-2">
            <span className="size-1.5 rounded-[2px] bg-teal-600 dark:bg-teal-400" aria-hidden />
            <div>
              <h3 className="text-xs font-semibold text-foreground">用量趋势</h3>
              <p className="text-[11px] text-muted-foreground">按日聚合查看请求频次与费用扣除</p>
            </div>
          </div>
          <div className="metric-switch">
            {([7, 30] as const).map(d => (
              <button
                key={d}
                type="button"
                onClick={() => onTrendDaysChange(d)}
                className={`metric-switch-button ${trendDays === d ? 'is-on' : ''}`}
              >
                {d} 天
              </button>
            ))}
          </div>
        </div>

        <div className="p-4 flex-1 flex flex-col justify-center">
          {trendData.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-10 text-center">
              <div className="flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground mb-3">
                <TrendingUp className="size-5" />
              </div>
              <h4 className="text-xs font-semibold text-foreground">最近 {trendDays} 天还没有调用</h4>
              <p className="mt-1 max-w-xs text-[11px] text-muted-foreground">
                可调整时间范围查看历史数据，或发起 API 请求生成图表。
              </p>
              <Button asChild size="sm" variant="outline" className="mt-3.5 h-7 text-xs rounded-[7px]">
                <a href={userRoutes.keys} className="inline-flex items-center gap-1">
                  获取 API 密钥
                </a>
              </Button>
            </div>
          ) : (
            <>
              <ResponsiveContainer width="100%" height={240}>
                <AreaChart data={trendData} margin={{ top: 10, right: 10, left: -15, bottom: 0 }}>
                  <defs>
                    <linearGradient id="userReqGrad" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#0d9488" stopOpacity={0.18} />
                      <stop offset="95%" stopColor="#0d9488" stopOpacity={0.0} />
                    </linearGradient>
                    <linearGradient id="userCostGrad" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#716dff" stopOpacity={0.18} />
                      <stop offset="95%" stopColor="#716dff" stopOpacity={0.0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" opacity={0.6} />
                  <XAxis dataKey="date" tick={{ fontSize: 10, fill: 'hsl(var(--muted-foreground))' }} axisLine={false} tickLine={false} />
                  <YAxis yAxisId="left" tick={{ fontSize: 10, fill: 'hsl(var(--muted-foreground))' }} axisLine={false} tickLine={false} width={40} />
                  <YAxis yAxisId="right" orientation="right" tick={{ fontSize: 10, fill: 'hsl(var(--muted-foreground))' }} axisLine={false} tickLine={false} width={48} tickFormatter={(v: number) => `$${v.toFixed(2)}`} />
                  <RechartsTooltip content={<CustomAreaTooltip />} />
                  <Area yAxisId="left" type="monotone" dataKey="requests" stroke="#0d9488" fill="url(#userReqGrad)" strokeWidth={2} dot={false} activeDot={{ r: 4, strokeWidth: 0 }} />
                  <Area yAxisId="right" type="monotone" dataKey="cost" stroke="#716dff" fill="url(#userCostGrad)" strokeWidth={2} dot={false} activeDot={{ r: 4, strokeWidth: 0 }} />
                </AreaChart>
              </ResponsiveContainer>

              <div className="mt-3 flex items-center justify-center gap-5 border-t border-border/50 pt-2.5 text-[11px] text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <span className="size-2 rounded-[2px] bg-[#0d9488]" />
                  请求数 (次)
                </span>
                <span className="inline-flex items-center gap-1.5">
                  <span className="size-2 rounded-[2px] bg-[#716dff]" />
                  费用 (USD)
                </span>
              </div>
            </>
          )}
        </div>
      </div>

      {/* Model Distribution (1 col) */}
      <div className="overflow-hidden rounded-[13px] border border-border bg-card shadow-[0_1px_2px_rgb(0_0_0/0.07)] flex flex-col justify-between">
        <div className="flex items-center justify-between border-b border-border px-4 py-3">
          <div className="flex items-center gap-2">
            <span className="size-1.5 rounded-[2px] bg-amber-600 dark:bg-amber-400" aria-hidden />
            <div>
              <h3 className="text-xs font-semibold text-foreground">模型用量分布</h3>
              <p className="text-[11px] text-muted-foreground">调用频次与费用结构占比</p>
            </div>
          </div>
        </div>

        <div className="p-4 flex-1 flex flex-col justify-center">
          {modelData.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-10 text-center">
              <div className="flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground mb-3">
                <Flame className="size-5" />
              </div>
              <h4 className="text-xs font-semibold text-foreground">还没有模型用量</h4>
              <p className="mt-1 max-w-xs text-[11px] text-muted-foreground">
                发起 API 请求后，此处将按模型维度统计调用次数与费用分布。
              </p>
              <Button asChild size="sm" variant="outline" className="mt-3.5 h-7 text-xs rounded-[7px]">
                <a href={userRoutes.models} className="inline-flex items-center gap-1">
                  <Cpu className="size-3" />
                  查看可用模型
                </a>
              </Button>
            </div>
          ) : (
            <div className="flex flex-col items-center gap-4">
              <div className="relative size-[160px]">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie
                      data={modelData.slice(0, 8).map((entry, index) => ({
                        ...entry,
                        fill: CHART_COLORS[index % CHART_COLORS.length],
                      }))}
                      dataKey="cost"
                      nameKey="model"
                      cx="50%"
                      cy="50%"
                      innerRadius={48}
                      outerRadius={74}
                      paddingAngle={2}
                      strokeWidth={0}
                    />
                    <RechartsTooltip content={<CustomDoughnutTooltip />} />
                  </PieChart>
                </ResponsiveContainer>
                <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
                  <span className="text-base font-bold tabular-nums text-foreground">{fmtCost(totalModelCost)}</span>
                  <span className="text-[10px] text-muted-foreground">总费用</span>
                </div>
              </div>

              <div className="w-full max-h-[140px] overflow-y-auto space-y-1.5 px-0.5">
                {modelData.slice(0, 8).map((m, i) => {
                  const percent = totalModelCost > 0 ? ((m.cost / totalModelCost) * 100).toFixed(0) : '0'
                  return (
                    <div
                      key={m.model}
                      className="flex items-center justify-between text-[11px] rounded-[6px] px-2 py-1 hover:bg-muted/40 transition-colors"
                    >
                      <div className="flex items-center gap-2 min-w-0 flex-1">
                        <div
                          className="size-2 rounded-[2px] shrink-0"
                          style={{ backgroundColor: CHART_COLORS[i % CHART_COLORS.length] }}
                        />
                        <span className="truncate font-mono text-[11px] text-foreground" title={m.model}>
                          {m.model}
                        </span>
                      </div>
                      <div className="flex items-center gap-3 shrink-0 ml-2">
                        <span className="text-muted-foreground tabular-nums">{percent}%</span>
                        <span className="text-foreground font-semibold tabular-nums w-14 text-right">
                          {fmtCost(m.cost)}
                        </span>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
