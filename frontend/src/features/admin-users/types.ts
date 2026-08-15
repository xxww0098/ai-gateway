// Types for admin users management

export interface UserItem {
  id: number
  email: string
  username?: string | null
  role: string
  balance: number
  status: string
  concurrency: number
  created_at: string
  updated_at: string
}

export interface ManagedApiKey {
  id: number
  name: string
  prefix: string
  status: string
  quota: number
  quota_used: number
  created_at: string
}

export interface BalanceHistoryEntry {
  id: number
  kind: string
  amount: number
  balance_before: number
  balance_after: number
  operator_email?: string | null
  note?: string | null
  created_at: string
}

export interface CreateUserPayload {
  email: string
  password: string
  role: string
  username?: string
  balance?: number
}

export interface UpdateUserPayload {
  role: string
  balance: number
  concurrency: number
  status: string
  username: string | null
  password?: string
}

export interface DepositPayload {
  amount: number
  note?: string
}

/** Parameters for the paginated users list query */
export interface LoadUsersParams {
  page: number
  pageSize?: number
  search?: string
  role?: string
  status?: string
}
