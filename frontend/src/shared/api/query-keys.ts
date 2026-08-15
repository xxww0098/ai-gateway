/**
 * Query key factories for all feature modules.
 * Keys are structured as [module, entity, ...params] for hierarchical cache invalidation.
 *
 * Usage:
 *   queryKeys.users.all()    → invalidates all user-related queries
 *   queryKeys.users.list(p)  → targets a specific paginated list query
 *   queryKeys.users.detail(1)→ targets a single user detail query
 */
export const queryKeys = {
  auth: {
    profile: () => ['auth', 'profile'] as const,
  },
  users: {
    all: () => ['users'] as const,
    list: (params: { page: number; pageSize: number }) =>
      ['users', 'list', params] as const,
    detail: (id: number) => ['users', 'detail', id] as const,
  },
  subscriptions: {
    all: () => ['subscriptions'] as const,
    list: (params: { page: number; pageSize: number }) =>
      ['subscriptions', 'list', params] as const,
  },
  apiKeys: {
    all: () => ['apiKeys'] as const,
    list: () => ['apiKeys', 'list'] as const,
  },
  tickets: {
    all: () => ['tickets'] as const,
    list: (params: { page: number; pageSize: number }) =>
      ['tickets', 'list', params] as const,
  },
  orders: {
    all: () => ['orders'] as const,
    list: (params: { page: number; pageSize: number; status?: string }) =>
      ['orders', 'list', params] as const,
    subscriptions: () => ['orders', 'subscriptions'] as const,
    refunds: () => ['orders', 'refunds'] as const,
  },
  usage: {
    all: () => ['usage'] as const,
    logs: (params: Record<string, unknown>) => ['usage', 'logs', params] as const,
    summary: () => ['usage', 'summary'] as const,
  },
  pricing: {
    all: () => ['pricing'] as const,
    models: () => ['pricing', 'models'] as const,
  },
  groups: {
    all: () => ['groups'] as const,
    list: () => ['groups', 'list'] as const,
  },
  proxy: {
    all: () => ['proxy'] as const,
    authFiles: (params?: Record<string, unknown>) =>
      ['proxy', 'authFiles', params] as const,
    providers: () => ['proxy', 'providers'] as const,
  },
  payment: {
    all: () => ['payment'] as const,
    wechatOrder: (orderId: string) => ['payment', 'wechatOrder', orderId] as const,
    wechatStatus: (orderId: string) => ['payment', 'wechatStatus', orderId] as const,
    alipayStatus: (orderId: string) => ['payment', 'alipayStatus', orderId] as const,
    stripeConfig: () => ['payment', 'stripeConfig'] as const,
  },
  dashboard: {
    all: () => ['dashboard'] as const,
    stats: () => ['dashboard', 'stats'] as const,
    trend: (days: number) => ['dashboard', 'trend', days] as const,
    recentUsage: () => ['dashboard', 'recentUsage'] as const,
  },
} as const
