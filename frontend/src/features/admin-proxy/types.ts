import type { SDKModelDefinition } from '@/features/pricing/model_prices'
import type { QuotaResult } from '@/features/usage/quota'

// ── API response/request types ────────────────────────────────────────────

/** Response from GET /auth-files */
export interface AuthFilesResponse {
  files?: AuthFileItem[]
  'auth-files'?: AuthFileItem[]
  authFiles?: AuthFileItem[]
}

/** Request body for PATCH /auth-files/status */
export interface AuthFileStatusRequest {
  name: string
  disabled: boolean
}

/** Response from POST /auth-files (multipart upload) */
export interface AuthFileUploadResponse {
  status?: string
  uploaded?: number
  failed?: Array<{ name: string; error: string }>
  [key: string]: unknown
}

// ── AuthFile types ────────────────────────────────────────────────

export interface AuthFileItem {
  name: string
  id?: string
  auth_id?: string
  provider?: string
  type?: string
  size?: string | number
  status?: string
  status_message?: string
  disabled?: boolean
  runtime_only?: boolean
  success?: number
  failed?: number
  recent_requests?: Array<{ success?: number; failed?: number; start?: string; end?: string }>
  email?: string
  label?: string
  prefix?: string
  proxy_url?: string
  base_url?: string
  account_id?: string
  api_key_preview?: string
  access_token_preview?: string
  refresh_token_preview?: string
  has_api_key?: boolean
  has_access_token?: boolean
  has_refresh_token?: boolean
  has_service_account?: boolean
  updated_at?: string
  source_kind?: string
  auth_type?: string
  auth_index?: string | number
  authIndex?: string | number
  chatgpt_account_id?: string
  project_id?: string
  location?: string
  state?: string
  models?: string[]
  [key: string]: unknown
}

export interface AuthFileEditFields {
  label?: string
  prefix?: string
  proxy_url?: string
  base_url?: string
  project_id?: string
  location?: string
  api_key?: string
  access_token?: string
  refresh_token?: string
  id_token?: string
  service_account?: string
}

export type SmartView = 'all' | 'healthy' | 'warning' | 'disabled'

// ── Component prop types ────────────────────────────────────────

export interface AuthFileToolbarProps {
  smartView: SmartView
  onSmartViewChange: (v: SmartView) => void
  filterProvider: string
  onFilterProviderChange: (p: string) => void
  searchQuery: string
  onSearchChange: (q: string) => void
  providers: string[]
  counts: { all: number; healthy: number; warning: number; disabled: number }
  selectedCount: number
  selectedPausableCount: number
  selectedResumableCount: number
  batchStatusLoading: boolean
  exportLoading: boolean
  onBatchPause: () => void
  onBatchResume: () => void
  onBatchDelete: () => void
  onBatchExport: () => void
  onRefresh: () => void
  refreshing: boolean
  onUpload: () => void
}

export interface AuthFileTableProps {
  files: AuthFileItem[]
  selectedNames: Set<string>
  allSelected: boolean
  indeterminate: boolean
  loading: boolean
  totalCount: number
  onSelectAll: (checked: boolean) => void
  onSelectOne: (name: string, checked: boolean) => void
  onToggle: (file: AuthFileItem) => void
  onDelete: (file: AuthFileItem) => void
  onDetail: (file: AuthFileItem) => void
  onEdit: (file: AuthFileItem) => void
  onDownload: (file: AuthFileItem) => void
  onLoadQuota: (file: AuthFileItem) => void
  quotaLoading: Record<string, boolean>
  downloadLoading: Record<string, boolean>
  providerBadgeColor: (p?: string) => string
  fmtRelative: (d?: string | null) => string
  stateInfo: (f: AuthFileItem) => { icon: React.ReactNode; label: string; tone: string }
}

export interface AuthFileUploadDialogProps {
  open: boolean
  onOpenChange: (o: boolean) => void
  onUpload: () => void
  uploading: boolean
  value: string
  onValueChange: (v: string) => void
  /** 多文件上传（与粘贴二选一，由对话框写入） */
  pickedFiles: File[]
  onPickedFilesChange: (files: File[]) => void
}

export interface AuthFileDeleteDialogProps {
  open: boolean
  onOpenChange: (o: boolean) => void
  onConfirm: () => void
  deleting: boolean
  item: AuthFileItem | null
}

export interface AuthFileBatchDeleteDialogProps {
  open: boolean
  onOpenChange: (o: boolean) => void
  onConfirm: () => void
  deleting: boolean
  count: number
}

export interface AuthFileDetailDialogProps {
  open: boolean
  onOpenChange: (o: boolean) => void
  item: AuthFileItem | null
  onToggle: (file: AuthFileItem) => void
  onDelete: (file: AuthFileItem) => void
  quotaResults: Record<string, QuotaResult>
  quotaLoading: Record<string, boolean>
  onLoadQuota: (file: AuthFileItem) => void
  authModels: Record<string, SDKModelDefinition[]>
  authModelsLoading: Record<string, boolean>
  onLoadAuthModels: (file: AuthFileItem) => void
  fmtRelative: (d?: string | null) => string
  stateInfo: (f: AuthFileItem) => { icon: React.ReactNode; label: string; tone: string }
  providerBadgeColor: (p?: string) => string
}

export interface AuthFileQuotaPanelProps {
  item: AuthFileItem
  quota: QuotaResult | undefined
  loading: boolean
  onRefresh: () => void
}

export interface AuthFileModelsPanelProps {
  item: AuthFileItem
  models: SDKModelDefinition[] | undefined
  loading: boolean
  onRefresh: () => void
}
