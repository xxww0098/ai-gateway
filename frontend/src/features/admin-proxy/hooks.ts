import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/shared/api/query-keys'
import { errorMessage } from '@/shared/api/errors'
import { toast } from 'sonner'
import {
  fetchAuthFiles,
  toggleAuthFileStatus,
  deleteAuthFile,
  uploadAuthFiles,
  updateAuthFile,
  downloadAuthFile,
  exportAuthFiles,
  saveBlobToFile,
} from './api'
import type { AuthFileItem } from './types'

// ── Auth Files Hooks ────────────────────────────────────────────────────────

/**
 * Fetches and returns the list of auth files from the SDK management API.
 * Normalizes the response to always return an array.
 */
export function useAuthFiles() {
  return useQuery({
    queryKey: queryKeys.proxy.authFiles(),
    queryFn: async () => {
      const res = await fetchAuthFiles()
      const parsed = res?.files || res?.['auth-files'] || res?.authFiles || []
      return Array.isArray(parsed) ? parsed : []
    },
  })
}

/**
 * Mutation to toggle an auth file's disabled status.
 * Invalidates the auth files query on success.
 */
export function useToggleAuthFile() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ name, disabled }: { name: string; disabled: boolean }) =>
      toggleAuthFileStatus(name, disabled),
    onSuccess: (_data, variables) => {
      toast.success(variables.disabled ? '已暂停调度' : '已恢复调度')
      qc.invalidateQueries({ queryKey: queryKeys.proxy.authFiles() })
    },
    onError: (err) => toast.error(errorMessage(err, '操作失败')),
  })
}

/**
 * Mutation to delete an auth file by name.
 * Invalidates the auth files query on success.
 */
export function useDeleteAuthFile() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (name: string) => deleteAuthFile(name),
    onSuccess: () => {
      toast.success('凭证已删除')
      qc.invalidateQueries({ queryKey: queryKeys.proxy.authFiles() })
    },
    onError: (err) => toast.error(errorMessage(err, '删除失败')),
  })
}

/**
 * Mutation to upload auth files (multipart/form-data).
 * Invalidates the auth files query on success.
 */
export function useUploadAuthFile() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (formData: FormData) => uploadAuthFiles(formData),
    onSuccess: (res) => {
      if (res?.status === 'partial' && Array.isArray(res.failed) && res.failed.length) {
        const uploaded = typeof res.uploaded === 'number' ? res.uploaded : 0
        toast.warning(`部分成功：已上传 ${uploaded} 个，${res.failed.length} 个失败`)
      } else if (typeof res?.uploaded === 'number' && res.uploaded > 1) {
        toast.success(`凭证导入成功（${res.uploaded} 个文件）`)
      } else {
        toast.success('凭证导入成功')
      }
      qc.invalidateQueries({ queryKey: queryKeys.proxy.authFiles() })
    },
    onError: (err) => toast.error(errorMessage(err, '导入失败')),
  })
}

/**
 * Mutation to edit an auth record's fields. Empty strings clear; masked
 * previews are ignored server-side so a leaked-mask round-trip is safe.
 */
export function useUpdateAuthFile() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, fields }: { id: string; fields: Record<string, unknown> }) =>
      updateAuthFile(id, fields),
    onSuccess: () => {
      toast.success('凭证已更新')
      qc.invalidateQueries({ queryKey: queryKeys.proxy.authFiles() })
    },
    onError: (err) => toast.error(errorMessage(err, '更新失败')),
  })
}

/**
 * Mutation to download a single auth record as JSON.
 * Triggers a browser save dialog on success.
 */
export function useDownloadAuthFile() {
  return useMutation({
    mutationFn: async (target: { id?: string; name?: string }) => {
      const { blob, filename } = await downloadAuthFile(target)
      saveBlobToFile(blob, filename)
      return filename
    },
    onSuccess: (filename) => toast.success(`已下载 ${filename}`),
    onError: (err) => toast.error(errorMessage(err, '下载失败')),
  })
}

/**
 * Mutation to download a zip archive of multiple auth records.
 */
export function useExportAuthFiles() {
  return useMutation({
    mutationFn: async (ids: string[]) => {
      if (ids.length === 0) throw new Error('未选择凭证')
      const { blob, filename } = await exportAuthFiles(ids)
      saveBlobToFile(blob, filename)
      return { filename, count: ids.length }
    },
    onSuccess: ({ filename, count }) => toast.success(`已导出 ${count} 个凭证：${filename}`),
    onError: (err) => toast.error(errorMessage(err, '导出失败')),
  })
}

// ── Batch Operations ────────────────────────────────────────────────────────

/**
 * Mutation to batch update auth file statuses (pause/resume multiple).
 * Invalidates the auth files query on success.
 */
export function useBatchToggleAuthFiles() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ files, disabled }: { files: AuthFileItem[]; disabled: boolean }) => {
      let successCount = 0
      let failCount = 0
      for (const file of files) {
        try {
          await toggleAuthFileStatus(file.name, disabled)
          successCount++
        } catch (e) {
          console.error(`Failed to update ${file.name}`, e)
          failCount++
        }
      }
      return { successCount, failCount, disabled }
    },
    onSuccess: ({ successCount, failCount, disabled }) => {
      if (failCount === 0) {
        toast.success(disabled ? `已暂停 ${successCount} 个凭证` : `已恢复 ${successCount} 个凭证`)
      } else {
        toast.warning(`${disabled ? '暂停' : '恢复'}完成: ${successCount} 成功, ${failCount} 失败`)
      }
      qc.invalidateQueries({ queryKey: queryKeys.proxy.authFiles() })
    },
    onError: (err) => toast.error(errorMessage(err, '批量操作失败')),
  })
}

/**
 * Mutation to batch delete auth files.
 * Invalidates the auth files query on success.
 */
export function useBatchDeleteAuthFiles() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (names: string[]) => {
      let successCount = 0
      let failCount = 0
      for (const name of names) {
        try {
          await deleteAuthFile(name)
          successCount++
        } catch (e) {
          console.error(`Failed to delete ${name}`, e)
          failCount++
        }
      }
      return { successCount, failCount }
    },
    onSuccess: ({ successCount, failCount }) => {
      if (failCount === 0) {
        toast.success(`成功删除 ${successCount} 个凭证`)
      } else {
        toast.warning(`删除完成: ${successCount} 成功, ${failCount} 失败`)
      }
      qc.invalidateQueries({ queryKey: queryKeys.proxy.authFiles() })
    },
    onError: (err) => toast.error(errorMessage(err, '批量删除失败')),
  })
}
