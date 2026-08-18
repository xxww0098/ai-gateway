// ── Dashboard types ──────────────────────────────────────────────────────────

export interface DashboardStats {
  users: { total: number; active: number }
  api_keys: { total: number; active: number }
  usage: { today_requests: number; today_cost: number; week_requests: number }
  isUser?: boolean
  balance?: number
  quota?: number
  used_quota?: number
}

export interface UsageStats {
  total_requests?: number
  success_count?: number
  total_tokens?: number
  models?: Record<string, { total_requests: number; total_tokens: number; success_count: number }>
}

export interface AnnouncementItem {
  id: number
  title: string
  type: string
}

export interface TrendPoint {
  date: string
  requests: number
  tokens: number
  cost: number
}

export interface ModelStat {
  model: string
  requests: number
  tokens: number
  cost: number
}

export interface RecentUsage {
  id: number
  model: string
  actual_cost: number
  input_tokens: number
  output_tokens: number
  failed: boolean
  created_at: string
  api_key_name?: string
}

// ── Component prop types ───────────────────────────────────────────────────

export type IntegrationTab = 'openai' | 'anthropic' | 'amp'

export interface DashboardAnnouncementsProps {
  announcements: AnnouncementItem[]
}

export interface AdminDashboardOverviewProps {
  stats: DashboardStats | null
}

export interface AdminDashboardChartsProps {
  trendData: TrendPoint[]
  modelData: ModelStat[]
  trendDays: 7 | 30
  onTrendDaysChange: (d: 7 | 30) => void
}

export interface UserDashboardHeroProps {
  email?: string
  stats: DashboardStats | null
  usageStats: UsageStats | null
}

export interface UserDashboardChartsProps {
  trendData: TrendPoint[]
  modelData: ModelStat[]
  trendDays: 7 | 30
  onTrendDaysChange: (d: 7 | 30) => void
}

export interface RecentUsageTableProps {
  recentUsage: RecentUsage[]
}

export interface QuickIntegrationPanelProps {
  apiKeyCount: number
  /** 累计成功调用数。0 表示还没跑通，面板切到引导态。 */
  totalRequests: number
  /** 可用余额。为 0 时请求会在预扣阶段被 402 拒掉，引导态要先把这一步点出来。 */
  balance?: number
  integrationTab: IntegrationTab
  onIntegrationTabChange: (tab: IntegrationTab) => void
}
