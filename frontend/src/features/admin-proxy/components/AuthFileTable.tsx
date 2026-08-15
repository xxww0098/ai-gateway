import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu'
import {
  RefreshCw, Trash2, ShieldCheck,
  Clock, MoreHorizontal, Play, Pause, AlertTriangle,
  FileJson, Activity, Download, Pencil
} from 'lucide-react'
import { resolveAuthFileIdentity } from '../authFileViewUtils'
import type { AuthFileTableProps } from '../types'
import { AuthProviderBrandIcon } from './AuthProviderBrandIcon'

export function AuthFileTable({
  files,
  selectedNames,
  allSelected,
  indeterminate,
  loading,
  totalCount,
  onSelectAll,
  onSelectOne,
  onToggle,
  onDelete,
  onDetail,
  onEdit,
  onDownload,
  onLoadQuota,
  quotaLoading,
  downloadLoading,
  providerBadgeColor,
  fmtRelative,
  stateInfo,
}: AuthFileTableProps) {
  return (
    <div className="rounded-2xl border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 shadow-sm">
      <div className="overflow-x-auto">
        <table className="w-full min-w-[980px] table-fixed">
          <colgroup>
            <col className="w-[52px]" />
            <col className="w-[72px]" />
            <col className="w-[380px]" />
            <col className="w-[130px]" />
            <col className="w-[120px]" />
            <col className="w-[130px]" />
            <col className="w-[150px]" />
            <col className="w-[64px]" />
          </colgroup>
          <thead>
            <tr className="border-b border-gray-200 dark:border-dark-600 bg-gray-50/80 dark:bg-dark-800/60">
              <th className="h-9 px-4 text-left">
                <div className="flex items-center">
                  <input
                    type="checkbox"
                    className="h-3.5 w-3.5 rounded border-gray-300 text-primary-600 focus:ring-primary-500 dark:border-dark-600 dark:bg-dark-800 dark:ring-offset-dark-900 cursor-pointer"
                    checked={files.length > 0 && allSelected}
                    ref={input => {
                      if (input) {
                        input.indeterminate = indeterminate
                      }
                    }}
                    onChange={e => onSelectAll(e.target.checked)}
                  />
                </div>
              </th>
              <th className="h-9 px-4 text-left text-[11px] font-medium uppercase tracking-[0.06em] text-gray-500 dark:text-dark-300">启用</th>
              <th className="h-9 px-4 text-left text-[11px] font-medium uppercase tracking-[0.06em] text-gray-500 dark:text-dark-300">凭证</th>
              <th className="h-9 px-4 text-left text-[11px] font-medium uppercase tracking-[0.06em] text-gray-500 dark:text-dark-300">
                <span className="inline-flex items-center gap-1"><ShieldCheck className="h-3.5 w-3.5" /> 状态</span>
              </th>
              <th className="h-9 px-4 text-left text-[11px] font-medium uppercase tracking-[0.06em] text-gray-500 dark:text-dark-300">
                <span className="inline-flex items-center gap-1"><Activity className="h-3.5 w-3.5" /> 请求</span>
              </th>
              <th className="h-9 px-4 text-left text-[11px] font-medium uppercase tracking-[0.06em] text-gray-500 dark:text-dark-300">
                <span className="inline-flex items-center gap-1"><RefreshCw className="h-3.5 w-3.5" /> 渠道</span>
              </th>
              <th className="h-9 px-4 text-left text-[11px] font-medium uppercase tracking-[0.06em] text-gray-500 dark:text-dark-300">
                <span className="inline-flex items-center gap-1"><Clock className="h-3.5 w-3.5" /> 更新时间</span>
              </th>
              <th className="h-9 px-4"></th>
            </tr>
          </thead>
          <tbody>
            {loading && files.length === 0 ? (
              <tr>
                <td colSpan={8} className="h-40 text-center">
                  <RefreshCw className="h-5 w-5 animate-spin mx-auto text-primary-500" />
                </td>
              </tr>
            ) : files.length === 0 ? (
              <tr>
                <td colSpan={8} className="h-40 text-center">
                  <div className="space-y-2">
                    <FileJson className="h-8 w-8 mx-auto text-gray-300 dark:text-dark-500" />
                    <p className="text-sm text-gray-500 dark:text-dark-400">
                      {totalCount === 0 ? '暂无凭证文件，请通过 OAuth 登录或手动导入' : '无匹配结果，请调整筛选条件'}
                    </p>
                  </div>
                </td>
              </tr>
            ) : files.map((file, i) => {
              const si = stateInfo(file)
              const identity = resolveAuthFileIdentity(file)
              return (
                <tr
                  key={file.name || i}
                  className={`group cursor-pointer border-b border-gray-100 dark:border-dark-700 transition-all hover:bg-gray-50/70 dark:hover:bg-dark-800/30 ${
                    file.disabled ? 'opacity-60' : ''
                  }`}
                  onClick={() => onDetail(file)}
                >
                  {/* Checkbox */}
                  <td className="px-4 py-2.5" onClick={e => e.stopPropagation()}>
                    <div className="flex items-center mt-0.5">
                      <input
                        type="checkbox"
                        className="h-3.5 w-3.5 rounded border-gray-300 text-primary-600 focus:ring-primary-500 dark:border-dark-600 dark:bg-dark-800 dark:ring-offset-dark-900 cursor-pointer"
                        checked={selectedNames.has(file.name)}
                        onChange={e => onSelectOne(file.name, e.target.checked)}
                      />
                    </div>
                  </td>

                  {/* Toggle */}
                  <td className="px-4 py-2.5" onClick={e => e.stopPropagation()}>
                    <button
                      onClick={() => onToggle(file)}
                      className={`relative h-5 w-9 rounded-full transition-colors ${
                        !file.disabled ? 'bg-primary-500' : 'bg-gray-300 dark:bg-dark-600'
                      }`}
                    >
                      <span className={`absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform ${
                        !file.disabled ? 'translate-x-4' : ''
                      }`} />
                    </button>
                  </td>

                  {/* Account Info */}
                  <td className="px-4 py-2.5">
                    <div className="flex items-start gap-2.5 min-w-0">
                      <div
                        className="h-7 w-7 rounded-lg flex items-center justify-center flex-shrink-0 mt-0.5 bg-gray-50 dark:bg-dark-800 border border-gray-100 dark:border-dark-700"
                        title={file.provider || undefined}
                      >
                        <AuthProviderBrandIcon provider={file.provider} size={20} />
                      </div>
                      <div className="min-w-0 max-w-[320px]">
                        <div className="truncate font-medium text-[13px] leading-5 text-gray-900 dark:text-white" title={identity.primaryTitle}>
                          {identity.primary}
                        </div>
                        <div className="mt-0.5 flex flex-wrap items-center gap-1 text-[9px] leading-none">
                          {identity.email && (
                            <span className="inline-flex max-w-[220px] items-center truncate rounded-md bg-gray-100 px-1 py-0.5 text-gray-500 dark:bg-dark-700 dark:text-gray-400" title={identity.email}>
                              {identity.email}
                            </span>
                          )}
                          {identity.sourceKind && (
                            <span className="inline-flex items-center rounded-md bg-gray-100 dark:bg-dark-700 px-1 py-0.5 text-gray-500 dark:text-gray-400 capitalize">
                              {identity.sourceKind}
                            </span>
                          )}
                          {identity.hasStatusMessage && (
                            <span className="inline-flex items-center rounded-md bg-amber-100 dark:bg-amber-950/40 px-1 py-0.5 text-amber-700 dark:text-amber-300" title={file.status_message}>
                              <AlertTriangle className="h-2.5 w-2.5" />
                            </span>
                          )}
                        </div>
                      </div>
                    </div>
                  </td>

                  {/* State */}
                  <td className="px-4 py-2.5">
                    <div className={`inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[10px] font-medium ${si.tone}`}>
                      {si.icon}
                      <span className="capitalize">{si.label}</span>
                    </div>
                  </td>

                  {/* Usage */}
                  <td className="px-4 py-2.5">
                    <div className="text-[11px] tabular-nums text-gray-500 dark:text-gray-400">
                      <span className="text-emerald-600 dark:text-emerald-400">{file.success ?? 0}</span>
                      <span className="mx-1">/</span>
                      <span className="text-red-500 dark:text-red-400">{file.failed ?? 0}</span>
                    </div>
                  </td>

                  {/* Provider */}
                  <td className="px-4 py-2.5">
                    <span className={`inline-flex items-center rounded-md px-2 py-0.5 text-[11px] font-medium capitalize ${providerBadgeColor(file.provider)}`}>
                      {file.provider || '未知'}
                    </span>
                  </td>

                  {/* Updated */}
                  <td className="px-4 py-2.5">
                    <div className="inline-flex items-center gap-1 text-xs text-gray-500 dark:text-gray-400">
                      <Clock className="h-3 w-3 text-gray-400 dark:text-dark-500" />
                      {fmtRelative(file.updated_at)}
                    </div>
                  </td>

                  {/* Actions */}
                  <td className="px-4 py-2.5" onClick={e => e.stopPropagation()}>
                    <DropdownMenuPrimitive.Root>
                      <DropdownMenuPrimitive.Trigger asChild>
                        <button className="h-7 w-7 rounded-md flex items-center justify-center text-gray-500 hover:bg-gray-100 dark:hover:bg-dark-700 hover:text-gray-900 dark:hover:text-white transition-colors">
                          <MoreHorizontal className="h-3.5 w-3.5" />
                        </button>
                      </DropdownMenuPrimitive.Trigger>
                      <DropdownMenuPrimitive.Portal>
                        <DropdownMenuPrimitive.Content align="end" sideOffset={4} className="z-50 w-44 rounded-xl border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-800 p-1 shadow-lg animate-in fade-in-0 zoom-in-95">
                          <DropdownMenuPrimitive.Item
                            className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm cursor-pointer outline-none hover:bg-gray-100 dark:hover:bg-dark-700 text-gray-700 dark:text-gray-200"
                            onSelect={() => onToggle(file)}
                          >
                            {file.disabled
                              ? <><Play className="h-4 w-4 text-emerald-500" /> 恢复调度</>
                              : <><Pause className="h-4 w-4 text-amber-500" /> 暂停调度</>
                            }
                          </DropdownMenuPrimitive.Item>
                          <DropdownMenuPrimitive.Item
                            disabled={file.runtime_only === true}
                            className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm cursor-pointer outline-none hover:bg-gray-100 dark:hover:bg-dark-700 text-gray-700 dark:text-gray-200 data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50"
                            onSelect={() => onEdit(file)}
                          >
                            <Pencil className="h-4 w-4 text-primary-500" /> 编辑凭证
                          </DropdownMenuPrimitive.Item>
                          <DropdownMenuPrimitive.Item
                            className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm cursor-pointer outline-none hover:bg-gray-100 dark:hover:bg-dark-700 text-gray-700 dark:text-gray-200"
                            onSelect={() => onDownload(file)}
                          >
                            <Download className={`h-4 w-4 text-indigo-500 ${downloadLoading[file.name] ? 'animate-pulse' : ''}`} /> 下载 JSON
                          </DropdownMenuPrimitive.Item>
                          <DropdownMenuPrimitive.Item
                            className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-sky-600 dark:text-sky-400 cursor-pointer outline-none hover:bg-sky-50 dark:hover:bg-sky-900/20"
                            onSelect={() => onLoadQuota(file)}
                          >
                            <Activity className={`h-4 w-4 ${quotaLoading[file.name] ? 'animate-pulse' : ''}`} /> 查询额度
                          </DropdownMenuPrimitive.Item>
                          <DropdownMenuPrimitive.Separator className="my-1 h-px bg-gray-200 dark:bg-dark-600" />
                          <DropdownMenuPrimitive.Item
                            className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-red-500 dark:text-red-400 cursor-pointer outline-none hover:bg-red-50 dark:hover:bg-red-900/20"
                            onSelect={() => onDelete(file)}
                          >
                            <Trash2 className="h-4 w-4" /> 删除凭证
                          </DropdownMenuPrimitive.Item>
                        </DropdownMenuPrimitive.Content>
                      </DropdownMenuPrimitive.Portal>
                    </DropdownMenuPrimitive.Root>
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      {/* Footer with count */}
      {files.length > 0 && (
        <div className="border-t border-gray-200 dark:border-dark-600 px-4 py-2.5 text-xs text-gray-500 dark:text-gray-400 tabular-nums">
          显示 {files.length} / {totalCount} 条凭证
        </div>
      )}
    </div>
  )
}
