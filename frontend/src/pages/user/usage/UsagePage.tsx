import { useState, useRef, useCallback } from 'react'
import { toast } from 'sonner'
import type { TooltipData, UsageLog } from '@/features/user-usage/types'
import { UsageStatsCards } from '@/features/user-usage/components/UsageStatsCards'
import { UsageFilterBar } from '@/features/user-usage/components/UsageFilterBar'
import { UsageTable } from '@/features/user-usage/components/UsageTable'
import { UsageCostTooltip } from '@/features/user-usage/components/UsageCostTooltip'
import { UsageTokenTooltip } from '@/features/user-usage/components/UsageTokenTooltip'
import { useUsageLogs, useUserApiKeys } from '@/features/user-usage/hooks'
import { fetchUsageLogs } from '@/features/user-usage/api'

// ── Helpers ─────────────────────────────────────────────────────────────────

function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

// ── Page Component ──────────────────────────────────────────────────────────

export default function Usage() {
  const {
    logs,
    stats,
    total,
    loading,
    page,
    pageSize,
    totalPages,
    filterKeyId,
    setFilterKeyId,
    filterModel,
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

  // Export state
  const [exporting, setExporting] = useState(false)

  // Tooltips
  const [costTooltip, setCostTooltip] = useState<TooltipData | null>(null)
  const [tokenTooltip, setTokenTooltip] = useState<TooltipData | null>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  // CSV Export
  const handleExport = useCallback(async () => {
    if (total === 0) { toast.warning('当前筛选条件下没有数据可导出'); return }
    setExporting(true)
    toast.info('正在准备导出...')
    try {
      const allLogs: UsageLog[] = []
      const ps = 100
      const pages = Math.ceil(total / ps)
      const dates = getEffectiveDates()
      for (let p = 1; p <= pages; p++) {
        const res = await fetchUsageLogs({
          page: p,
          pageSize: ps,
          apiKeyId: filterKeyId || undefined,
          model: filterModel.trim() || undefined,
          startDate: dates.startDate,
          endDate: dates.endDate,
        })
        if (res?.items) allLogs.push(...res.items)
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
          l.duration_ms ?? '',
          l.failed ? '失败' : '成功',
        ].join(',')
      ).join('\n')

      const BOM = '\uFEFF'
      const blob = new Blob([BOM + header + rows], { type: 'text/csv;charset=utf-8;' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `usage_${dates.startDate}_${dates.endDate}.csv`
      a.click()
      URL.revokeObjectURL(url)
      toast.success(`导出成功，共 ${allLogs.length} 条记录`)
    } catch (err) {
      toast.error(err instanceof Error ? err.message : '导出失败')
    } finally {
      setExporting(false)
    }
  }, [total, filterKeyId, filterModel, getEffectiveDates])

  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500" style={{ willChange: 'transform, opacity' }} ref={containerRef}>
      {/* Header */}
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">使用明细</h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1">查看每一次 API 调用的详细信息、Token 用量及费用。</p>
      </div>

      <UsageStatsCards stats={stats} />

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
        onExport={handleExport}
        loading={loading}
        exporting={exporting}
        total={total}
      />

      <UsageTable
        logs={logs}
        loading={loading}
        total={total}
        page={page}
        pageSize={pageSize}
        totalPages={totalPages}
        onPageChange={handlePageChange}
        onPageSizeChange={handlePageSizeChange}
        onCostTooltip={setCostTooltip}
        onTokenTooltip={setTokenTooltip}
      />

      {costTooltip && <UsageCostTooltip data={costTooltip} onClose={() => setCostTooltip(null)} />}
      {tokenTooltip && <UsageTokenTooltip data={tokenTooltip} onClose={() => setTokenTooltip(null)} />}
    </div>
  )
}
