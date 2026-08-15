import { useState, useCallback, useMemo, useRef } from "react"
import { toast } from "sonner"
import {
  RefreshCw, Search, UserPlus, MoreHorizontal,
  Pencil, Plus, Key, Clock, Trash2,
  ShieldCheck, ChevronLeft, ChevronRight
} from "lucide-react"
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu"
import { useUsers, useDeleteUser } from "@/features/admin-users/hooks"
import type { UserItem } from "@/features/admin-users/types"
import { confirmModal } from "@/shared/confirm-modal"
import { QueryStateWrapper } from "@/shared/components/QueryStateWrapper"
import { AdminUserCreateDialog } from "@/features/admin-users/components/AdminUserCreateDialog"
import { AdminUserEditDialog } from "@/features/admin-users/components/AdminUserEditDialog"
import { AdminUserDepositDialog } from "@/features/admin-users/components/AdminUserDepositDialog"
import { AdminUserApiKeysDialog } from "@/features/admin-users/components/AdminUserApiKeysDialog"
import { AdminUserHistoryDialog } from "@/features/admin-users/components/AdminUserHistoryDialog"

export default function AdminUsersPage() {
  // Pagination & filters
  const [page, setPage] = useState(1)
  const [searchQuery, setSearchQuery] = useState("")
  const [debouncedSearch, setDebouncedSearch] = useState("")
  const [filterRole, setFilterRole] = useState("")
  const [filterStatus, setFilterStatus] = useState("")
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Query
  const { data, isLoading: loading, refetch } = useUsers({
    page,
    pageSize: 15,
    search: debouncedSearch || undefined,
    role: filterRole || undefined,
    status: filterStatus || undefined,
  })

  const users = data?.items ?? []
  const totalUsers = data?.total ?? 0
  const pageSize = data?.page_size ?? 15
  const totalPages = Math.max(1, Math.ceil(totalUsers / pageSize))

  // Mutations
  const deleteUserMutation = useDeleteUser()

  // Dialog states
  const [showCreate, setShowCreate] = useState(false)
  const [actionUser, setActionUser] = useState<UserItem | null>(null)
  const [showEdit, setShowEdit] = useState(false)
  const [showDeposit, setShowDeposit] = useState(false)
  const [showKeys, setShowKeys] = useState(false)
  const [showHistory, setShowHistory] = useState(false)
  const [selectedUserIds, setSelectedUserIds] = useState<Set<number>>(new Set())
  const [batchDeleting, setBatchDeleting] = useState(false)

  // Search debounce
  const handleSearchInput = useCallback((value: string) => {
    setSearchQuery(value)
    if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current)
    searchTimeoutRef.current = setTimeout(() => {
      setDebouncedSearch(value.trim())
      setPage(1)
    }, 300)
  }, [])

  // Dialog helpers — memoized to avoid re-creating on every render
  const openEdit = useCallback((u: UserItem) => { setActionUser(u); setShowEdit(true) }, [])
  const openDeposit = useCallback((u: UserItem) => { setActionUser(u); setShowDeposit(true) }, [])
  const openKeys = useCallback((u: UserItem) => { setActionUser(u); setShowKeys(true) }, [])
  const openHistory = useCallback((u: UserItem) => { setActionUser(u); setShowHistory(true) }, [])

  // Selection — memoized derived values
  const selectedVisibleCount = useMemo(() => users.filter(u => selectedUserIds.has(u.id)).length, [users, selectedUserIds])
  const allVisibleSelected = useMemo(() => users.length > 0 && selectedVisibleCount === users.length, [users.length, selectedVisibleCount])
  const selectionIndeterminate = useMemo(() => selectedVisibleCount > 0 && selectedVisibleCount < users.length, [selectedVisibleCount, users.length])
  const handleSelectAllVisible = useCallback((checked: boolean) => {
    setSelectedUserIds(checked ? new Set(users.map(u => u.id)) : new Set())
  }, [users])
  const handleSelectUser = useCallback((id: number, checked: boolean) => {
    setSelectedUserIds(prev => {
      const next = new Set(prev)
      if (checked) { next.add(id) } else { next.delete(id) }
      return next
    })
  }, [])

  const handleDeleteUser = useCallback(async (u: UserItem) => {
    const ok = await confirmModal({
      title: "删除用户",
      message: `删除用户 ${u.email}？\n该用户将从列表移除，CPA API Key 会同时禁用。`,
      confirmText: "删除",
      cancelText: "取消",
      variant: "danger",
    })
    if (!ok) return
    try {
      await deleteUserMutation.mutateAsync(u.id)
      setSelectedUserIds(prev => {
        const next = new Set(prev)
        next.delete(u.id)
        return next
      })
    } catch {
      // Error toast handled by hook
    }
  }, [deleteUserMutation])

  const handleBatchDelete = useCallback(async () => {
    const targets = users.filter(u => selectedUserIds.has(u.id))
    if (targets.length === 0) return
    const preview = targets.slice(0, 3).map(u => u.email).join("、")
    const previewText = targets.length > 3 ? `${preview} 等 ${targets.length} 位用户` : preview
    const ok = await confirmModal({
      title: `批量删除 ${targets.length} 位用户`,
      message: `${previewText}\n\n这些用户将从列表移除，CPA API Key 会同时禁用。`,
      confirmText: "批量删除",
      cancelText: "取消",
      variant: "danger",
    })
    if (!ok) return

    setBatchDeleting(true)
    let successCount = 0
    let failCount = 0
    try {
      for (const u of targets) {
        try {
          await deleteUserMutation.mutateAsync(u.id)
          successCount++
        } catch {
          failCount++
        }
      }
      setSelectedUserIds(new Set())
      if (failCount === 0) {
        toast.success(`成功删除 ${successCount} 位用户`)
      } else {
        toast.warning(`批量删除完成：${successCount} 成功，${failCount} 失败`)
      }
    } finally {
      setBatchDeleting(false)
    }
  }, [users, selectedUserIds, deleteUserMutation])

  // ── Render ──
  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500" style={{ willChange: 'transform, opacity' }}>
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">平台账户管理</h2>
          <p className="text-gray-500 dark:text-dark-300 mt-1 max-w-2xl">
            管理所有开发者账户，包括角色授权、USD 计费余额、并发数及 API Key 分配。
          </p>
        </div>
        <div className="flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400">
          <span className="rounded-md border border-gray-200 dark:border-dark-600 bg-gray-50 dark:bg-dark-800 px-2 py-1 font-medium tabular-nums">
            {loading ? "同步中..." : `${totalUsers} 位用户`}
          </span>
          <button className="btn btn-primary px-4 py-2 text-sm" onClick={() => setShowCreate(true)}>
            <UserPlus className="h-4 w-4" />
            手动注册
          </button>
        </div>
      </div>

      {/* Filter Bar */}
      <div className="rounded-xl border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-800/70 shadow-sm p-3">
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-[minmax(240px,1fr)_130px_130px_auto]">
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-400" />
            <input
              type="text"
              className="input h-9 pl-8 text-sm shadow-none"
              placeholder="搜索邮箱、用户名..."
              value={searchQuery}
              onChange={(e) => handleSearchInput(e.target.value)}
            />
          </div>
          <select
            className="input h-9 text-sm shadow-none"
            value={filterRole}
            onChange={(e) => { setFilterRole(e.target.value); setPage(1) }}
          >
            <option value="">全部角色</option>
            <option value="admin">Admin</option>
            <option value="user">User</option>
          </select>
          <select
            className="input h-9 text-sm shadow-none"
            value={filterStatus}
            onChange={(e) => { setFilterStatus(e.target.value); setPage(1) }}
          >
            <option value="">全部状态</option>
            <option value="active">Active</option>
            <option value="disabled">Disabled</option>
          </select>
          <button
            className="btn btn-secondary h-9 px-3 text-sm shadow-none"
            onClick={() => refetch()}
            disabled={loading}
          >
            <RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
            刷新
          </button>
        </div>
        {selectedVisibleCount > 0 && (
          <div className="mt-3 flex flex-wrap items-center justify-between gap-2 rounded-lg border border-primary-200 bg-primary-50/80 px-3 py-2 dark:border-primary-900/60 dark:bg-primary-950/30">
            <span className="text-xs font-medium text-primary-700 dark:text-primary-300">
              已选 {selectedVisibleCount} 位用户
            </span>
            <button
              className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-red-600 px-3 text-xs font-medium text-white shadow-sm transition-colors hover:bg-red-700 disabled:cursor-not-allowed disabled:opacity-60"
              onClick={() => { void handleBatchDelete() }}
              disabled={batchDeleting}
            >
              <Trash2 className="h-3.5 w-3.5" />
              {batchDeleting ? "删除中..." : "批量删除"}
            </button>
          </div>
        )}
      </div>

      {/* Data Table */}
      <div className="glass-card overflow-hidden">
        <QueryStateWrapper
          isLoading={loading}
          error={data === undefined && !loading ? new Error('加载用户列表失败') : null}
          isEmpty={!loading && users.length === 0}
          onRetry={() => refetch()}
          emptyMessage={debouncedSearch || filterRole || filterStatus ? "无匹配结果，请调整筛选条件" : "暂无注册用户"}
        >
        <div className="overflow-x-auto">
          <table className="table">
            <thead>
              <tr>
                <th className="w-[44px]">
                  <div className="flex items-center justify-center">
                    <input
                      type="checkbox"
                      className="h-3.5 w-3.5 cursor-pointer rounded border-gray-300 text-primary-600 focus:ring-primary-500 dark:border-dark-600 dark:bg-dark-800 dark:ring-offset-dark-900"
                      checked={allVisibleSelected}
                      ref={input => {
                        if (input) input.indeterminate = selectionIndeterminate
                      }}
                      onChange={e => handleSelectAllVisible(e.target.checked)}
                      disabled={loading || users.length === 0}
                      aria-label="选择当前页全部用户"
                    />
                  </div>
                </th>
                <th>用户信息</th>
                <th>角色</th>
                <th className="text-right">余额 (USD)</th>
                <th className="text-center">并发数</th>
                <th>状态</th>
                <th className="text-right">注册时间</th>
                <th className="w-[60px]"></th>
              </tr>
            </thead>
            <tbody>
              {users.map((u) => (
                  <tr key={u.id} className="group">
                    <td>
                      <div className="flex items-center justify-center">
                        <input
                          type="checkbox"
                          className="h-3.5 w-3.5 cursor-pointer rounded border-gray-300 text-primary-600 focus:ring-primary-500 dark:border-dark-600 dark:bg-dark-800 dark:ring-offset-dark-900"
                          checked={selectedUserIds.has(u.id)}
                          onChange={e => handleSelectUser(u.id, e.target.checked)}
                          aria-label={`选择用户 ${u.email}`}
                        />
                      </div>
                    </td>
                    <td>
                      <div className="min-w-0 space-y-0.5">
                        <div className="flex items-center gap-2">
                          <p className="truncate text-[13px] font-medium leading-5 text-gray-900 dark:text-white">{u.email}</p>
                          {u.role === "admin" && <ShieldCheck className="h-3.5 w-3.5 text-emerald-500 flex-shrink-0" />}
                        </div>
                        <p className="truncate font-mono text-[11px] text-gray-400 dark:text-dark-400">
                          ID {u.id}{u.username ? ` · ${u.username}` : ""}
                        </p>
                      </div>
                    </td>
                    <td>
                      <span className={`inline-flex items-center rounded-md px-2 py-0.5 text-[11px] font-medium ring-1 ring-inset ${
                        u.role === "admin"
                          ? "bg-purple-50 text-purple-700 ring-purple-600/20 dark:bg-purple-900/20 dark:text-purple-400 dark:ring-purple-500/20"
                          : "bg-blue-50 text-blue-700 ring-blue-600/20 dark:bg-blue-900/20 dark:text-blue-400 dark:ring-blue-500/20"
                      }`}>
                        {u.role.toUpperCase()}
                      </span>
                    </td>
                    <td className="text-right">
                      <span className="font-mono font-bold text-[13px] tabular-nums text-gray-800 dark:text-gray-200">
                        ${u.balance.toFixed(4)}
                      </span>
                    </td>
                    <td className="text-center">
                      <span className="font-mono text-[13px] text-gray-600 dark:text-gray-400">{u.concurrency}</span>
                    </td>
                    <td>
                      <span className={`inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-[11px] font-medium ${
                        u.status === "active"
                          ? "bg-green-50 text-green-700 dark:bg-green-900/20 dark:text-green-400"
                          : "bg-gray-100 text-gray-600 dark:bg-dark-700 dark:text-gray-400"
                      }`}>
                        <span className={`h-1.5 w-1.5 rounded-full ${u.status === "active" ? "bg-green-500" : "bg-gray-400"}`} />
                        {u.status === "active" ? "正常" : "禁用"}
                      </span>
                    </td>
                    <td className="text-right text-[13px] text-gray-500 dark:text-gray-400 tabular-nums">
                      {new Date(u.created_at).toLocaleString("zh-CN", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}
                    </td>
                    <td>
                      <DropdownMenuPrimitive.Root>
                        <DropdownMenuPrimitive.Trigger asChild>
                          <button className="h-7 w-7 rounded-md p-0 flex items-center justify-center text-gray-500 hover:bg-gray-100 dark:hover:bg-dark-700 hover:text-gray-900 dark:hover:text-white transition-colors">
                            <MoreHorizontal className="h-3.5 w-3.5" />
                          </button>
                        </DropdownMenuPrimitive.Trigger>
                        <DropdownMenuPrimitive.Portal>
                          <DropdownMenuPrimitive.Content
                            align="end"
                            sideOffset={4}
                            className="z-50 w-48 rounded-xl border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-800 p-1 shadow-lg animate-in fade-in-0 zoom-in-95"
                          >
                            <DropdownMenuPrimitive.Item className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-gray-700 dark:text-gray-200 cursor-pointer outline-none hover:bg-gray-100 dark:hover:bg-dark-700" onSelect={() => openEdit(u)}>
                              <Pencil className="h-4 w-4" /> 编辑用户
                            </DropdownMenuPrimitive.Item>
                            <DropdownMenuPrimitive.Item className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-emerald-600 dark:text-emerald-400 cursor-pointer outline-none hover:bg-emerald-50 dark:hover:bg-emerald-900/20" onSelect={() => openDeposit(u)}>
                              <Plus className="h-4 w-4" /> 充值余额
                            </DropdownMenuPrimitive.Item>
                            <DropdownMenuPrimitive.Item className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-gray-700 dark:text-gray-200 cursor-pointer outline-none hover:bg-gray-100 dark:hover:bg-dark-700" onSelect={() => openKeys(u)}>
                              <Key className="h-4 w-4" /> 查看 API Keys
                            </DropdownMenuPrimitive.Item>
                            <DropdownMenuPrimitive.Item className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-gray-700 dark:text-gray-200 cursor-pointer outline-none hover:bg-gray-100 dark:hover:bg-dark-700" onSelect={() => openHistory(u)}>
                              <Clock className="h-4 w-4" /> 余额变动记录
                            </DropdownMenuPrimitive.Item>
                            <DropdownMenuPrimitive.Separator className="my-1 h-px bg-gray-200 dark:bg-dark-600" />
                            <DropdownMenuPrimitive.Item className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-red-600 dark:text-red-400 cursor-pointer outline-none hover:bg-red-50 dark:hover:bg-red-900/20" onSelect={() => { void handleDeleteUser(u) }}>
                              <Trash2 className="h-4 w-4" /> 删除用户
                            </DropdownMenuPrimitive.Item>
                          </DropdownMenuPrimitive.Content>
                        </DropdownMenuPrimitive.Portal>
                      </DropdownMenuPrimitive.Root>
                    </td>
                  </tr>
                ))
              }
            </tbody>
          </table>
        </div>
        </QueryStateWrapper>

        {/* Pagination */}
        {totalPages > 1 && (
          <div className="flex items-center justify-between border-t border-border px-4 py-3">
            <p className="text-xs text-gray-500 dark:text-gray-400 tabular-nums">
              第 {page} / {totalPages} 页 · 共 {totalUsers} 条
            </p>
            <div className="flex items-center gap-1">
              <button
                className="h-8 w-8 rounded-lg border border-gray-200 dark:border-dark-600 flex items-center justify-center text-gray-500 hover:bg-gray-100 dark:hover:bg-dark-700 disabled:opacity-40 transition-colors"
                disabled={page <= 1}
                onClick={() => setPage(p => Math.max(1, p - 1))}
              >
                <ChevronLeft className="h-4 w-4" />
              </button>
              {Array.from({ length: Math.min(5, totalPages) }, (_, i) => {
                let pageNum: number
                if (totalPages <= 5) { pageNum = i + 1 }
                else if (page <= 3) { pageNum = i + 1 }
                else if (page >= totalPages - 2) { pageNum = totalPages - 4 + i }
                else { pageNum = page - 2 + i }
                return (
                  <button
                    key={pageNum}
                    className={`h-8 w-8 rounded-lg text-xs font-medium transition-colors ${
                      pageNum === page
                        ? "bg-primary-500 text-white shadow-sm"
                        : "border border-gray-200 dark:border-dark-600 text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-dark-700"
                    }`}
                    onClick={() => setPage(pageNum)}
                  >
                    {pageNum}
                  </button>
                )
              })}
              <button
                className="h-8 w-8 rounded-lg border border-gray-200 dark:border-dark-600 flex items-center justify-center text-gray-500 hover:bg-gray-100 dark:hover:bg-dark-700 disabled:opacity-40 transition-colors"
                disabled={page >= totalPages}
                onClick={() => setPage(p => Math.min(totalPages, p + 1))}
              >
                <ChevronRight className="h-4 w-4" />
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Dialogs */}
      <AdminUserCreateDialog open={showCreate} onOpenChange={setShowCreate} />
      <AdminUserEditDialog open={showEdit} onOpenChange={setShowEdit} user={actionUser} />
      <AdminUserDepositDialog open={showDeposit} onOpenChange={setShowDeposit} user={actionUser} />
      <AdminUserApiKeysDialog open={showKeys} onOpenChange={setShowKeys} user={actionUser} />
      <AdminUserHistoryDialog open={showHistory} onOpenChange={setShowHistory} user={actionUser} />
    </div>
  )
}
