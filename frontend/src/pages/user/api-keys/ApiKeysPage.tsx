import { useApiKeys, CreateApiKeyDialog, ApiKeysTable, NeedSubscriptionDialog } from "@/features/user-api-keys"

export default function Keys() {
  const {
    keys,
    loading,
    copiedId,
    groups,
    groupsLoading,
    rebindingId,
    needSubDialog,
    setNeedSubDialog,
    handleCreate,
    handleDelete,
    handleCopy,
    handleRebindGroup,
  } = useApiKeys()

  return (
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500" style={{ willChange: 'transform, opacity' }}>
      <div className="flex flex-col sm:flex-row justify-between items-start sm:items-center gap-4">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">API 密钥</h2>
          <p className="text-gray-500 dark:text-dark-300 mt-1">管理您用于访问接口的身份凭证及用量限制。</p>
        </div>

        <CreateApiKeyDialog onCreate={handleCreate} groups={groups} groupsLoading={groupsLoading} />
      </div>

      <ApiKeysTable
        keys={keys}
        loading={loading}
        copiedId={copiedId}
        onCopy={handleCopy}
        onDelete={handleDelete}
        groups={groups}
        groupsLoading={groupsLoading}
        rebindingId={rebindingId}
        onRebindGroup={handleRebindGroup}
      />

      <NeedSubscriptionDialog
        open={needSubDialog.open}
        onOpenChange={(open) => setNeedSubDialog((s) => ({ ...s, open }))}
        groupName={needSubDialog.groupName}
      />
    </div>
  )
}
