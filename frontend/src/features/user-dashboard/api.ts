// API functions for user-dashboard feature
import { apiClient } from '@/shared/api/client'
import type {
  UsageStats,
  AnnouncementItem,
  TrendPoint,
  ModelStat,
  RecentUsage,
} from './types'

// ── Response types ──────────────────────────────────────────────────────────

export interface UserProfileDashboardData {
  user?: { balance: number }
  available_balance?: number
  balance?: number
  key_count: number
  quota: number
  used_quota: number
  usage?: UsageStats
}

export interface AdminDashboardResponse {
  users: { total: number; active: number }
  api_keys: { total: number; active: number }
  usage: { today_requests: number; today_cost: number; week_requests: number }
}

export interface UsageDetailResponse {
  items: RecentUsage[]
  total: number
  page: number
  page_size: number
}

// ── Admin API Functions ─────────────────────────────────────────────────────

export function fetchAdminDashboard() {
  return apiClient.get<AdminDashboardResponse>('/admin/dashboard')
}

export function fetchAdminUsageTrend(days: number) {
  return apiClient.get<TrendPoint[]>(`/admin/usage/trend?days=${days}`)
}

export function fetchAdminModelStats() {
  return apiClient.get<ModelStat[]>('/admin/usage/models?days=30')
}

// ── User API Functions ──────────────────────────────────────────────────────

export function fetchUserProfile() {
  return apiClient.get<UserProfileDashboardData>('/user/profile')
}

export function fetchUserUsageStats() {
  return apiClient.get<{ usage: UsageStats | null }>('/user/usage/stats')
}

export function fetchUserUsageTrend(days: number) {
  return apiClient.get<TrendPoint[]>(`/user/usage/trend?days=${days}`)
}

export function fetchUserModelStats() {
  return apiClient.get<ModelStat[]>('/user/usage/models?days=30')
}

export function fetchUserRecentUsage() {
  return apiClient.get<UsageDetailResponse>('/user/usage/detail?page=1&page_size=5')
}

// ── Shared API Functions ────────────────────────────────────────────────────

export function fetchAnnouncements() {
  return apiClient.get<AnnouncementItem[]>('/user/announcements')
}
