import { useState, useCallback, useMemo, useRef } from 'react'
import { buildAuthFilesFormData } from '@/features/admin-proxy/authFileImportUtils'
import { getBatchStatusTargets, providerBadgeColor } from '@/features/admin-proxy/authFileViewUtils'
import { fetchQuotaForFile, type QuotaResult } from '@/features/usage/quota'
import { modelsApi } from '@/features/pricing/model_prices'
import { toast } from 'sonner'
import { ShieldCheck, ShieldX, ShieldAlert } from 'lucide-react'
import type { AuthFileEditFields, AuthFileItem, SmartView } from '@/features/admin-proxy/types'
import { AuthFileToolbar } from '@/features/admin-proxy/components/AuthFileToolbar'
import { AuthFileTable } from '@/features/admin-proxy/components/AuthFileTable'
import { AuthFileUploadDialog } from '@/features/admin-proxy/components/AuthFileUploadDialog'
import { AuthFileDeleteDialog } from '@/features/admin-proxy/components/AuthFileDeleteDialog'
import { AuthFileBatchDeleteDialog } from '@/features/admin-proxy/components/AuthFileBatchDeleteDialog'
import { AuthFileDetailDialog } from '@/features/admin-proxy/components/AuthFileDetailDialog'
import { AuthFileEditDialog } from '@/features/admin-proxy/components/AuthFileEditDialog'
import {
  useAuthFiles,
  useToggleAuthFile,
  useDeleteAuthFile,
  useUploadAuthFile,
  useUpdateAuthFile,
  useDownloadAuthFile,
  useExportAuthFiles,
  useBatchToggleAuthFiles,
  useBatchDeleteAuthFiles,
} from '@/features/admin-proxy/hooks'
import type { SDKModelDefinition } from '@/features/pricing/model_prices'

function authFileTarget(file: AuthFileItem): string {
  return file.id || file.auth_id || file.name
}

// ── Helpers ─────────────────────────────────────────────────────────────────

function isWarning(f: AuthFileItem) {
  return !f.disabled && (f.status === 'error' || f.status === 'offline' || f.state === 'cooling' || f.state === 'blocked' || !!f.status_message)
}

function isHealthy(f: AuthFileItem) {
  return !f.disabled && !isWarning(f)
}

function isDisabled(f: AuthFileItem) {
  return !!f.disabled
}

function fmtRelative(dateStr?: string | null) {
  if (!dateStr) return '—'
  try {
    const ts = new Date(dateStr).getTime()
    if (isNaN(ts)) return String(dateStr)
    const diffSec = Math.round((ts - Date.now()) / 1000)
    const abs = Math.abs(diffSec)
    const fmt = new Intl.RelativeTimeFormat('zh-CN', { numeric: 'auto' })
    if (abs < 60) return fmt.format(diffSec, 'second')
    if (abs < 3600) return fmt.format(Math.round(diffSec / 60), 'minute')
    if (abs < 86400) return fmt.format(Math.round(diffSec / 3600), 'hour')
    return fmt.format(Math.round(diffSec / 86400), 'day')
  } catch { return String(dateStr) }
}

function stateInfo(f: AuthFileItem) {
  if (f.disabled) return { icon: <ShieldX className="h-3.5 w-3.5" />, label: '已停用', tone: 'bg-gray-100 text-gray-600 dark:bg-dark-700 dark:text-gray-400' }

  const msg = (f.status_message || '').toLowerCase()
  if (f.state === 'blocked' || f.status === 'error' || msg.includes('401') || msg.includes('失效') || msg.includes('error')) {
    return { icon: <ShieldX className="h-3.5 w-3.5" />, label: '异常', tone: 'bg-red-100 text-red-700 dark:bg-red-950/40 dark:text-red-300' }
  }

  if (f.state === 'cooling' || f.status === 'offline' || !!f.status_message) {
    return { icon: <ShieldAlert className="h-3.5 w-3.5" />, label: '异常', tone: 'bg-amber-100 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300' }
  }

  return { icon: <ShieldCheck className="h-3.5 w-3.5" />, label: '正常', tone: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300' }
}

// ── Page Component ──────────────────────────────────────────────────────────

export default function AdminProxyAuthFilesPage() {
  // ── Data fetching via hooks ──
  const { data: authFiles = [], isLoading: loading, refetch } = useAuthFiles()
  const toggleMutation = useToggleAuthFile()
  const deleteMutation = useDeleteAuthFile()
  const uploadMutation = useUploadAuthFile()
  const updateMutation = useUpdateAuthFile()
  const downloadMutation = useDownloadAuthFile()
  const exportMutation = useExportAuthFiles()
  const batchToggleMutation = useBatchToggleAuthFiles()
  const batchDeleteMutation = useBatchDeleteAuthFiles()

  const [refreshing, setRefreshing] = useState(false)

  // Filters
  const [searchQuery, setSearchQuery] = useState('')
  const [filterProvider, setFilterProvider] = useState('all')
  const [smartView, setSmartView] = useState<SmartView>('all')

  // Upload dialog
  const [showUpload, setShowUpload] = useState(false)
  const [uploadText, setUploadText] = useState('')
  const [uploadPickedFiles, setUploadPickedFiles] = useState<File[]>([])

  // Delete dialog
  const [showDelete, setShowDelete] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<AuthFileItem | null>(null)

  // Detail dialog
  const [showDetail, setShowDetail] = useState(false)
  const [detailItem, setDetailItem] = useState<AuthFileItem | null>(null)

  // Edit dialog
  const [showEdit, setShowEdit] = useState(false)
  const [editTarget, setEditTarget] = useState<AuthFileItem | null>(null)

  // Per-row download loading state
  const [downloadLoading, setDownloadLoading] = useState<Record<string, boolean>>({})

  // Selection
  const [selectedNames, setSelectedNames] = useState<Set<string>>(new Set())
  const [showBatchDelete, setShowBatchDelete] = useState(false)

  // Quota
  const quotaCache = useRef<Record<string, { result: QuotaResult; ts: number }>>({})
  const [quotaLoading, setQuotaLoading] = useState<Record<string, boolean>>({})
  const [quotaResults, setQuotaResults] = useState<Record<string, QuotaResult>>({})

  // Registered models
  const [authModels, setAuthModels] = useState<Record<string, SDKModelDefinition[]>>({})
  const [authModelsLoading, setAuthModelsLoading] = useState<Record<string, boolean>>({})

  // ── Computed ──
  const providers = useMemo(() => {
    const set = new Set(authFiles.map(f => f.provider || 'unknown').filter(Boolean))
    return Array.from(set).sort()
  }, [authFiles])

  const counts = useMemo(() => ({
    all: authFiles.length,
    healthy: authFiles.filter(isHealthy).length,
    warning: authFiles.filter(isWarning).length,
    disabled: authFiles.filter(isDisabled).length,
  }), [authFiles])

  const processedFiles = useMemo(() => {
    let items = [...authFiles]

    if (smartView === 'healthy') items = items.filter(isHealthy)
    else if (smartView === 'warning') items = items.filter(isWarning)
    else if (smartView === 'disabled') items = items.filter(isDisabled)

    if (filterProvider !== 'all') {
      items = items.filter(f => (f.provider || 'unknown') === filterProvider)
    }

    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase()
      items = items.filter(f =>
        f.name?.toLowerCase().includes(q) ||
        f.email?.toLowerCase().includes(q) ||
        f.label?.toLowerCase().includes(q) ||
        f.provider?.toLowerCase().includes(q)
      )
    }

    return items
  }, [authFiles, smartView, filterProvider, searchQuery])

  const selectedVisibleCount = processedFiles.filter(f => selectedNames.has(f.name)).length
  const allSelected = processedFiles.length > 0 && selectedVisibleCount === processedFiles.length
  const indeterminate = selectedVisibleCount > 0 && selectedVisibleCount < processedFiles.length
  const batchStatusTargets = useMemo(
    () => getBatchStatusTargets(authFiles, selectedNames),
    [authFiles, selectedNames]
  )

  // ── Actions ──
  const handleSelectAll = (checked: boolean) => {
    if (checked) {
      setSelectedNames(new Set(processedFiles.map(f => f.name)))
    } else {
      setSelectedNames(new Set())
    }
  }

  const handleSelectOne = (name: string, checked: boolean) => {
    setSelectedNames(prev => {
      const next = new Set(prev)
      if (checked) next.add(name)
      else next.delete(name)
      return next
    })
  }

  const handleToggle = (file: AuthFileItem) => {
    toggleMutation.mutate({ name: file.name, disabled: !file.disabled })
  }

  const handleRefreshList = useCallback(async () => {
    setRefreshing(true)
    try {
      await refetch()
      toast.success('已同步 SDK 凭证列表')
    } finally {
      setRefreshing(false)
    }
  }, [refetch])

  const openDelete = (file: AuthFileItem) => {
    setDeleteTarget(file)
    setShowDelete(true)
  }

  const handleDelete = () => {
    if (!deleteTarget) return
    deleteMutation.mutate(deleteTarget.name, {
      onSuccess: () => {
        setShowDelete(false)
        setSelectedNames(prev => {
          const next = new Set(prev)
          next.delete(deleteTarget.name)
          return next
        })
      },
    })
  }

  const handleBatchStatus = (disabled: boolean) => {
    const targets = disabled ? batchStatusTargets.pauseTargets : batchStatusTargets.resumeTargets
    if (targets.length === 0) return
    batchToggleMutation.mutate({ files: targets, disabled }, {
      onSuccess: () => setSelectedNames(new Set()),
    })
  }

  const handleBatchDelete = () => {
    if (selectedNames.size === 0) return
    batchDeleteMutation.mutate(Array.from(selectedNames), {
      onSuccess: () => {
        setSelectedNames(new Set())
        setShowBatchDelete(false)
      },
    })
  }

  const openEdit = (file: AuthFileItem) => {
    if (file.runtime_only === true) {
      toast.error('运行时凭证由 config.yaml 注入，无法在此处编辑')
      return
    }
    setEditTarget(file)
    setShowEdit(true)
  }

  const handleEditSave = (fields: AuthFileEditFields) => {
    if (!editTarget) return
    const id = authFileTarget(editTarget)
    if (!id) {
      toast.error('凭证缺少 ID，无法保存')
      return
    }
    updateMutation.mutate(
      { id, fields: fields as Record<string, unknown> },
      {
        onSuccess: () => {
          setShowEdit(false)
          setEditTarget(null)
        },
      }
    )
  }

  const handleDownload = (file: AuthFileItem) => {
    const target = authFileTarget(file)
    if (!target) {
      toast.error('凭证缺少 ID，无法下载')
      return
    }
    setDownloadLoading((prev) => ({ ...prev, [file.name]: true }))
    downloadMutation.mutate(
      { id: file.id, name: target },
      {
        onSettled: () => setDownloadLoading((prev) => ({ ...prev, [file.name]: false })),
      }
    )
  }

  const handleBatchExport = () => {
    const ids = Array.from(selectedNames)
      .map((name) => {
        const file = authFiles.find((f) => f.name === name)
        return file ? authFileTarget(file) : ''
      })
      .filter((id) => id !== '')
    if (ids.length === 0) {
      toast.error('请选择要导出的凭证')
      return
    }
    exportMutation.mutate(ids)
  }

  const handleUpload = () => {
    const text = uploadText.trim()
    const hasFiles = uploadPickedFiles.length > 0
    if (!hasFiles && !text) {
      toast.error('请粘贴凭证内容或选择文件')
      return
    }

    let formData: FormData
    if (hasFiles) {
      formData = buildAuthFilesFormData(uploadPickedFiles)
    } else {
      formData = new FormData()
      formData.append('file', new File([text], 'pasted-import.json', { type: 'application/json' }))
    }

    uploadMutation.mutate(formData, {
      onSuccess: () => {
        setShowUpload(false)
        setUploadText('')
        setUploadPickedFiles([])
      },
    })
  }

  // ── Quota & Models ──
  const loadQuota = async (file: AuthFileItem) => {
    const cached = quotaCache.current[file.name]
    if (cached && Date.now() - cached.ts < 60_000) {
      setQuotaResults(prev => ({ ...prev, [file.name]: cached.result }))
      return
    }
    setQuotaLoading(prev => ({ ...prev, [file.name]: true }))
    try {
      const result = await fetchQuotaForFile(file)
      quotaCache.current[file.name] = { result, ts: Date.now() }
      setQuotaResults(prev => ({ ...prev, [file.name]: result }))
    } catch (e: unknown) {
      const err = e instanceof Error ? e.message : '查询失败'
      setQuotaResults(prev => ({ ...prev, [file.name]: { provider: file.provider || '', windows: [], error: err } }))
    } finally {
      setQuotaLoading(prev => ({ ...prev, [file.name]: false }))
    }
  }

  const loadAuthModels = async (file: AuthFileItem) => {
    const key = file.name
    if (!key) return
    setAuthModelsLoading(prev => ({ ...prev, [key]: true }))
    try {
      const models = await modelsApi.fetchAuthFileModels(key)
      setAuthModels(prev => ({ ...prev, [key]: models }))
    } catch (e: unknown) {
      const err = e instanceof Error ? e.message : '查询失败'
      toast.error(`获取已注册模型失败: ${err}`)
      setAuthModels(prev => ({ ...prev, [key]: [] }))
    } finally {
      setAuthModelsLoading(prev => ({ ...prev, [key]: false }))
    }
  }

  const openDetail = (file: AuthFileItem) => {
    setDetailItem(file)
    setShowDetail(true)
    if ((file.auth_index ?? file.authIndex) !== undefined) {
      loadQuota(file)
    }
    if (!authModels[file.name]) {
      loadAuthModels(file)
    }
  }

  return (
    <div className="space-y-4">
      <AuthFileToolbar
        smartView={smartView}
        onSmartViewChange={setSmartView}
        filterProvider={filterProvider}
        onFilterProviderChange={setFilterProvider}
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
        providers={providers}
        counts={counts}
        selectedCount={selectedNames.size}
        selectedPausableCount={batchStatusTargets.pauseTargets.length}
        selectedResumableCount={batchStatusTargets.resumeTargets.length}
        batchStatusLoading={batchToggleMutation.isPending}
        exportLoading={exportMutation.isPending}
        onBatchPause={() => { handleBatchStatus(true) }}
        onBatchResume={() => { handleBatchStatus(false) }}
        onBatchDelete={() => setShowBatchDelete(true)}
        onBatchExport={handleBatchExport}
        onRefresh={handleRefreshList}
        refreshing={refreshing}
        onUpload={() => {
          setUploadText('')
          setUploadPickedFiles([])
          setShowUpload(true)
        }}
      />

      <AuthFileTable
        files={processedFiles}
        selectedNames={selectedNames}
        allSelected={allSelected}
        indeterminate={indeterminate}
        loading={loading}
        totalCount={authFiles.length}
        onSelectAll={handleSelectAll}
        onSelectOne={handleSelectOne}
        onToggle={handleToggle}
        onDelete={openDelete}
        onDetail={openDetail}
        onEdit={openEdit}
        onDownload={handleDownload}
        onLoadQuota={loadQuota}
        quotaLoading={quotaLoading}
        downloadLoading={downloadLoading}
        providerBadgeColor={providerBadgeColor}
        fmtRelative={fmtRelative}
        stateInfo={stateInfo}
      />

      <AuthFileUploadDialog
        key={showUpload ? 'upload-open' : 'upload-closed'}
        open={showUpload}
        onOpenChange={o => {
          setShowUpload(o)
          if (!o) {
            setUploadText('')
            setUploadPickedFiles([])
          }
        }}
        onUpload={handleUpload}
        uploading={uploadMutation.isPending}
        value={uploadText}
        onValueChange={setUploadText}
        pickedFiles={uploadPickedFiles}
        onPickedFilesChange={setUploadPickedFiles}
      />

      <AuthFileBatchDeleteDialog
        open={showBatchDelete}
        onOpenChange={setShowBatchDelete}
        onConfirm={handleBatchDelete}
        deleting={batchDeleteMutation.isPending}
        count={selectedNames.size}
      />

      <AuthFileDeleteDialog
        open={showDelete}
        onOpenChange={setShowDelete}
        onConfirm={handleDelete}
        deleting={deleteMutation.isPending}
        item={deleteTarget}
      />

      <AuthFileEditDialog
        open={showEdit}
        onOpenChange={(open) => {
          setShowEdit(open)
          if (!open) setEditTarget(null)
        }}
        item={editTarget}
        saving={updateMutation.isPending}
        onSave={handleEditSave}
      />

      <AuthFileDetailDialog
        open={showDetail}
        onOpenChange={setShowDetail}
        item={detailItem}
        onToggle={handleToggle}
        onDelete={openDelete}
        quotaResults={quotaResults}
        quotaLoading={quotaLoading}
        onLoadQuota={loadQuota}
        authModels={authModels}
        authModelsLoading={authModelsLoading}
        onLoadAuthModels={loadAuthModels}
        fmtRelative={fmtRelative}
        stateInfo={stateInfo}
        providerBadgeColor={providerBadgeColor}
      />
    </div>
  )
}
