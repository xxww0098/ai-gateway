import { useCallback, useEffect, useState } from "react"
import { fetchApi } from "@/shared/api/client"
import { Card } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/shared/components/ui/table"
import { Badge } from "@/shared/components/ui/badge"
import { toast } from "sonner"
import { Input } from "@/shared/components/ui/input"
import { Trash2, Plus, Copy, Check, Gift } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/shared/components/ui/dialog"
import { confirmModal } from "@/shared/confirm-modal"

interface RedeemCode {
  id: number
  code: string
  amount: number
  status: string
  created_at: string
  used_at?: string
  used_by?: string
}

export default function RedeemCodes() {
  const [codes, setCodes] = useState<RedeemCode[]>([])
  const [loading, setLoading] = useState(true)
  const [copiedId, setCopiedId] = useState<number | null>(null)

  // Create State
  const [open, setOpen] = useState(false)
  const [createForm, setCreateForm] = useState({
    count: "1",
    amount: "10.00",
  })
  const [creating, setCreating] = useState(false)

  const loadCodes = useCallback(async (silent = false) => {
    if (!silent) setLoading(true)
    try {
      const res = await fetchApi(`/admin/redeem-codes`)
      setCodes(res.data.items || [])
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "加载失败")
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    const timer = globalThis.setTimeout(() => { void loadCodes() }, 0)
    return () => globalThis.clearTimeout(timer)
  }, [loadCodes])

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!createForm.count || !createForm.amount) return
    setCreating(true)
    try {
      const res = await fetchApi("/admin/redeem-codes", {
        method: "POST",
        body: JSON.stringify({
          count: parseInt(createForm.count, 10),
          amount: parseFloat(createForm.amount),
        }),
      })
      toast.success(`成功生成 ${res.data.created} 个兑换码`)
      setOpen(false)
      void loadCodes(true)
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "生成失败")
    } finally {
      setCreating(false)
    }
  }

  const handleDelete = async (id: number) => {
    const ok = await confirmModal({ message: "确定要删除此兑换码吗？" })
    if (!ok) return
    try {
      await fetchApi(`/admin/redeem-codes/${id}`, { method: "DELETE" })
      toast.success("删除成功")
      void loadCodes(true)
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "删除失败")
    }
  }

  const handleCopy = (id: number, codeStr: string) => {
    navigator.clipboard.writeText(codeStr)
    setCopiedId(id)
    setTimeout(() => setCopiedId(null), 2000)
    toast.success("兑换码已复制")
  }

  return (
    <div className="space-y-6 animate-in fade-in duration-500" style={{ willChange: 'transform, opacity' }}>
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div className="space-y-1">
          <h3 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
            <Gift className="w-5 h-5 text-purple-500" />
            充值兑换卡密
          </h3>
          <p className="text-sm text-gray-500 max-w-2xl">
            生成含有预设余额的兑换码（如类似于礼品卡）。您可以将这些卡密发放给用户，用户在充值页面使用后，对应的金额将自动增加到他们的账户余额中。
          </p>
        </div>
        
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogTrigger asChild>
            <Button className="gap-2 bg-purple-600 hover:bg-purple-700 text-white shadow-sm">
              <Plus className="h-4 w-4" />
              批量生成
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>生成兑换卡密</DialogTitle>
              <DialogDescription>
                指定生成的数量和每个卡密包含的余额（USD）。生成的卡密可以线下分发给用户。
              </DialogDescription>
            </DialogHeader>
            <form onSubmit={handleCreate} className="space-y-4 pt-4">
              <div className="space-y-2">
                <label className="text-sm font-medium">生成数量 (最多 100 个)</label>
                <Input
                  type="number" min="1" max="100"
                  value={createForm.count}
                  onChange={(e) => setCreateForm({...createForm, count: e.target.value})}
                  required
                />
              </div>
              <div className="space-y-2">
                <label className="text-sm font-medium">每个卡密的面值 ($)</label>
                <Input
                  type="number" step="0.01" min="0.01"
                  value={createForm.amount}
                  onChange={(e) => setCreateForm({...createForm, amount: e.target.value})}
                  required
                />
              </div>
              <div className="flex justify-end gap-2 pt-4">
                <Button type="button" variant="outline" onClick={() => setOpen(false)}>取消</Button>
                <Button type="submit" disabled={creating} className="bg-purple-600 hover:bg-purple-700">
                  {creating ? "生成中..." : "立即生成"}
                </Button>
              </div>
            </form>
          </DialogContent>
        </Dialog>
      </div>

      <Card className="shadow-sm border-border overflow-hidden">
        <Table>
          <TableHeader className="bg-gray-50/50 dark:bg-dark-900/50">
            <TableRow>
              <TableHead>卡密代码 (点击可复制)</TableHead>
              <TableHead>面值</TableHead>
              <TableHead>状态</TableHead>
              <TableHead>使用者</TableHead>
              <TableHead>使用时间</TableHead>
              <TableHead className="text-right">生成时间</TableHead>
              <TableHead className="w-[80px]"></TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={7} className="h-32 text-center text-muted-foreground">正在加载卡密记录...</TableCell>
              </TableRow>
            ) : codes.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} className="h-32 text-center text-muted-foreground">
                  <div className="flex h-full min-h-32 w-full flex-col items-center justify-center gap-2">
                    <Gift className="w-8 h-8 text-gray-300 dark:text-gray-700" />
                    <p>您还没有生成任何兑换码。</p>
                  </div>
                </TableCell>
              </TableRow>
            ) : (
              codes.map((c) => (
                <TableRow key={c.id}>
                  <TableCell>
                    <div 
                      className="font-mono text-sm bg-gray-100 dark:bg-dark-800 hover:bg-gray-200 dark:hover:bg-dark-700 text-gray-900 dark:text-gray-100 p-1.5 px-2.5 rounded cursor-pointer inline-flex items-center gap-2 group transition-colors shadow-sm"
                      onClick={() => handleCopy(c.id, c.code)}
                    >
                      {c.code}
                      {copiedId === c.id ? (
                        <Check className="h-3.5 w-3.5 text-emerald-500" />
                      ) : (
                        <Copy className="h-3.5 w-3.5 opacity-0 group-hover:opacity-100 transition-opacity text-gray-400" />
                      )}
                    </div>
                  </TableCell>
                  <TableCell className="font-medium text-purple-600 dark:text-purple-400">${c.amount.toFixed(4)}</TableCell>
                  <TableCell>
                     <Badge variant={c.status === 'active' ? 'default' : (c.status === 'exhausted' ? 'secondary' : 'destructive')} 
                            className={c.status === 'active' ? 'bg-emerald-500 hover:bg-emerald-600 text-white border-transparent' : 'font-normal'}>
                        {c.status === 'active' ? '未兑换 (Active)' : (c.status === 'exhausted' ? '已使用 (Redeemed)' : '已禁用')}
                     </Badge>
                  </TableCell>
                  <TableCell className="text-gray-600 dark:text-gray-300 text-sm">
                    {c.used_by || <span className="text-gray-400">-</span>}
                  </TableCell>
                  <TableCell className="text-gray-500 text-xs">
                    {c.used_at ? new Date(c.used_at).toLocaleString() : '-'}
                  </TableCell>
                  <TableCell className="text-right text-gray-500 text-xs">
                    {new Date(c.created_at).toLocaleString()}
                  </TableCell>
                  <TableCell className="text-right">
                     <Button type="button" variant="dangerIcon" onClick={() => handleDelete(c.id)} title="删除" aria-label="删除">
                       <Trash2 />
                     </Button>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </Card>
    </div>
  )
}
