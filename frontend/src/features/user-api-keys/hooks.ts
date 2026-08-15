import { useState } from "react"
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import { errorMessage } from "@/shared/api/errors"
import { confirmModal } from "@/shared/confirm-modal"
import { toast } from "sonner"
import type { CreateKeyForm, CreateKeyPayload } from "./types"
import {
  fetchApiKeys,
  createApiKey,
  deleteApiKey,
  rebindApiKeyGroup,
  fetchAvailableGroups,
} from "./api"

// ── Helpers ─────────────────────────────────────────────────────────────────

function isSubscriptionRequiredError(err: unknown): boolean {
  const msg = errorMessage(err, "")
  return msg.includes("需要先订阅该分组")
}

/** Converts the form values (strings) into a typed API payload */
function buildCreatePayload(form: CreateKeyForm): CreateKeyPayload {
  const payload: CreateKeyPayload = { name: form.name.trim() }
  if (form.quota) payload.quota = parseFloat(form.quota)
  if (form.rate_5h) payload.rate_limit_5h = parseFloat(form.rate_5h)
  if (form.rate_1d) payload.rate_limit_1d = parseFloat(form.rate_1d)
  if (form.rate_7d) payload.rate_limit_7d = parseFloat(form.rate_7d)
  if (form.rate_30d) payload.rate_limit_30d = parseFloat(form.rate_30d)
  if (form.group_id) payload.group_id = form.group_id
  if (form.expires_in_days !== undefined) payload.expires_in_days = form.expires_in_days
  return payload
}

// ── useApiKeys (main composite hook) ────────────────────────────────────────

export function useApiKeys() {
  const [copiedId, setCopiedId] = useState<number | null>(null)
  const [rebindingId, setRebindingId] = useState<number | null>(null)
  const [needSubDialog, setNeedSubDialog] = useState<{ open: boolean; groupName?: string }>({ open: false })
  const qc = useQueryClient()

  const keysQuery = useQuery({
    queryKey: queryKeys.apiKeys.list(),
    queryFn: fetchApiKeys,
  })

  const groupsQuery = useQuery({
    queryKey: queryKeys.groups.list(),
    queryFn: fetchAvailableGroups,
  })

  const invalidateKeys = () => {
    qc.invalidateQueries({ queryKey: queryKeys.apiKeys.all() })
  }

  const handleCreate = async (form: CreateKeyForm, onSuccess: () => void) => {
    if (!form.name.trim()) return
    try {
      const payload = buildCreatePayload(form)
      await createApiKey(payload)
      toast.success("成功创建 API Key")
      onSuccess()
      invalidateKeys()
    } catch (err: unknown) {
      if (isSubscriptionRequiredError(err)) {
        const gid = form.group_id
        const gname = gid ? groups.find((g) => g.id === gid)?.name : undefined
        setNeedSubDialog({ open: true, groupName: gname })
        return
      }
      toast.error(errorMessage(err, "创建失败"))
    }
  }

  const handleDelete = async (id: number) => {
    const ok = await confirmModal({
      title: "删除 API Key",
      message: "确定要删除这个 API Key 吗？此操作不可撤销。",
      variant: "danger",
      confirmText: "删除",
    })
    if (!ok) return
    try {
      await deleteApiKey(id)
      toast.success("API Key 已删除")
      invalidateKeys()
    } catch (err: unknown) {
      toast.error(errorMessage(err, "删除失败"))
    }
  }

  const handleCopy = (id: number, keyStr: string) => {
    navigator.clipboard.writeText(keyStr)
    setCopiedId(id)
    setTimeout(() => setCopiedId(null), 2000)
    toast.success("已复制到剪贴板")
  }

  const handleRebindGroup = async (keyId: number, groupId: number | null) => {
    setRebindingId(keyId)
    try {
      await rebindApiKeyGroup(keyId, groupId)
      toast.success(groupId ? "已切换分组" : "已解绑分组")
      invalidateKeys()
    } catch (err: unknown) {
      if (isSubscriptionRequiredError(err)) {
        const gname = groupId != null ? groups.find((g) => g.id === groupId)?.name : undefined
        setNeedSubDialog({ open: true, groupName: gname })
        return
      }
      toast.error(errorMessage(err, "切换分组失败"))
    } finally {
      setRebindingId(null)
    }
  }

  const keys = keysQuery.data?.items ?? []
  const groups = groupsQuery.data ?? []
  const loading = keysQuery.isLoading
  const groupsLoading = groupsQuery.isLoading

  return {
    keys,
    loading,
    copiedId,
    groups,
    groupsLoading,
    rebindingId,
    needSubDialog,
    setNeedSubDialog,
    loadKeys: invalidateKeys,
    handleCreate,
    handleDelete,
    handleCopy,
    handleRebindGroup,
  }
}

// ── useCreateKey (standalone mutation hook) ─────────────────────────────────

export function useCreateKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (payload: CreateKeyPayload) => createApiKey(payload),
    onSuccess: () => {
      toast.success("成功创建 API Key")
      qc.invalidateQueries({ queryKey: queryKeys.apiKeys.all() })
    },
    onError: (err) => toast.error(errorMessage(err, "创建失败")),
  })
}

// ── useDeleteKey (standalone mutation hook) ──────────────────────────────────

export function useDeleteKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => deleteApiKey(id),
    onSuccess: () => {
      toast.success("API Key 已删除")
      qc.invalidateQueries({ queryKey: queryKeys.apiKeys.all() })
    },
    onError: (err) => toast.error(errorMessage(err, "删除失败")),
  })
}
