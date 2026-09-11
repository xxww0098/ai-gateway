import { useState, useCallback } from "react"
import { Button } from "@/shared/components/ui/button"
import { PackagePlus } from "lucide-react"
import { EmptyState } from "@/shared/components/EmptyState"
import {
  useSubscriptions,
  useGroupCrud,
  AdminSubscriptionAssignDialog,
  AdminSubscriptionExtendDialog,
  AdminSubscriptionGroupDialog,
  AdminSubscriptionsTable,
  SubscriptionPackageCard,
} from "@/features/admin-subscriptions"

export default function Subscriptions() {
  const { subs, groups, loading, page, setPage, totalPages, loadData, handleExtend, handleRevoke, handleReactivate, handleResetQuota } =
    useSubscriptions()

  const { openCreateGroup, groupDialogOpen, setGroupDialogOpen, editingGroupId, groupForm, setGroupForm, savingGroup, openEditGroup, handleSaveGroup, handleDeleteGroup } =
    useGroupCrud(loadData)

  const [extendOpen, setExtendOpen] = useState(false)
  const [extendId, setExtendId] = useState(0)
  const [extendDays, setExtendDays] = useState("30")

  const handleExtendClick = useCallback((id: number) => {
    setExtendId(id)
    setExtendDays("30")
    setExtendOpen(true)
  }, [])

  return (
    <div className="space-y-6">
      <div className="flex flex-col justify-between gap-4 sm:flex-row sm:items-center">
        <div className="space-y-1">
          <h2 className="text-xl font-bold text-foreground">订阅套餐管理</h2>
          <p className="max-w-2xl text-sm text-muted-foreground">
            管理订阅套餐及其配额规则。用户开通订阅后可在周期内享受独立配额。
          </p>
        </div>

        <div className="flex gap-2">
          <Button variant="outline" className="gap-2" onClick={openCreateGroup}>
            <PackagePlus className="h-4 w-4" />
            新建套餐
          </Button>
          {groups.length > 0 && (
            <AdminSubscriptionAssignDialog groups={groups} onAssigned={loadData} />
          )}
        </div>
      </div>

      {/* ── Subscription Groups (套餐列表) ── */}
      {groups.length > 0 && (
        <div className="grid gap-3 md:grid-cols-2 lg:grid-cols-3">
          {groups.map(g => (
            <SubscriptionPackageCard
              key={g.id}
              group={g}
              onEdit={openEditGroup}
              onDelete={handleDeleteGroup}
            />
          ))}
        </div>
      )}

      {groups.length === 0 && !loading && (
        <EmptyState
          bordered
          tone="first-use"
          icon={PackagePlus}
          title="还没有订阅套餐"
          description="创建套餐后，用户即可在订阅页用余额开通对应周期的额度。"
          action={{ label: "新建套餐", onClick: openCreateGroup }}
        />
      )}

      {/* ── Subscription Table ── */}
      {groups.length > 0 && (
        <AdminSubscriptionsTable
          subs={subs}
          loading={loading}
          page={page}
          totalPages={totalPages}
          onPageChange={setPage}
          onExtend={handleExtendClick}
          onResetQuota={handleResetQuota}
          onRevoke={handleRevoke}
          onReactivate={handleReactivate}
        />
      )}

      {/* ── Extend Dialog ── */}
      <AdminSubscriptionExtendDialog
        open={extendOpen}
        onOpenChange={setExtendOpen}
        extendId={extendId}
        extendDays={extendDays}
        setExtendDays={setExtendDays}
        onExtend={handleExtend}
      />

      {/* ── Group Create/Edit Dialog ── */}
      <AdminSubscriptionGroupDialog
        open={groupDialogOpen}
        onOpenChange={setGroupDialogOpen}
        editingGroupId={editingGroupId}
        groupForm={groupForm}
        setGroupForm={setGroupForm}
        savingGroup={savingGroup}
        onSave={handleSaveGroup}
      />
    </div>
  )
}
