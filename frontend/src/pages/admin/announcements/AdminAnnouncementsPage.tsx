import { useState, useEffect } from "react"
import { fetchApi } from "@/shared/api/client"
import { Card } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/shared/components/ui/table"
import { Badge } from "@/shared/components/ui/badge"
import { toast } from "sonner"
import { Input } from "@/shared/components/ui/input"
import { Textarea } from "@/shared/components/ui/textarea"
import { Trash2, Plus, Edit, Save, X } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/shared/components/ui/dialog"

interface Announcement {
  id: number
  title: string
  content: string
  type: string
  is_active: boolean
  created_at: string
}

export default function Announcements() {
  const [items, setItems] = useState<Announcement[]>([])
  const [loading, setLoading] = useState(true)

  // Edit State
  const [editingId, setEditingId] = useState<number | null>(null)
  const [editForm, setEditForm] = useState<{ title: string; content: string; type: string; is_active: boolean | string }>({
    title: '', content: '', type: 'info', is_active: true
  })

  // Create State
  const [open, setOpen] = useState(false)
  const [createForm, setCreateForm] = useState({
    title: "",
    content: "",
    type: "info",
    is_active: "true"
  })
  const [creating, setCreating] = useState(false)

  useEffect(() => {
    loadItems()
  }, [])

  const loadItems = async (silent = false) => {
    if (!silent) setLoading(true)
    try {
      const res = await fetchApi(`/admin/announcements`)
      setItems(res.data.items || [])
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "加载失败")
    } finally {
      setLoading(false)
    }
  }

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!createForm.title || !createForm.content) return
    setCreating(true)
    try {
      await fetchApi("/admin/announcements", {
        method: "POST",
        body: JSON.stringify({
          title: createForm.title,
          content: createForm.content,
          type: createForm.type,
          is_active: createForm.is_active === "true",
        }),
      })
      toast.success("公告发布成功")
      setOpen(false)
      loadItems(true)
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "发布失败")
    } finally {
      setCreating(false)
    }
  }

  const handleEdit = (r: Announcement) => {
    setEditingId(r.id)
    setEditForm({
      title: r.title,
      content: r.content,
      type: r.type,
      is_active: r.is_active
    })
  }

  const handleSave = async (id: number) => {
    try {
      await fetchApi(`/admin/announcements/${id}`, {
        method: "PUT",
        body: JSON.stringify({
          title: editForm.title,
          content: editForm.content,
          type: editForm.type,
          is_active: String(editForm.is_active) === "true",
        }),
      })
      toast.success("已更新")
      setEditingId(null)
      loadItems(true)
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "保存失败")
    }
  }

  const handleDelete = async (id: number) => {
    if (!confirm("确定要删除这条公告吗？")) return
    try {
      await fetchApi(`/admin/announcements/${id}`, { method: "DELETE" })
      toast.success("删除成功")
      loadItems(true)
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "删除失败")
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex justify-end">
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogTrigger asChild>
            <Button className="gap-2">
              <Plus className="h-4 w-4" />
              发布公告
            </Button>
          </DialogTrigger>
          <DialogContent className="max-w-xl">
            <DialogHeader>
              <DialogTitle>分布新的公告</DialogTitle>
              <DialogDescription>
                公告支持基础 Markdown 展示，会在用户控制台首页置顶显示。
              </DialogDescription>
            </DialogHeader>
            <form onSubmit={handleCreate} className="space-y-4 pt-4">
              <div className="space-y-2">
                <label className="text-sm font-medium">标题</label>
                <Input
                  placeholder="如：国庆期间 API 调用限流说明"
                  value={createForm.title}
                  onChange={(e) => setCreateForm({...createForm, title: e.target.value})}
                  required
                />
              </div>
              <div className="space-y-2">
                <label className="text-sm font-medium">内容正文</label>
                <Textarea
                  placeholder="可以在这里输入详情..."
                  value={createForm.content}
                  onChange={(e) => setCreateForm({...createForm, content: e.target.value})}
                  rows={4}
                  required
                />
              </div>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <label className="text-sm font-medium">样式类型</label>
                  <select 
                    className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                    value={createForm.type}
                    onChange={(e) => setCreateForm({...createForm, type: e.target.value})}
                  >
                    <option value="info">默认 (信息)</option>
                    <option value="warning">警告 (Warning)</option>
                    <option value="danger">危险 (Danger)</option>
                  </select>
                </div>
                <div className="space-y-2">
                  <label className="text-sm font-medium">立即发布</label>
                  <select 
                    className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background disabled:cursor-not-allowed disabled:opacity-50"
                    value={createForm.is_active}
                    onChange={(e) => setCreateForm({...createForm, is_active: e.target.value})}
                  >
                    <option value="true">是，即刻展示</option>
                    <option value="false">否，存为草稿</option>
                  </select>
                </div>
              </div>
              <div className="flex justify-end gap-2">
                <Button type="button" variant="outline" onClick={() => setOpen(false)}>取消</Button>
                <Button type="submit" disabled={creating || !createForm.title}>
                  {creating ? "保存中..." : "保存发布"}
                </Button>
              </div>
            </form>
          </DialogContent>
        </Dialog>
      </div>

      <Card className="shadow-sm border-border overflow-hidden">
        <Table>
          <TableHeader className="bg-secondary/50">
            <TableRow>
              <TableHead>标题</TableHead>
              <TableHead>类型</TableHead>
              <TableHead>状态</TableHead>
              <TableHead>发布时间</TableHead>
              <TableHead className="text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={5} className="h-32 text-center text-muted-foreground">加载中...</TableCell>
              </TableRow>
            ) : items.length === 0 ? (
              <TableRow>
                <TableCell colSpan={5} className="h-32 text-center text-muted-foreground">没有公告数据</TableCell>
              </TableRow>
            ) : (
              items.map((r) => {
                const isEditing = editingId === r.id
                return (
                  <TableRow key={r.id}>
                    <TableCell className="font-medium">
                      {isEditing ? (
                        <Input value={editForm.title} onChange={(e) => setEditForm({...editForm, title: e.target.value})} className="h-8" />
                      ) : (
                        r.title
                      )}
                    </TableCell>
                    <TableCell>
                      {isEditing ? (
                        <select value={editForm.type} onChange={(e) => setEditForm({...editForm, type: e.target.value})} className="h-8 w-24 border rounded">
                          <option value="info">info</option>
                          <option value="warning">warning</option>
                          <option value="danger">danger</option>
                        </select>
                      ) : (
                        <Badge variant="outline" className={
                          r.type === 'danger' ? 'bg-destructive/10 text-destructive border-transparent' : 
                          r.type === 'warning' ? 'bg-amber-500/10 text-amber-600 border-transparent' : 
                          'bg-primary/10 text-primary border-transparent'
                        }>{r.type}</Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      {isEditing ? (
                         <select value={String(editForm.is_active)} onChange={(e) => setEditForm({...editForm, is_active: e.target.value})} className="h-8 w-20 border rounded">
                         <option value="true">生效</option>
                         <option value="false">隐藏</option>
                       </select>
                      ) : (
                        <Badge variant={r.is_active ? 'default' : 'secondary'} className={r.is_active ? 'bg-primary text-primary-foreground hover:bg-primary' : ''}>
                          {r.is_active ? '生效中' : '已隐藏'}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-muted-foreground text-sm">
                      {new Date(r.created_at).toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right">
                      {isEditing ? (
                        <div className="flex justify-end gap-1">
                          <Button size="icon" variant="ghost" onClick={() => handleSave(r.id)} className="text-primary hover:text-primary hover:bg-primary/10"><Save className="h-4 w-4" /></Button>
                          <Button size="icon" variant="ghost" onClick={() => setEditingId(null)}><X className="h-4 w-4" /></Button>
                        </div>
                      ) : (
                        <div className="flex justify-end gap-1">
                          <Button size="icon" variant="ghost" onClick={() => handleEdit(r)} className="text-muted-foreground"><Edit className="h-4 w-4" /></Button>
                          <Button type="button" variant="dangerIcon" onClick={() => handleDelete(r.id)} title="删除" aria-label="删除"><Trash2 /></Button>
                        </div>
                      )}
                    </TableCell>
                  </TableRow>
                )
              })
            )}
          </TableBody>
        </Table>
      </Card>
    </div>
  )
}
