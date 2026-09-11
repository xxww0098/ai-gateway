import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiClient } from '@/shared/api/client'
import { queryKeys } from '@/shared/api/query-keys'
import { useAuthStore } from '@/features/auth/auth_store'

export type NotificationItem = {
  id: number
  title: string
  content: string
  is_read: boolean
  notification_type: string
  created_at: string
  related_id?: number | null
}

type NotificationsPage = {
  items: NotificationItem[]
  total: number
}

export const NOTIFICATIONS_PAGE_SIZE = 10

type ListCache = {
  items: NotificationItem[]
  total: number
}

function unreadOnPage(items: NotificationItem[], total: number): number | null {
  if (total <= NOTIFICATIONS_PAGE_SIZE) {
    return items.filter(item => !item.is_read).length
  }
  return null
}

export function useUnreadCount(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.notifications.unread(),
    queryFn: async () => {
      const data = await apiClient.get<{ unread_count: number }>(
        '/user/notifications/unread-count',
        { cache: 'no-store' },
      )
      return data.unread_count ?? 0
    },
    enabled,
    staleTime: 30 * 1000,
    refetchOnWindowFocus: true,
    retry: 0,
  })
}

export function useNotificationList(enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.notifications.list(),
    queryFn: async () => {
      const data = await apiClient.get<NotificationsPage>(
        `/user/notifications?page=1&page_size=${NOTIFICATIONS_PAGE_SIZE}`,
        { cache: 'no-store' },
      )
      return {
        items: data.items ?? [],
        total: data.total ?? 0,
      } satisfies ListCache
    },
    enabled,
    retry: 0,
  })
}

export function useMarkNotificationRead() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => apiClient.put(`/user/notifications/${id}/read`),
    onMutate: async (id) => {
      await qc.cancelQueries({ queryKey: queryKeys.notifications.all() })
      const prevList = qc.getQueryData<ListCache>(queryKeys.notifications.list())
      const prevUnread = qc.getQueryData<number>(queryKeys.notifications.unread())
      qc.setQueryData<ListCache>(queryKeys.notifications.list(), old =>
        old
          ? { ...old, items: old.items.map(item => (item.id === id ? { ...item, is_read: true } : item)) }
          : old,
      )
      qc.setQueryData<number>(queryKeys.notifications.unread(), n => Math.max(0, (n ?? 0) - 1))
      return { prevList, prevUnread }
    },
    onError: (_err, _id, ctx) => {
      if (ctx?.prevList) qc.setQueryData(queryKeys.notifications.list(), ctx.prevList)
      if (ctx?.prevUnread !== undefined) qc.setQueryData(queryKeys.notifications.unread(), ctx.prevUnread)
    },
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.notifications.all() })
    },
  })
}

export function useMarkAllNotificationsRead() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: () => apiClient.put('/user/notifications/read-all'),
    onMutate: async () => {
      await qc.cancelQueries({ queryKey: queryKeys.notifications.all() })
      const prevList = qc.getQueryData<ListCache>(queryKeys.notifications.list())
      const prevUnread = qc.getQueryData<number>(queryKeys.notifications.unread())
      qc.setQueryData<ListCache>(queryKeys.notifications.list(), old =>
        old ? { ...old, items: old.items.map(item => ({ ...item, is_read: true })) } : old,
      )
      qc.setQueryData<number>(queryKeys.notifications.unread(), 0)
      return { prevList, prevUnread }
    },
    onError: (_err, _id, ctx) => {
      if (ctx?.prevList) qc.setQueryData(queryKeys.notifications.list(), ctx.prevList)
      if (ctx?.prevUnread !== undefined) qc.setQueryData(queryKeys.notifications.unread(), ctx.prevUnread)
    },
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.notifications.all() })
    },
  })
}

export function useHeaderNotifications(open: boolean) {
  const user = useAuthStore(s => s.user)
  const signedIn = Boolean(user)
  const unreadQuery = useUnreadCount(signedIn)
  const listQuery = useNotificationList(signedIn && open)
  const markRead = useMarkNotificationRead()
  const markAllRead = useMarkAllNotificationsRead()

  const items = listQuery.data?.items ?? []
  const synced = listQuery.data
    ? unreadOnPage(listQuery.data.items, listQuery.data.total)
    : null

  return {
    items,
    unreadCount: synced ?? unreadQuery.data ?? 0,
    loading: listQuery.isLoading,
    markRead: markRead.mutate,
    markAllRead: markAllRead.mutate,
  }
}
