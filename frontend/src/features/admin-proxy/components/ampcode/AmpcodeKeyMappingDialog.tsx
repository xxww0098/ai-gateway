import { useEffect, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/shared/components/ui/dialog'
import { Button } from '@/shared/components/ui/button'
import { Input } from '@/shared/components/ui/input'
import { Textarea } from '@/shared/components/ui/textarea'
import { Label } from '@/shared/components/ui/label'
import { Badge } from '@/shared/components/ui/badge'
import { Eye, EyeOff, KeyRound, Loader2, Plus, Sparkles, AlertCircle } from 'lucide-react'
import { parseAmpUpstreamAPIKeyForm, type AmpUpstreamAPIKeyEntry } from '../../ampcodeConfig'

interface AmpcodeKeyMappingDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialEntry?: AmpUpstreamAPIKeyEntry | null
  existingKeys: string[]
  onSave: (entry: AmpUpstreamAPIKeyEntry) => Promise<void>
  saving: boolean
}

export function AmpcodeKeyMappingDialog({
  open,
  onOpenChange,
  initialEntry,
  existingKeys,
  onSave,
  saving,
}: AmpcodeKeyMappingDialogProps) {
  const isEditing = !!initialEntry
  const [upstreamApiKey, setUpstreamApiKey] = useState('')
  const [clientKeysText, setClientKeysText] = useState('')
  const [showKey, setShowKey] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (open) {
      if (initialEntry) {
        setUpstreamApiKey(initialEntry['upstream-api-key'])
        setClientKeysText(initialEntry['api-keys'].join('\n'))
      } else {
        setUpstreamApiKey('')
        setClientKeysText('')
      }
      setShowKey(false)
      setError(null)
    }
  }, [open, initialEntry])

  const parsedEntry = parseAmpUpstreamAPIKeyForm({
    upstreamApiKey,
    apiKeysText: clientKeysText,
  })

  const isDuplicate = !isEditing && existingKeys.includes(upstreamApiKey.trim())

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)

    if (!parsedEntry['upstream-api-key']) {
      setError('目标上游 API Key 不能为空')
      return
    }

    try {
      await onSave(parsedEntry)
      onOpenChange(false)
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <form onSubmit={handleSubmit} className="space-y-4">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-base sm:text-lg">
              <KeyRound className="h-5 w-5 text-primary" />
              <span>{isEditing ? '编辑 SDK 密钥路由映射' : '新建 SDK 密钥路由映射'}</span>
            </DialogTitle>
            <DialogDescription className="text-xs">
              指定当客户端使用某些特定 API Key 请求网关时，透明换源并路由至此专属上游 Key。
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4 py-2">
            {/* Upstream Key */}
            <div className="grid gap-1.5">
              <div className="flex items-center justify-between">
                <Label className="text-xs font-semibold text-foreground/90">
                  目标上游 API Key (Upstream Key) <span className="text-destructive">*</span>
                </Label>
                {isDuplicate && (
                  <span className="text-[11px] text-amber-600 dark:text-amber-400 font-medium">
                    提示: 该上游 Key 已存在，保存将覆盖对应规则
                  </span>
                )}
              </div>
              <div className="relative flex items-center">
                <Input
                  type={showKey ? 'text' : 'password'}
                  disabled={isEditing} // keep upstream key fixed when editing
                  value={upstreamApiKey}
                  onChange={e => setUpstreamApiKey(e.target.value)}
                  placeholder="sgamp_user_xxxxxxxxxxxxxx"
                  className="font-mono text-xs pr-10 bg-background"
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="absolute right-1 h-7 w-7 text-muted-foreground hover:text-foreground"
                  onClick={() => setShowKey(!showKey)}
                >
                  {showKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                </Button>
              </div>
              {isEditing && (
                <p className="text-[11px] text-muted-foreground">如需变更上游 Key，请删除本条后重新创建。</p>
              )}
            </div>

            {/* Client Keys */}
            <div className="grid gap-1.5">
              <div className="flex items-center justify-between">
                <Label className="text-xs font-semibold text-foreground/90">
                  匹配的客户端 API Keys (Client Keys)
                </Label>
                <span className="text-[11px] text-muted-foreground">支持换行或逗号分隔</span>
              </div>
              <Textarea
                rows={4}
                value={clientKeysText}
                onChange={e => setClientKeysText(e.target.value)}
                placeholder={'client-key-alpha\nclient-key-beta, client-key-gamma'}
                className="font-mono text-xs bg-background"
              />
            </div>

            {/* Live Chips Preview */}
            <div className="rounded-lg border border-border/70 bg-muted/20 p-3 space-y-2">
              <div className="flex items-center justify-between text-xs">
                <span className="font-semibold text-foreground/80 flex items-center gap-1.5">
                  <Sparkles className="h-3.5 w-3.5 text-primary" />
                  已识别客户端 Key 预览 ({parsedEntry['api-keys'].length})
                </span>
                {parsedEntry['api-keys'].length > 0 && (
                  <span className="text-[11px] text-muted-foreground">已自动去重</span>
                )}
              </div>

              {parsedEntry['api-keys'].length === 0 ? (
                <p className="text-[11px] text-muted-foreground">暂未输入客户端 Key，保存后将暂不路由任何客户端</p>
              ) : (
                <div className="flex flex-wrap gap-1.5 max-h-28 overflow-y-auto pt-1">
                  {parsedEntry['api-keys'].map((key, i) => (
                    <Badge
                      key={i}
                      variant="secondary"
                      className="font-mono text-[11px] px-2 py-0.5 max-w-[240px] truncate"
                    >
                      {key}
                    </Badge>
                  ))}
                </div>
              )}
            </div>

            {error && (
              <div className="flex items-center gap-2 p-2.5 rounded-lg border border-destructive/30 bg-destructive/10 text-destructive text-xs">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{error}</span>
              </div>
            )}
          </div>

          <DialogFooter className="gap-2 sm:gap-0">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => onOpenChange(false)}
              disabled={saving}
              className="text-xs"
            >
              取消
            </Button>
            <Button
              type="submit"
              size="sm"
              disabled={saving || !upstreamApiKey.trim()}
              className="text-xs gap-1.5"
            >
              {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Plus className="h-3.5 w-3.5" />}
              {isEditing ? '保存修改' : '确认添加'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
