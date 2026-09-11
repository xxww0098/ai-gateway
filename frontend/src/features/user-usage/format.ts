export function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}m`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return n.toLocaleString()
}

export function fmtTokensExact(n: number): string {
  return n.toLocaleString()
}

export function fmtCost(n: number): string {
  if (n === 0) return 'US$0.00'
  if (n < 0.01) return `US$${n.toFixed(4)}`
  return `US$${n.toFixed(2)}`
}

export function fmtDuration(ms: number | null): string {
  if (ms == null) return '-'
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

export function fmtDateTime(iso: string): string {
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

export function rangeDays(
  dateRange: 'today' | '7d' | '30d' | 'custom',
  startDate: string,
  endDate: string,
): number {
  if (dateRange === 'today') return 1
  if (dateRange === '7d') return 7
  if (dateRange === '30d') return 30
  const start = Date.parse(startDate)
  const end = Date.parse(endDate)
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return 7
  return Math.min(30, Math.max(1, Math.round((end - start) / 86_400_000) + 1))
}
