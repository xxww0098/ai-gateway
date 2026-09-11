import { useMemo, useState } from 'react'
import {
  Bar,
  CartesianGrid,
  ComposedChart,
  Line,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip as RechartsTooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { EmptyState } from '@/shared/components/EmptyState'
import { BarChart3, PieChart as PieIcon } from 'lucide-react'
import type { UsageModelStat, UsageTrendPoint } from '../types'
import { fmtCost, fmtTokens } from '../format'

const DONUT_COLORS = [
  '#0f766e', // Teal (Primary brand)
  '#716dff', // Cost Purple
  '#e59b24', // Amber
  '#0284c7', // Sky Blue
  '#c65746', // Coral Red
  '#059669', // Emerald
  '#d946ef', // Fuchsia
  '#64748b', // Slate
]

type ShareMetric = 'requests' | 'tokens' | 'cost'

const SHARE_TABS: { id: ShareMetric; label: string }[] = [
  { id: 'requests', label: '调用数' },
  { id: 'tokens', label: 'Token' },
  { id: 'cost', label: '费用' },
]

function metricValue(row: UsageModelStat, metric: ShareMetric): number {
  return row[metric]
}

function TrendTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean
  payload?: Array<{ value: number; dataKey: string }>
  label?: string
}) {
  if (!active || !payload?.length) return null
  return (
    <div className="min-w-[180px] rounded-[10px] border border-border bg-popover px-3 py-2.5 text-xs shadow-[0_20px_48px_rgb(0_0_0/0.18)]">
      <p className="mb-1.5 font-semibold text-foreground">{label}</p>
      {payload.map((p) => {
        const isCost = p.dataKey === 'cost'
        return (
          <div key={p.dataKey} className="flex items-center justify-between gap-6 py-0.5 text-muted-foreground">
            <span className="inline-flex items-center gap-1.5">
              <span
                className={`h-2 w-2 rounded-[2px] ${isCost ? 'bg-[#716dff]' : 'bg-[#0f766e]'}`}
                aria-hidden
              />
              {isCost ? '费用' : 'Token'}
            </span>
            <strong className={`tabular-nums ${isCost ? 'text-[#716dff] dark:text-[#a09dff] font-semibold' : 'text-foreground'}`}>
              {isCost ? fmtCost(p.value) : fmtTokens(p.value)}
            </strong>
          </div>
        )
      })}
    </div>
  )
}

export function UsageCharts({
  trend,
  models,
  loading,
}: {
  trend: UsageTrendPoint[]
  models: UsageModelStat[]
  loading: boolean
}) {
  const [shareMetric, setShareMetric] = useState<ShareMetric>('requests')
  const ranked = useMemo(
    () => [...models].sort((a, b) => metricValue(b, shareMetric) - metricValue(a, shareMetric)),
    [models, shareMetric],
  )
  const shareTotal = ranked.reduce((sum, row) => sum + metricValue(row, shareMetric), 0)

  const centerColorClass =
    shareMetric === 'cost'
      ? 'text-[#716dff] dark:text-[#a09dff]'
      : shareMetric === 'tokens'
        ? 'text-[#0f766e] dark:text-[#2dd4bf]'
        : 'text-foreground'

  return (
    <div className="grid gap-3.5 lg:grid-cols-[minmax(0,1.85fr)_minmax(0,0.85fr)]">
      <section className="usage-panel">
        <div className="flex min-h-[58px] items-center justify-between gap-3 px-[18px]">
          <div>
            <h2 className="text-sm font-bold text-foreground">Token 消耗趋势</h2>
            <p className="text-[11px] text-muted-foreground">按日聚合展示调用量与费用趋势</p>
          </div>
          <div className="flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
            <span className="inline-flex items-center gap-1.5">
              <span className="h-2 w-2 rounded-[2px] bg-[#0f766e]" /> Token
            </span>
            <span className="inline-flex items-center gap-1.5">
              <span className="h-0 w-4 border-t-2 border-dashed border-[#716dff]" /> 费用
            </span>
          </div>
        </div>
        <div className="border-t border-border px-2 pb-3 pt-2">
          {loading ? (
            <div className="h-[300px] animate-pulse rounded-lg bg-muted/60" />
          ) : trend.every((p) => p.tokens === 0 && p.cost === 0) ? (
            <EmptyState
              size="compact"
              tone="no-results"
              icon={BarChart3}
              title="这段时间还没有调用"
              description="调整时间范围，或发一条 /v1 请求后再回来看趋势。"
              className="h-[300px]"
            />
          ) : (
            <ResponsiveContainer width="100%" height={300}>
              <ComposedChart data={trend} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                <CartesianGrid strokeDasharray="3 5" stroke="hsl(var(--border))" vertical={false} />
                <XAxis dataKey="date" tick={{ fontSize: 10, fill: '#938c84' }} axisLine={false} tickLine={false} />
                <YAxis yAxisId="tokens" tick={{ fontSize: 10, fill: '#938c84' }} axisLine={false} tickLine={false} tickFormatter={(v: number) => fmtTokens(v)} />
                <YAxis yAxisId="cost" orientation="right" tick={{ fontSize: 10, fill: '#716dff' }} axisLine={false} tickLine={false} tickFormatter={(v: number) => fmtCost(v)} />
                <RechartsTooltip content={<TrendTooltip />} />
                <Bar yAxisId="tokens" dataKey="tokens" fill="#0f766e" radius={[3, 3, 0, 0]} maxBarSize={34} />
                <Line yAxisId="cost" dataKey="cost" stroke="#716dff" strokeWidth={2} strokeDasharray="7 5" dot={false} />
              </ComposedChart>
            </ResponsiveContainer>
          )}
        </div>
      </section>

      <section className="usage-panel">
        <div className="flex min-h-[58px] items-center justify-between gap-2 px-[18px]">
          <h2 className="text-sm font-bold text-foreground">模型调用占比</h2>
          <div className="metric-switch" role="tablist" aria-label="占比口径">
            {SHARE_TABS.map((tab) => (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={shareMetric === tab.id}
                onClick={() => setShareMetric(tab.id)}
                className={`metric-switch-button ${shareMetric === tab.id ? 'is-on' : ''}`}
              >
                {tab.label}
              </button>
            ))}
          </div>
        </div>
        <div className="grid min-h-[330px] grid-cols-1 items-center gap-2 border-t border-border px-3 py-3 sm:grid-cols-[minmax(0,0.95fr)_minmax(0,1.05fr)]">
          {loading ? (
            <div className="col-span-full h-[280px] animate-pulse rounded-lg bg-muted/60" />
          ) : ranked.length === 0 ? (
            <EmptyState
              size="compact"
              tone="first-use"
              icon={PieIcon}
              title="还没有模型用量"
              description="成功调用后，这里会按调用数、Token 或费用拆开。"
              className="col-span-full h-[280px]"
            />
          ) : (
            <>
              <div className="relative mx-auto aspect-square w-full max-w-[220px]">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie
                      data={ranked.map((row, i) => ({
                        ...row,
                        value: metricValue(row, shareMetric),
                        fill: DONUT_COLORS[i % DONUT_COLORS.length],
                      }))}
                      dataKey="value"
                      nameKey="model"
                      cx="50%"
                      cy="50%"
                      innerRadius={58}
                      outerRadius={86}
                      paddingAngle={1.5}
                      strokeWidth={0}
                    />
                  </PieChart>
                </ResponsiveContainer>
                <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
                  <span className={`text-[22px] font-bold tabular-nums leading-none ${centerColorClass}`}>
                    {shareMetric === 'cost' ? fmtCost(shareTotal) : shareMetric === 'tokens' ? fmtTokens(shareTotal) : shareTotal.toLocaleString()}
                  </span>
                  <span className="mt-1 text-[10px] font-medium text-muted-foreground">
                    {SHARE_TABS.find((t) => t.id === shareMetric)?.label}
                  </span>
                </div>
              </div>
              <ul className="max-h-[300px] space-y-0.5 overflow-auto pr-1">
                {ranked.map((row, i) => {
                  const value = metricValue(row, shareMetric)
                  const share = shareTotal ? (value / shareTotal) * 100 : 0
                  return (
                    <li key={row.model} className="grid grid-cols-[10px_minmax(0,1fr)_auto] items-center gap-2 rounded-lg px-1.5 py-1.5 transition-colors hover:bg-muted/50">
                      <span className="h-2.5 w-2.5 rounded-[3px] shrink-0" style={{ backgroundColor: DONUT_COLORS[i % DONUT_COLORS.length] }} />
                      <div className="min-w-0">
                        <p className="truncate text-[11px] font-semibold text-foreground">{row.model}</p>
                        <p className="truncate text-[10px] text-muted-foreground tabular-nums">
                          {row.requests.toLocaleString()} 次 · {fmtTokens(row.tokens)}
                        </p>
                      </div>
                      <span className="text-[10px] font-medium tabular-nums text-foreground/80">{share.toFixed(1)}%</span>
                    </li>
                  )
                })}
              </ul>
            </>
          )}
        </div>
      </section>
    </div>
  )
}
