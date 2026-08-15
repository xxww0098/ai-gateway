import { useState } from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import { errorMessage } from "@/shared/api/errors"
import { toast } from "sonner"
import type { Group } from "./types"
import {
  fetchSubscriptions,
  fetchGroups,
  extendSubscription,
  revokeSubscription,
  reactivateSubscription,
  reactivateSubscriptionFallback,
  resetSubscriptionQuota,
  createGroup,
  updateGroup,
  deleteGroup,
} from "./api"

// ── Subscriptions Hook ──────────────────────────────────────────────────────

interface UseSubscriptionsOptions {
  pageSize?: number
}

export function useSubscriptions(opts: UseSubscriptionsOptions = {}) {
  const { pageSize = 20 } = opts
  const [page, setPage] = useState(1)
  const qc = useQueryClient()

  const subsQuery = useQuery({
    queryKey: queryKeys.subscriptions.list({ page, pageSize }),
    queryFn: () => fetchSubscriptions(page, pageSize),
  })

  const groupsQuery = useQuery({
    queryKey: queryKeys.groups.list(),
    queryFn: fetchGroups,
    select: (data) => (data || []).filter((g: Group) => g.subscription_type === "subscription"),
  })

  const invalidateAll = () => {
    qc.invalidateQueries({ queryKey: queryKeys.subscriptions.all() })
    qc.invalidateQueries({ queryKey: queryKeys.groups.all() })
  }

  const handleExtend = async (id: number, days: number) => {
    try {
      await extendSubscription(id, days)
      toast.success("续期成功")
      invalidateAll()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "续期失败"))
    }
  }

  const handleRevoke = async (id: number) => {
    try {
      await revokeSubscription(id)
      toast.success("订阅已撤销")
      invalidateAll()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "撤销失败"))
    }
  }

  const handleReactivate = async (id: number) => {
    const isNotFoundMsg = (m: string) => /^not\s*found$/i.test(m.trim())
    try {
      await reactivateSubscription(id)
      toast.success("订阅已恢复")
      invalidateAll()
    } catch (errPost: unknown) {
      const msgPost = errPost instanceof Error ? errPost.message : ""
      if (isNotFoundMsg(msgPost)) {
        try {
          await reactivateSubscriptionFallback(id)
          toast.success("订阅已恢复")
          invalidateAll()
          return
        } catch (errPut: unknown) {
          const msgPut = errPut instanceof Error ? errPut.message : ""
          if (isNotFoundMsg(msgPut)) {
            toast.error("恢复接口未注册：请重新编译并重启后端（需含 /subscriptions/:id/reactivate）。")
            return
          }
          toast.error(msgPut || "恢复失败")
          return
        }
      }
      toast.error(msgPost || "恢复失败")
    }
  }

  const handleResetQuota = async (id: number) => {
    try {
      await resetSubscriptionQuota(id)
      toast.success("配额已重置")
      invalidateAll()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "重置失败"))
    }
  }

  const subs = subsQuery.data?.items || []
  const total = subsQuery.data?.total || 0
  const totalPages = Math.ceil(total / pageSize)
  const loading = subsQuery.isLoading || groupsQuery.isLoading

  return {
    subs,
    groups: groupsQuery.data || [],
    loading,
    total,
    page,
    setPage,
    totalPages,
    reload: invalidateAll,
    loadData: invalidateAll,
    handleExtend,
    handleRevoke,
    handleReactivate,
    handleResetQuota,
  }
}

// ── Group CRUD Hook ─────────────────────────────────────────────────────────

export function useGroupCrud(onSaved?: () => void | Promise<void>) {
  const [groupDialogOpen, setGroupDialogOpen] = useState(false)
  const [editingGroupId, setEditingGroupId] = useState<number | null>(null)
  const [groupForm, setGroupForm] = useState({
    name: "",
    rate_multiplier: "1.0",
    daily_limit_usd: "",
    weekly_limit_usd: "",
    monthly_limit_usd: "",
    default_validity_days: "30",
    subscription_price_usd: "",
  })
  const [savingGroup, setSavingGroup] = useState(false)
  const qc = useQueryClient()

  const invalidateGroups = () => {
    qc.invalidateQueries({ queryKey: queryKeys.groups.all() })
    qc.invalidateQueries({ queryKey: queryKeys.subscriptions.all() })
  }

  const openCreateGroup = () => {
    setEditingGroupId(null)
    setGroupForm({
      name: "",
      rate_multiplier: "1.0",
      daily_limit_usd: "",
      weekly_limit_usd: "",
      monthly_limit_usd: "",
      default_validity_days: "30",
      subscription_price_usd: "",
    })
    setGroupDialogOpen(true)
  }

  const openEditGroup = (g: Group) => {
    setEditingGroupId(g.id)
    setGroupForm({
      name: g.name,
      rate_multiplier: String(g.rate_multiplier ?? 1.0),
      daily_limit_usd: g.daily_limit_usd != null ? String(g.daily_limit_usd) : "",
      weekly_limit_usd: g.weekly_limit_usd != null ? String(g.weekly_limit_usd) : "",
      monthly_limit_usd: g.monthly_limit_usd != null ? String(g.monthly_limit_usd) : "",
      default_validity_days: String(g.default_validity_days ?? 30),
      subscription_price_usd:
        g.subscription_price_usd != null && g.subscription_price_usd > 0
          ? String(g.subscription_price_usd)
          : "",
    })
    setGroupDialogOpen(true)
  }

  const handleSaveGroup = async (e: React.FormEvent) => {
    e.preventDefault()
    setSavingGroup(true)
    try {
      const body: Record<string, unknown> = {
        name: groupForm.name,
        subscription_type: "subscription",
        rate_multiplier: parseFloat(groupForm.rate_multiplier) || 1.0,
        default_validity_days: parseInt(groupForm.default_validity_days) || 30,
      }
      if (groupForm.daily_limit_usd) body.daily_limit_usd = parseFloat(groupForm.daily_limit_usd)
      if (groupForm.weekly_limit_usd) body.weekly_limit_usd = parseFloat(groupForm.weekly_limit_usd)
      if (groupForm.monthly_limit_usd) body.monthly_limit_usd = parseFloat(groupForm.monthly_limit_usd)
      const selfPrice = parseFloat(groupForm.subscription_price_usd)
      body.subscription_price_usd = !Number.isFinite(selfPrice) || selfPrice < 0 ? 0 : selfPrice

      if (editingGroupId) {
        await updateGroup(editingGroupId, body)
        toast.success("订阅分组已更新")
      } else {
        await createGroup(body)
        toast.success("订阅分组创建成功")
      }
      invalidateGroups()
      await onSaved?.()
      setGroupDialogOpen(false)
    } catch (err: unknown) {
      toast.error(errorMessage(err, "保存失败"))
    } finally {
      setSavingGroup(false)
    }
  }

  const handleDeleteGroup = async (g: Group) => {
    if (!confirm(`确定删除订阅分组「${g.name}」？关联的订阅将失效。`)) return
    try {
      await deleteGroup(g.id)
      toast.success("分组已删除")
      invalidateGroups()
      onSaved?.()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "删除失败"))
    }
  }

  return {
    groupDialogOpen,
    setGroupDialogOpen,
    editingGroupId,
    groupForm,
    setGroupForm,
    savingGroup,
    openCreateGroup,
    openEditGroup,
    handleSaveGroup,
    handleDeleteGroup,
  }
}
