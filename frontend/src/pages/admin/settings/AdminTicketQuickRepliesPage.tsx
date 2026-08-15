import { useState, useEffect, useCallback, useRef } from "react"
import { DEFAULT_TICKET_QUICK_REPLIES } from "@/shared/constants/ticketQuickReplyDefaults"
import { errorMessage } from "@/shared/api/errors"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Textarea } from "@/shared/components/ui/textarea"
import { toast } from "sonner"
import { Plus, Trash2, Save } from "lucide-react"
import { fetchTicketQuickReplies, saveTicketQuickReplies } from "@/features/tickets/api"
import type { TicketQuickReplyItem } from "@/features/tickets/types"

export type { TicketQuickReplyItem } from "@/features/tickets/types"

type RowItem = TicketQuickReplyItem & { id: string }

const MAX_ITEMS = 24
const MAX_LABEL = 32
const MAX_TEXT = 2000

function newId(): string {
  return crypto.randomUUID()
}

function countChars(s: string): number {
  return Array.from(s).length
}

function toRows(list: TicketQuickReplyItem[]): RowItem[] {
  return list.map((r) => ({ ...r, id: newId() }))
}

function stripIds(rows: RowItem[]): TicketQuickReplyItem[] {
  return rows.map(({ label, text }) => ({ label, text }))
}

export default function AdminTicketQuickRepliesPage() {
  const [items, setItems] = useState<RowItem[]>([])
  const [draft, setDraft] = useState({ label: "", text: "" })
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const listEndRef = useRef<HTMLDivElement>(null)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const res = await fetchTicketQuickReplies()
      const list = res?.items || []
      setItems(toRows(list.length > 0 ? list : [...DEFAULT_TICKET_QUICK_REPLIES]))
    } catch (e) {
      toast.error(errorMessage(e, "加载失败"))
      setItems(toRows([...DEFAULT_TICKET_QUICK_REPLIES]))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const appendFromDraft = () => {
    const label = draft.label.trim()
    const text = draft.text.trim()
    if (!label || !text) {
      toast.error("请先在上方填写按钮标题和发送正文，再点击添加")
      return
    }
    if (countChars(label) > MAX_LABEL) {
      toast.error(`标题超过 ${MAX_LABEL} 字`)
      return
    }
    if (countChars(text) > MAX_TEXT) {
      toast.error(`正文超过 ${MAX_TEXT} 字`)
      return
    }
    if (items.length >= MAX_ITEMS) {
      toast.message(`最多 ${MAX_ITEMS} 条`)
      return
    }
    setItems((prev) => [...prev, { id: newId(), label, text }])
    setDraft({ label: "", text: "" })
    requestAnimationFrame(() => {
      listEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" })
    })
  }

  const removeRow = (id: string) => {
    setItems((prev) => prev.filter((r) => r.id !== id))
  }

  const updateRow = (id: string, patch: Partial<TicketQuickReplyItem>) => {
    setItems((prev) => prev.map((row) => (row.id === id ? { ...row, ...patch } : row)))
  }

  const handleSave = async () => {
    const clean = stripIds(items)
      .map((r) => ({ label: r.label.trim(), text: r.text.trim() }))
      .filter((r) => r.label !== "" || r.text !== "")

    if (clean.length === 0) {
      toast.error("请至少保留一条完整的快捷回复（标题与正文）")
      return
    }
    for (const r of clean) {
      if (!r.label || !r.text) {
        toast.error("每条快捷回复的标题与正文均不能为空")
        return
      }
      if (countChars(r.label) > MAX_LABEL) {
        toast.error(`标题「${r.label.slice(0, 8)}…」超过 ${MAX_LABEL} 字`)
        return
      }
      if (countChars(r.text) > MAX_TEXT) {
        toast.error("某条正文超过 2000 字")
        return
      }
    }

    setSaving(true)
    try {
      const res = await saveTicketQuickReplies(clean)
      const saved = res?.items || clean
      setItems(toRows(saved.length > 0 ? saved : [...DEFAULT_TICKET_QUICK_REPLIES]))
      toast.success("已保存")
    } catch (e) {
      toast.error(errorMessage(e, "保存失败"))
    } finally {
      setSaving(false)
    }
  }

  if (loading) {
    return <p className="text-sm text-gray-500 dark:text-dark-400">加载中…</p>
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-gray-600 dark:text-gray-300">
        配置后在「工单管理」右侧回复区显示为快捷按钮；点击即发送该条正文（无需再点发送）。在下方草稿区填写后点「添加一条」会插入列表末尾。
      </p>

      <div className="rounded-lg border border-dashed border-border bg-muted/30 p-3 dark:bg-dark-900/40">
        <p className="mb-2 text-xs font-medium text-gray-600 dark:text-gray-300">新快捷回复（草稿）</p>
        <div className="space-y-2">
          <div>
            <label className="mb-1 block text-xs text-gray-500 dark:text-dark-400">按钮标题（≤{MAX_LABEL} 字）</label>
            <Input
              value={draft.label}
              onChange={(e) => setDraft((d) => ({ ...d, label: e.target.value }))}
              placeholder="填写后点击下方「添加一条」"
            />
          </div>
          <div>
            <label className="mb-1 block text-xs text-gray-500 dark:text-dark-400">发送正文（≤{MAX_TEXT} 字）</label>
            <Textarea
              value={draft.text}
              onChange={(e) => setDraft((d) => ({ ...d, text: e.target.value }))}
              placeholder="填写完整正文…"
              rows={3}
              className="resize-y text-sm"
            />
          </div>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" variant="outline" size="sm" onClick={appendFromDraft} disabled={items.length >= MAX_ITEMS}>
          <Plus className="mr-1 h-4 w-4" />
          添加一条
        </Button>
        <Button type="button" size="sm" onClick={() => void handleSave()} disabled={saving}>
          <Save className="mr-1 h-4 w-4" />
          {saving ? "保存中…" : "保存配置"}
        </Button>
      </div>

      <ul className="space-y-4">
        {items.map((row, index) => (
          <li
            key={row.id}
            className="rounded-lg border border-border bg-gray-50/50 p-3 dark:bg-dark-900/50"
          >
            <div className="mb-2 flex items-center justify-between gap-2">
              <span className="text-xs text-gray-400 dark:text-dark-500">第 {index + 1} 条</span>
              <Button
                type="button"
                variant="dangerIcon"
                onClick={() => removeRow(row.id)}
                disabled={items.length <= 1}
                aria-label="删除"
                title="删除"
              >
                <Trash2 />
              </Button>
            </div>
            <div className="space-y-2">
              <div>
                <label className="mb-1 block text-xs text-gray-500 dark:text-dark-400">按钮标题（≤{MAX_LABEL} 字）</label>
                <Input
                  value={row.label}
                  onChange={(e) => updateRow(row.id, { label: e.target.value })}
                  placeholder="如：已收到"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs text-gray-500 dark:text-dark-400">发送正文（≤{MAX_TEXT} 字）</label>
                <Textarea
                  value={row.text}
                  onChange={(e) => updateRow(row.id, { text: e.target.value })}
                  placeholder="点击快捷按钮后，将直接发送此段文字。"
                  rows={3}
                  className="resize-y text-sm"
                />
              </div>
            </div>
          </li>
        ))}
        <div ref={listEndRef} aria-hidden />
      </ul>
    </div>
  )
}
