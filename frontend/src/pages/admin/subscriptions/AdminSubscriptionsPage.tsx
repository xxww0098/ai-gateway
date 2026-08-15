import { useState, useCallback } from "react"
import { Button } from "@/shared/components/ui/button"
import { Card, CardContent } from "@/shared/components/ui/card"
import { Crown, PackagePlus } from "lucide-react"
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
    <div className="space-y-6 animate-in fade-in duration-500" style={{ willChange: 'transform, opacity' }}>
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div className="space-y-1">
          <h3 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <Crown className="w-5 h-5 text-amber-500" />
            订阅管理
          </h3>
          <p className="text-sm text-gray-500 max-w-2xl">
            管理用户的月卡/套餐订阅。订阅类型的分组按5h/周/月限额计费，不扣用户余额。
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
        <Card className="border-dashed border-2 border-amber-200 dark:border-amber-800/40 bg-amber-50/30 dark:bg-amber-950/10">
          <CardContent className="flex flex-col items-center justify-center py-12 text-center">
            <PackagePlus className="w-12 h-12 text-amber-300 dark:text-amber-700 mb-4" />
            <h4 className="text-lg font-semibold text-gray-700 dark:text-gray-300 mb-2">尚未创建订阅套餐</h4>
            <p className="text-sm text-gray-500 max-w-md mb-4">
              创建一个订阅类型的分组来启用月卡/套餐功能。订阅分组按5h/周/月限额计费，不扣用户余额。
            </p>
            <Button className="gap-2 bg-amber-600 hover:bg-amber-700 text-white" onClick={openCreateGroup}>
              <PackagePlus className="h-4 w-4" />
              创建第一个订阅套餐
            </Button>
          </CardContent>
        </Card>
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
