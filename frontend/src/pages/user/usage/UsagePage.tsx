import { lazy, Suspense, useState, useCallback, useMemo } from 'react'
import { toast } from 'sonner'
import type { UsageLog } from '@/features/user-usage/types'
import { UsageStatsCards } from '@/features/user-usage/components/UsageStatsCards'
import { UsageFilterBar } from '@/features/user-usage/components/UsageFilterBar'
import { UsageTable } from '@/features/user-usage/components/UsageTable'

const UsageCharts = lazy(() =>
  import('@/features/user-usage/components/UsageCharts').then(m => ({
    default: m.UsageCharts,
  })),
)
import { useUsageLogs, useUserApiKeys } from '@/features/user-usage/hooks'
import { fetchUsageLogs } from '@/features/user-usage/api'
import { fmtDateTime } from '@/features/user-usage/format'

function rangeLabel(
  dateRange: 'today' | '7d' | '30d' | 'custom',
  startDate: string,
  endDate: string,
): string {
  if (dateRange === 'today') return startDate
  return `${startDate} → ${endDate}`
}

export default function Usage() {
  const {
    logs,
    stats,
    total,
    loading,
    trend,
    models,
    chartsLoading,
    page,
    pageSize,
    totalPages,
    filterKeyId,
    setFilterKeyId,
    filterModel,
    appliedModel,
    setFilterModel,
    dateRange,
    handleDateRangeChange,
    startDate,
    setStartDate,
    endDate,
    setEndDate,
    handleFilter,
    handlePageChange,
    handlePageSizeChange,
    refresh,
    getEffectiveDates,
  } = useUsageLogs()

  const { apiKeys } = useUserApiKeys()
  const [exporting, setExporting] = useState(false)
  const [updatedAt] = useState(() => new Date())

  const dates = getEffectiveDates()
  const topModel = useMemo(() => {
    if (!models.length) return null
    return [...models].sort((a, b) => b.requests - a.requests)[0] ?? null
  }, [models])

  const handleExport = useCallback(async () => {
    if (total === 0) { toast.warning('当前筛选条件下没有数据可导出'); return }
    setExporting(true)
    toast.info('正在准备导出...')
    try {
      const allLogs: UsageLog[] = []
      const ps = 100
      const pages = Math.ceil(total / ps)
      const exportDates = getEffectiveDates()
      for (let p = 1; p <= pages; p++) {
        const res = await fetchUsageLogs({
          page: p,
          pageSize: ps,
          apiKeyId: filterKeyId || undefined,
          model: appliedModel.trim() || undefined,
          startDate: exportDates.startDate,
          endDate: exportDates.endDate,
        })
        allLogs.push(...res.items)
      }

      const header = '时间,模型,API Key,类型,输入Tokens,输出Tokens,推理Tokens,缓存Tokens,标准费用,实际扣费,倍率,耗时(ms),状态\n'
      const rows = allLogs.map(l =>
        [
          fmtDateTime(l.created_at),
          l.model,
          l.api_key_name || '-',
          l.stream ? 'Stream' : 'Sync',
          l.input_tokens,
          l.output_tokens,
          l.reasoning_tokens,
          l.cached_tokens,
          l.total_cost.toFixed(6),
          l.actual_cost.toFixed(6),
          l.rate_multiplier,
          l.duration_ms,
          l.failed ? '失败' : '成功',
        ].join(',')
      ).join('\n')

      const blob = new Blob([`\uFEFF${header}${rows}`], { type: 'text/csv;charset=utf-8;' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `usage_${exportDates.startDate}_${exportDates.endDate}.csv`
      a.click()
      URL.revokeObjectURL(url)
      toast.success(`导出成功，共 ${allLogs.length} 条记录`)
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '导出失败')
    } finally {
      setExporting(false)
    }
  }, [total, filterKeyId, appliedModel, getEffectiveDates])

  return (
    <div className="usage-paper space-y-3.5">
      {/*
        THESIS: 用量页是一张分析纸：先看消耗与扣费，再看趋势和模型占比，明细在折页下。拒绝图标四宫格加一张表。
        OWN-WORLD: 暖纸卡 13px、发丝边、大号 tabular 数字、青是动作色、紫只作费用虚线。
        STORY: 租户打开就知道这段时间花了多少 token、扣了多少钱、谁吃得最多，并能导出或点进一行。
        FIRST VIEWPORT: 标题、筛选、四张 KPI、左 2/3 趋势 + 右 1/3 环图；刷新在筛选右。
        FORM: 用户钉死的 CAP 分析纸分块，构图 classic-paper。
        FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
      */}
      <UsageFilterBar
        apiKeys={apiKeys}
        filterKeyId={filterKeyId}
        onFilterKeyIdChange={setFilterKeyId}
        filterModel={filterModel}
        onFilterModelChange={setFilterModel}
        dateRange={dateRange}
        onDateRangeChange={handleDateRangeChange}
        startDate={startDate}
        onStartDateChange={setStartDate}
        endDate={endDate}
        onEndDateChange={setEndDate}
        onFilter={handleFilter}
        onRefresh={refresh}
        onExport={() => { void handleExport() }}
        loading={loading}
        exporting={exporting}
        total={total}
      />

      <p className="flex min-h-5 items-center gap-2 text-xs text-muted-foreground">
        <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/50" aria-hidden />
        范围 {rangeLabel(dateRange, dates.startDate, dates.endDate)}
        <span>·</span>
        按日聚合
        <span>·</span>
        数据更新于 {updatedAt.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })}
      </p>

      <UsageStatsCards stats={stats} topModel={topModel} />
      <Suspense
        fallback={
          <div
            className="h-[360px] animate-pulse rounded-xl bg-muted/60"
            role="status"
            aria-busy="true"
            aria-label="图表加载中"
          />
        }
      >
        <UsageCharts trend={trend} models={models} loading={chartsLoading} />
      </Suspense>
      <UsageTable
        logs={logs}
        loading={loading}
        total={total}
        page={page}
        pageSize={pageSize}
        totalPages={totalPages}
        onPageChange={handlePageChange}
        onPageSizeChange={handlePageSizeChange}
      />
    </div>
  )
}
