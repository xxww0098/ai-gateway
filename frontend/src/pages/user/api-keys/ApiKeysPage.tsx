import {
  useApiKeys,
  CreateApiKeyDialog,
  ApiKeysTable,
  NeedSubscriptionDialog,
  GatewayEndpointBar,
  ApiKeysStatsBar,
} from "@/features/user-api-keys"
import { Plus, BookOpen, KeyRound } from "lucide-react"
import { Link } from "react-router-dom"
import { docsPath } from "@/shared/routes/docs"

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
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div className="space-y-1 max-w-2xl">
          <div className="flex items-center gap-2.5">
            <h1 className="text-xl sm:text-2xl font-bold tracking-tight text-foreground">
              API 密钥
            </h1>
            <span className="inline-flex items-center gap-1 rounded-full border border-border bg-muted/60 px-2.5 py-0.5 text-xs font-medium text-muted-foreground">
              <KeyRound className="h-3 w-3" />
              <span>凭证管理</span>
            </span>
          </div>
          <p className="text-xs sm:text-sm text-muted-foreground leading-relaxed">
            管理用于访问 AI-GateWay 的调用凭证。所有密钥均以 <code className="font-mono text-primary font-medium">agw-</code> 开头，同一把密钥可走 Chat Completions、Responses 与 Anthropic Messages。
          </p>
        </div>

        <div className="flex items-center gap-2.5 shrink-0">
          <Link
            to={docsPath('quickstart')}
            className="inline-flex items-center gap-1.5 rounded-xl border border-border bg-card px-3.5 py-2 text-xs sm:text-sm font-medium text-foreground hover:bg-muted transition-colors shadow-2xs"
          >
            <BookOpen className="h-4 w-4 text-muted-foreground" />
            <span>接入文档</span>
          </Link>
          <CreateApiKeyDialog
            onCreate={handleCreate}
            groups={groups}
            groupsLoading={groupsLoading}
            trigger={
              <button className="btn btn-primary px-4 py-2 text-xs sm:text-sm font-medium shadow-glow flex items-center gap-1.5">
                <Plus className="h-4 w-4" />
                <span>新建 Key</span>
              </button>
            }
          />
        </div>
      </div>

      <GatewayEndpointBar />

      {/* Quick Summary Metrics when keys exist */}
      {!loading && keys.length > 0 && <ApiKeysStatsBar keys={keys} />}

      {/* Main Table or Guided Onboarding Empty State */}
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
        emptyAction={
          <CreateApiKeyDialog
            onCreate={handleCreate}
            groups={groups}
            groupsLoading={groupsLoading}
            trigger={
              <button className="btn btn-primary px-5 py-2.5 text-sm font-semibold shadow-glow flex items-center gap-2">
                <Plus className="h-4 w-4" />
                <span>创建第一把 API Key</span>
              </button>
            }
          />
        }
      />

      <NeedSubscriptionDialog
        open={needSubDialog.open}
        onOpenChange={(open) => setNeedSubDialog((s) => ({ ...s, open }))}
        groupName={needSubDialog.groupName}
      />
    </div>
  )
}
