import * as DialogPrimitive from '@radix-ui/react-dialog'
import { CheckCircle2, FileText, FileUp, RotateCcw, Upload, X } from 'lucide-react'
import { useId, useState } from 'react'
import { toast } from 'sonner'
import { dedupeAuthFiles } from '@/features/admin-proxy/authFileImportUtils'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/components/ui/tabs'
import { cn } from '@/shared/utils/utils'
import type { AuthFileUploadDialogProps } from '../types'

const ACCEPT = '.json,application/json'

function fileAllowed(file: File): boolean {
  // 仅接受扩展名 .json（系统报告的 MIME 常为 octet-stream / 空）
  return /\.json$/i.test(file.name)
}

export function AuthFileUploadDialog({
  open,
  onOpenChange,
  onUpload,
  uploading,
  value,
  onValueChange,
  pickedFiles,
  onPickedFilesChange,
}: AuthFileUploadDialogProps) {
  const fileInputId = useId()
  const [uploadMode, setUploadMode] = useState<'files' | 'paste'>('files')
  const [dragActive, setDragActive] = useState(false)
  const [readingFiles, setReadingFiles] = useState(false)
  const [lastPickSummary, setLastPickSummary] = useState<{ total: number; unique: number; removed: number } | null>(
    null
  )

  const handleFileList = async (list: FileList | File[] | null) => {
    const raw = list ? Array.from(list) : []
    const allowed = raw.filter(fileAllowed)
    if (raw.length > 0 && allowed.length === 0) {
      toast.error('仅支持 .json 文件')
      return
    }
    if (allowed.length === 0) return

    setReadingFiles(true)
    try {
      const { files, removedDuplicates } = await dedupeAuthFiles(allowed)
      onPickedFilesChange(files)
      onValueChange('')
      setUploadMode('files')
      setLastPickSummary({
        total: allowed.length,
        unique: files.length,
        removed: removedDuplicates,
      })
      if (removedDuplicates > 0) {
        toast.message(`已去除 ${removedDuplicates} 个重复文件（内容相同）`)
      }
    } finally {
      setReadingFiles(false)
    }
  }

  const handleModeChange = (mode: string) => {
    const nextMode = mode === 'paste' ? 'paste' : 'files'
    setUploadMode(nextMode)
    setDragActive(false)
    setLastPickSummary(null)
    if (nextMode === 'files') {
      onValueChange('')
    } else {
      onPickedFilesChange([])
    }
  }

  const clearFiles = () => {
    onPickedFilesChange([])
    setLastPickSummary(null)
  }

  const canUpload = uploadMode === 'files' ? pickedFiles.length > 0 : value.trim().length > 0
  const uploadLabel = uploading
    ? '导入中...'
    : uploadMode === 'files'
      ? pickedFiles.length > 0 ? `导入 ${pickedFiles.length} 个文件` : '导入文件'
      : '导入粘贴内容'
  const pickSummaryText = lastPickSummary && lastPickSummary.removed > 0
    ? `共 ${lastPickSummary.total} 个，已去重 ${lastPickSummary.removed} 个`
    : '相同内容会自动去重'
  const previewFiles = pickedFiles.slice(0, 3)

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-xl translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white flex items-center gap-2">
            <Upload className="h-5 w-5 text-primary-500" /> 导入凭证
          </DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            粘贴 JSON 格式的 Auth File 内容，或一次选择 / 拖放多个 JSON 文件；相同内容的文件会自动去重。
          </DialogPrimitive.Description>

          <div className="py-5">
            <Tabs value={uploadMode} onValueChange={handleModeChange}>
              <TabsList className="grid h-10 w-full grid-cols-2 rounded-xl bg-gray-100/80 p-1 dark:bg-dark-800/80">
                <TabsTrigger value="files" className="rounded-lg text-xs data-[state=active]:bg-white dark:data-[state=active]:bg-dark-700">
                  上传 JSON 文件
                </TabsTrigger>
                <TabsTrigger value="paste" className="rounded-lg text-xs data-[state=active]:bg-white dark:data-[state=active]:bg-dark-700">
                  粘贴 JSON 内容
                </TabsTrigger>
              </TabsList>

              <TabsContent value="files" className="mt-4 focus-visible:outline-none focus-visible:ring-0">
                <div
                  onDragEnter={e => {
                    e.preventDefault()
                    e.stopPropagation()
                    setDragActive(true)
                  }}
                  onDragOver={e => {
                    e.preventDefault()
                    e.stopPropagation()
                    setDragActive(true)
                  }}
                  onDragLeave={e => {
                    e.preventDefault()
                    e.stopPropagation()
                    if (e.currentTarget.contains(e.relatedTarget as Node)) return
                    setDragActive(false)
                  }}
                  onDrop={e => {
                    e.preventDefault()
                    e.stopPropagation()
                    setDragActive(false)
                    void handleFileList(e.dataTransfer.files)
                  }}
                  className={cn(
                    'rounded-xl border-2 border-dashed px-4 py-6 text-center transition-colors',
                    readingFiles && 'pointer-events-none opacity-60',
                    dragActive
                      ? 'border-primary-400 bg-primary-50/80 dark:bg-primary-950/30'
                      : pickedFiles.length > 0
                        ? 'border-primary-200 bg-primary-50/60 dark:border-primary-900/60 dark:bg-primary-950/20'
                        : 'border-gray-200 bg-gray-50/50 dark:border-dark-600 dark:bg-dark-800/50'
                  )}
                >
                  <input
                    id={fileInputId}
                    type="file"
                    accept={ACCEPT}
                    multiple
                    className="sr-only"
                    tabIndex={-1}
                    disabled={readingFiles}
                    onChange={e => {
                      void handleFileList(e.target.files)
                      e.target.value = ''
                    }}
                  />

                  {pickedFiles.length > 0 ? (
                    <div className="mx-auto flex max-w-md flex-col items-center gap-3">
                      <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary-100 text-primary-600 dark:bg-primary-950/50 dark:text-primary-300">
                        <CheckCircle2 className="h-5 w-5" aria-hidden />
                      </div>
                      <div>
                        <p className="text-sm font-semibold text-gray-900 dark:text-white">
                          已选择 {pickedFiles.length} 个 JSON 文件
                        </p>
                        <p className="mt-1 text-xs text-gray-500 dark:text-dark-400">
                          {readingFiles ? '读取中...' : pickSummaryText}，导入后会自动分配到对应 Provider
                        </p>
                      </div>
                      <div className="flex max-w-full flex-wrap justify-center gap-1.5">
                        {previewFiles.map(file => (
                          <span
                            key={`${file.name}-${file.size}`}
                            className="inline-flex max-w-[150px] items-center gap-1 truncate rounded-lg bg-white px-2 py-1 text-[11px] text-gray-500 shadow-sm ring-1 ring-gray-200 dark:bg-dark-900 dark:text-dark-300 dark:ring-dark-700"
                            title={file.name}
                          >
                            <FileText className="h-3 w-3 flex-shrink-0" />
                            <span className="truncate">{file.name}</span>
                          </span>
                        ))}
                        {pickedFiles.length > previewFiles.length && (
                          <span className="inline-flex items-center rounded-lg bg-white px-2 py-1 text-[11px] text-gray-500 shadow-sm ring-1 ring-gray-200 dark:bg-dark-900 dark:text-dark-300 dark:ring-dark-700">
                            +{pickedFiles.length - previewFiles.length}
                          </span>
                        )}
                      </div>
                      <div className="flex items-center gap-2">
                        <label
                          htmlFor={fileInputId}
                          className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-lg border border-primary-200 bg-white px-3 text-xs font-medium text-primary-700 shadow-sm transition-colors hover:bg-primary-50 dark:border-primary-900/60 dark:bg-dark-900 dark:text-primary-300 dark:hover:bg-primary-950/30"
                        >
                          <RotateCcw className="h-3.5 w-3.5" />
                          重新选择
                        </label>
                        <button
                          type="button"
                          onClick={clearFiles}
                          className="h-8 rounded-lg px-3 text-xs font-medium text-gray-500 transition-colors hover:bg-gray-100 hover:text-gray-700 dark:text-dark-300 dark:hover:bg-dark-700 dark:hover:text-white"
                        >
                          清空
                        </button>
                      </div>
                    </div>
                  ) : (
                    <div className="mx-auto flex max-w-md flex-col items-center gap-3">
                      <FileUp className="h-9 w-9 text-primary-500 opacity-80" aria-hidden />
                      <div>
                        <p className="text-sm font-semibold text-gray-900 dark:text-white">拖入 JSON 文件，或点击选择</p>
                        <p className="mt-1 text-xs text-gray-500 dark:text-dark-400">支持多选，重复内容会自动去重</p>
                      </div>
                      <label
                        htmlFor={fileInputId}
                        className="inline-flex h-9 cursor-pointer items-center rounded-lg bg-primary-600 px-4 text-sm font-medium text-white shadow-sm shadow-primary-500/20 transition-colors hover:bg-primary-700"
                      >
                        选择文件
                      </label>
                      <p className="text-[11px] text-gray-400 dark:text-dark-500">
                        仅支持 .json（UTF-8）
                      </p>
                    </div>
                  )}
                </div>
              </TabsContent>

              <TabsContent value="paste" className="mt-4 focus-visible:outline-none focus-visible:ring-0">
                <textarea
                  className="input min-h-[220px] resize-y font-mono text-xs"
                  placeholder="粘贴 JSON 凭证内容..."
                  value={value}
                  onChange={e => {
                    onValueChange(e.target.value)
                    onPickedFilesChange([])
                    setLastPickSummary(null)
                  }}
                />
              </TabsContent>
            </Tabs>

            <div className="mt-4 rounded-xl border border-dashed border-gray-200 dark:border-dark-600 bg-gray-50/80 dark:bg-dark-800/70 px-3 py-3 text-xs leading-5 text-gray-500 dark:text-dark-400">
              支持 CPA JSON 和标准 Auth File。导入后会自动识别 Provider，相同内容只保留一份。
            </div>
          </div>

          <div className="flex justify-end gap-3">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-ghost px-4 text-sm">取消</button>
            </DialogPrimitive.Close>
            <button
              className="btn btn-primary px-5 text-sm"
              onClick={onUpload}
              disabled={uploading || readingFiles || !canUpload}
            >
              {uploadLabel}
            </button>
          </div>
          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-md p-1 opacity-70 hover:opacity-100 transition-opacity">
            <X className="h-4 w-4" />
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
