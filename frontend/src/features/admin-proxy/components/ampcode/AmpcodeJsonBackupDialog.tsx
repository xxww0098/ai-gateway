import { useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/shared/components/ui/dialog'
import { Button } from '@/shared/components/ui/button'
import { Textarea } from '@/shared/components/ui/textarea'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/shared/components/ui/tabs'
import { Code2, Copy, Check, Download, Upload, AlertCircle, Loader2 } from 'lucide-react'
import { copyTextToClipboard } from '@/shared/utils/clipboard'
import { toast } from 'sonner'
import { updateProviderConfig } from '../../api'

interface AmpcodeJsonBackupDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  rawConfig: Record<string, unknown>
  onConfigImported: () => Promise<void>
}

export function AmpcodeJsonBackupDialog({
  open,
  onOpenChange,
  rawConfig,
  onConfigImported,
}: AmpcodeJsonBackupDialogProps) {
  const [copied, setCopied] = useState(false)
  const [importJson, setImportJson] = useState('')
  const [importing, setImporting] = useState(false)
  const [importError, setImportError] = useState<string | null>(null)

  const formattedJson = JSON.stringify(rawConfig, null, 2)

  const handleCopy = async () => {
    await copyTextToClipboard(formattedJson)
    setCopied(true)
    toast.success('配置 JSON 已复制到剪贴板')
    setTimeout(() => setCopied(false), 2000)
  }

  const handleDownload = () => {
    try {
      const blob = new Blob([formattedJson], { type: 'application/json' })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `ampcode-config-${new Date().toISOString().slice(0, 10)}.json`
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
      toast.success('配置已导出')
    } catch {
      toast.error('导出文件失败')
    }
  }

  const handleImport = async () => {
    setImportError(null)
    if (!importJson.trim()) {
      setImportError('请输入或粘贴 JSON 配置内容')
      return
    }

    let parsed: unknown
    try {
      parsed = JSON.parse(importJson)
    } catch (e: unknown) {
      setImportError(`JSON 语法解析错误: ${e instanceof Error ? e.message : String(e)}`)
      return
    }

    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      setImportError('配置必须为顶层 JSON 对象')
      return
    }

    setImporting(true)
    try {
      await updateProviderConfig('/ampcode', parsed as Record<string, unknown>)
      toast.success('配置导入并保存成功')
      await onConfigImported()
      onOpenChange(false)
      setImportJson('')
    } catch (e: unknown) {
      setImportError(`导入失败: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setImporting(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl max-h-[90vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-base sm:text-lg">
            <Code2 className="h-5 w-5 text-primary" />
            <span>Ampcode 原始配置 (JSON)</span>
          </DialogTitle>
          <DialogDescription className="text-xs">
            直接查看、备份或导入底层数据库中存储的 <code>ampcode_configs</code> 配置块。
          </DialogDescription>
        </DialogHeader>

        <Tabs defaultValue="view" className="flex-1 flex flex-col overflow-hidden py-1">
          <TabsList className="grid w-full grid-cols-2 mb-3">
            <TabsTrigger value="view" className="text-xs">查看与备份导出</TabsTrigger>
            <TabsTrigger value="import" className="text-xs">导入新配置</TabsTrigger>
          </TabsList>

          {/* View Tab */}
          <TabsContent value="view" className="space-y-3 flex-1 overflow-hidden flex flex-col mt-0">
            <div className="relative flex-1 min-h-[300px] overflow-hidden rounded-xl border border-border/70 bg-muted/40">
              <pre className="h-full max-h-[420px] overflow-auto p-4 text-xs font-mono text-foreground select-all leading-relaxed">
                {formattedJson}
              </pre>
            </div>

            <div className="flex items-center justify-end gap-2 pt-1">
              <Button
                variant="outline"
                size="sm"
                className="h-8.5 text-xs gap-1.5 cursor-pointer shadow-2xs"
                onClick={handleCopy}
              >
                {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                复制 JSON
              </Button>
              <Button
                variant="outline"
                size="sm"
                className="h-8.5 text-xs gap-1.5 cursor-pointer shadow-2xs"
                onClick={handleDownload}
              >
                <Download className="h-3.5 w-3.5" />
                下载为 .json 文件
              </Button>
            </div>
          </TabsContent>

          {/* Import Tab */}
          <TabsContent value="import" className="space-y-3 flex-1 flex flex-col mt-0">
            <div className="text-xs text-muted-foreground">
              粘贴 JSON 对象，将全量合并或更新 Ampcode 上游 URL、API Key、模型映射和 SDK Key 列表。
            </div>

            <Textarea
              value={importJson}
              onChange={e => setImportJson(e.target.value)}
              placeholder='{\n  "upstream-url": "https://ampcode.com",\n  "force-model-mappings": false,\n  "model-mappings": []\n}'
              className="flex-1 min-h-[260px] font-mono text-xs bg-background leading-relaxed"
            />

            {importError && (
              <div className="flex items-center gap-2 p-2.5 rounded-lg border border-destructive/30 bg-destructive/10 text-destructive text-xs">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{importError}</span>
              </div>
            )}

            <div className="flex items-center justify-end gap-2 pt-1">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => onOpenChange(false)}
                disabled={importing}
                className="text-xs"
              >
                取消
              </Button>
              <Button
                type="button"
                size="sm"
                onClick={handleImport}
                disabled={importing || !importJson.trim()}
                className="text-xs gap-1.5"
              >
                {importing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Upload className="h-3.5 w-3.5" />}
                确认导入并覆盖配置
              </Button>
            </div>
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  )
}
