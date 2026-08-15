import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X, Play, Pause, Trash2 } from 'lucide-react'
import type { AuthFileDetailDialogProps } from '../types'
import { AuthFileQuotaPanel } from './AuthFileQuotaPanel'
import { AuthFileModelsPanel } from './AuthFileModelsPanel'

export function AuthFileDetailDialog({
  open,
  onOpenChange,
  item,
  onToggle,
  onDelete,
  quotaResults,
  quotaLoading,
  onLoadQuota,
  authModels,
  authModelsLoading,
  onLoadAuthModels,
  fmtRelative,
  stateInfo,
}: AuthFileDetailDialogProps) {
  if (!item) return null
  const si = stateInfo(item)

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-lg translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 max-h-[85vh] overflow-y-auto">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white">{item.label || item.name}</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            凭证详细信息
          </DialogPrimitive.Description>

          <div className="space-y-4 py-5">
            {/* Status Banner */}
            <div className={`flex items-center gap-3 rounded-xl border px-4 py-3 ${
              item.disabled
                ? 'border-gray-200 dark:border-dark-600 bg-gray-50/80 dark:bg-dark-800/70'
                : item.state === 'blocked' || item.status === 'error'
                  ? 'border-red-200 dark:border-red-800/60 bg-red-50/80 dark:bg-red-950/30'
                  : 'border-emerald-200 dark:border-emerald-800/60 bg-emerald-50/80 dark:bg-emerald-950/30'
            }`}>
              <div className={`inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium ${si.tone}`}>
                {si.icon} {si.label}
              </div>
              {item.status_message && (
                <span className="text-xs text-gray-600 dark:text-gray-300 truncate">{item.status_message}</span>
              )}
            </div>

            {/* Detail Grid */}
            <div className="grid grid-cols-2 gap-3">
              {[
                { label: '文件名', value: item.name },
                { label: '渠道', value: item.provider || '未知' },
                { label: '类型', value: item.type || item.auth_type || '—' },
                { label: '来源', value: item.source_kind || '—' },
                { label: '邮箱', value: item.email || '—' },
                { label: '成功请求', value: String(item.success ?? 0) },
                { label: '失败请求', value: String(item.failed ?? 0) },
                { label: '更新时间', value: fmtRelative(item.updated_at) },
              ].map(detail => (
                <div key={detail.label} className="rounded-lg border border-gray-200 dark:border-dark-600 bg-gray-50/80 dark:bg-dark-800/50 px-3 py-2">
                  <p className="text-[10px] text-gray-500 dark:text-dark-400 uppercase tracking-wider">{detail.label}</p>
                  <p className="mt-0.5 text-sm font-medium text-gray-900 dark:text-white truncate" title={detail.value}>{detail.value}</p>
                </div>
              ))}
            </div>

            {/* Quota Panel */}
            <AuthFileQuotaPanel
              item={item}
              quota={quotaResults[item.name]}
              loading={quotaLoading[item.name]}
              onRefresh={() => onLoadQuota(item)}
            />

            {/* Registered Models Panel */}
            <AuthFileModelsPanel
              item={item}
              models={authModels[item.name]}
              loading={authModelsLoading[item.name]}
              onRefresh={() => onLoadAuthModels(item)}
            />

            {/* Action Buttons */}
            <div className="grid grid-cols-2 gap-2">
              <button
                className={`btn text-sm rounded-xl ${item.disabled ? 'btn-success' : 'btn-warning'}`}
                onClick={() => { onToggle(item); onOpenChange(false) }}
              >
                {item.disabled ? <><Play className="h-4 w-4" /> 恢复调度</> : <><Pause className="h-4 w-4" /> 暂停调度</>}
              </button>
              <button
                className="btn btn-danger text-sm rounded-xl"
                onClick={() => { onOpenChange(false); onDelete(item) }}
              >
                <Trash2 className="h-4 w-4" /> 删除凭证
              </button>
            </div>
          </div>

          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-md p-1 opacity-70 hover:opacity-100 transition-opacity">
            <X className="h-4 w-4" />
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
