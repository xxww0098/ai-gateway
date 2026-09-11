import { useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card'
import { Input } from '@/shared/components/ui/input'
import { Button } from '@/shared/components/ui/button'
import { Badge } from '@/shared/components/ui/badge'
import { EmptyState } from '@/shared/components/EmptyState'
import {
  KeyRound,
  Plus,
  Trash2,
  Pencil,
  Search,
  Copy,
  Check,
  Eye,
  EyeOff,
  HelpCircle,
} from 'lucide-react'
import { copyTextToClipboard } from '@/shared/utils/clipboard'
import { toast } from 'sonner'
import { maskApiKey, type AmpUpstreamAPIKeyEntry } from '../../ampcodeConfig'

interface AmpcodeKeyRoutingCardProps {
  entries: AmpUpstreamAPIKeyEntry[]
  onOpenAddDialog: () => void
  onOpenEditDialog: (entry: AmpUpstreamAPIKeyEntry) => void
  onDeleteEntry: (upstreamKey: string) => Promise<void>
  deletingKey: string | null
}

export function AmpcodeKeyRoutingCard({
  entries,
  onOpenAddDialog,
  onOpenEditDialog,
  onDeleteEntry,
  deletingKey,
}: AmpcodeKeyRoutingCardProps) {
  const [search, setSearch] = useState('')
  const [revealedKeys, setRevealedKeys] = useState<Record<string, boolean>>({})
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  const toggleReveal = (key: string) => {
    setRevealedKeys(prev => ({ ...prev, [key]: !prev[key] }))
  }

  const handleCopy = async (key: string) => {
    await copyTextToClipboard(key)
    setCopiedKey(key)
    toast.success('上游 Key 已复制')
    setTimeout(() => setCopiedKey(null), 2000)
  }

  const filteredEntries = entries.filter(item => {
    const q = search.trim().toLowerCase()
    if (!q) return true
    if (item['upstream-api-key'].toLowerCase().includes(q)) return true
    return item['api-keys'].some(k => k.toLowerCase().includes(q))
  })

  return (
    <Card className="border-border/80 shadow-2xs">
      <CardHeader className="pb-4">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div>
            <CardTitle className="text-base sm:text-lg font-semibold flex items-center gap-2">
              <KeyRound className="h-5 w-5 text-primary" />
              <span>SDK 密钥路由映射 (Upstream API Keys)</span>
            </CardTitle>
            <CardDescription className="text-xs sm:text-sm mt-1">
              按客户端请求所携带的 SDK API Key，将请求自动换源并分流至指定 Ampcode 上游凭证。
            </CardDescription>
          </div>
          <Button
            size="sm"
            onClick={onOpenAddDialog}
            className="h-8.5 px-3.5 text-xs gap-1.5 cursor-pointer shrink-0 shadow-2xs"
          >
            <Plus className="h-3.5 w-3.5" />
            新建密钥映射
          </Button>
        </div>
      </CardHeader>

      <CardContent className="space-y-4">
        {/* Notice Info */}
        <div className="flex items-start gap-2.5 p-3 rounded-xl border border-border/70 bg-muted/20 text-xs text-muted-foreground">
          <HelpCircle className="h-4 w-4 text-primary shrink-0 mt-0.5" />
          <div className="leading-relaxed space-y-0.5">
            <span className="font-semibold text-foreground">路由工作机制：</span>
            若下游应用直接通过 Amp SDK 接入网关，网关会检查匹配规则；一旦命中绑定的 Client Key，即替换为该目标上游 Key 转发。
          </div>
        </div>

        {/* Filter bar */}
        {entries.length > 0 && (
          <div className="relative max-w-sm">
            <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
            <Input
              value={search}
              onChange={e => setSearch(e.target.value)}
              placeholder="搜索上游或客户端 Key..."
              className="pl-8 h-8 text-xs bg-background shadow-2xs"
            />
          </div>
        )}

        {/* List / Table */}
        {entries.length === 0 ? (
          <EmptyState
            icon={KeyRound}
            title="暂未配置 SDK 密钥路由"
            description="当需要根据不同的客户端 Key 路由至不同 Ampcode 上游账户或配额池时，在此添加映射规则。"
            tone="default"
            bordered
            action={{
              label: '添加首条密钥映射',
              onClick: onOpenAddDialog,
            }}
          />
        ) : filteredEntries.length === 0 ? (
          <EmptyState
            icon={Search}
            title="未找到匹配的密钥映射"
            description="尝试调整搜索关键词"
            tone="no-results"
            bordered
            action={{
              label: '清除搜索条件',
              onClick: () => setSearch(''),
            }}
          />
        ) : (
          <div className="rounded-xl border border-border/70 bg-card overflow-hidden shadow-2xs divide-y divide-border/60">
            {filteredEntries.map(entry => {
              const upstreamKey = entry['upstream-api-key']
              const isRevealed = !!revealedKeys[upstreamKey]
              const displayKey = isRevealed ? upstreamKey : maskApiKey(upstreamKey)
              const isDeleting = deletingKey === upstreamKey

              return (
                <div
                  key={upstreamKey}
                  className="p-3.5 sm:p-4 hover:bg-muted/20 transition-colors flex flex-col md:flex-row md:items-center justify-between gap-3.5"
                >
                  <div className="space-y-2 min-w-0 flex-1">
                    {/* Upstream Key Row */}
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
                        上游 Key:
                      </span>
                      <code className="font-mono text-xs font-semibold px-2 py-0.5 rounded bg-muted text-foreground select-all">
                        {displayKey}
                      </code>

                      <div className="inline-flex items-center gap-0.5">
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6 text-muted-foreground hover:text-foreground"
                          onClick={() => toggleReveal(upstreamKey)}
                          title={isRevealed ? '掩码隐藏' : '明文展示'}
                        >
                          {isRevealed ? <EyeOff className="h-3 w-3" /> : <Eye className="h-3 w-3" />}
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6 text-muted-foreground hover:text-foreground"
                          onClick={() => handleCopy(upstreamKey)}
                          title="复制上游 Key"
                        >
                          {copiedKey === upstreamKey ? (
                            <Check className="h-3 w-3 text-emerald-500" />
                          ) : (
                            <Copy className="h-3 w-3" />
                          )}
                        </Button>
                      </div>

                      <Badge variant="outline" className="text-[11px] font-normal">
                        绑定 {entry['api-keys'].length} 个 Client Key
                      </Badge>
                    </div>

                    {/* Client Keys Chips */}
                    <div className="flex flex-wrap gap-1.5 items-center pt-0.5">
                      {entry['api-keys'].length === 0 ? (
                        <span className="text-xs text-muted-foreground italic">未指定客户端 Key (全量默认)</span>
                      ) : (
                        entry['api-keys'].map((clientKey, idx) => (
                          <Badge
                            key={idx}
                            variant="secondary"
                            className="font-mono text-[11px] font-normal px-2 py-0.5 bg-muted/60 text-muted-foreground hover:text-foreground border border-border/40 max-w-[260px] truncate"
                            title={clientKey}
                          >
                            {clientKey}
                          </Badge>
                        ))
                      )}
                    </div>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-1.5 self-end md:self-center shrink-0">
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8 px-2.5 text-xs gap-1 cursor-pointer"
                      onClick={() => onOpenEditDialog(entry)}
                    >
                      <Pencil className="h-3.5 w-3.5 text-muted-foreground" />
                      编辑
                    </Button>
                    <Button
                      variant="dangerIcon"
                      size="icon"
                      className="h-8 w-8 text-destructive"
                      disabled={isDeleting}
                      onClick={() => onDeleteEntry(upstreamKey)}
                      title="删除此映射"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
