import { useState, useEffect } from "react"
import { fetchApi } from "@/shared/api/client"
import { toast } from "sonner"
import { Plus, Percent, PencilLine } from "lucide-react"
import { Card, CardContent } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/shared/components/ui/dialog"
import { Badge } from "@/shared/components/ui/badge"

interface UserGroup {
  group_name: string
  discount_rate: number
}

export default function Pricing() {
  const [userGroups, setUserGroups] = useState<UserGroup[]>([])
  const [loading, setLoading] = useState(true)

  // Edit Group Discount
  const [openGroupForm, setOpenGroupForm] = useState(false)
  const [groupFormData, setGroupFormData] = useState({
    group_name: "",
    discount_rate: "",
  })

  useEffect(() => {
    loadData()
  }, [])

  const loadData = async () => {
    try {
      const groupsRes = await fetchApi("/admin/pricing/groups")
      setUserGroups(groupsRes.data || [])
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "无法加载定价数据")
    } finally {
      setLoading(false)
    }
  }

  const handleSaveGroup = async (e: React.FormEvent) => {
    e.preventDefault()
    try {
      await fetchApi("/admin/pricing/groups", {
        method: "POST",
        body: JSON.stringify({
          group_name: groupFormData.group_name,
          discount_rate: parseFloat(groupFormData.discount_rate),
        })
      })
      toast.success("分组倍率保存成功")
      setOpenGroupForm(false)
      loadData()
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "保存失败")
    }
  }

  const handleDeleteGroup = async (name: string) => {
    if(!confirm(`确定删除 ${name} 分组？所有属于该分组的 Key 将自动回退为 default (无折扣) 倍率。`)) return
    
    if (name === "default") {
      toast.error("default 分组为系统全局基准，不可删除。")
      return
    }

    try {
      await fetchApi(`/admin/pricing/groups/${encodeURIComponent(name)}`, { method: "DELETE" })
      toast.success("分组已删除")
      loadData()
    } catch(err: unknown) {
      toast.error(err instanceof Error ? err.message : "删除失败")
    }
  }

  return (
    <div className="space-y-12 animate-in fade-in duration-500" style={{ willChange: 'transform, opacity' }}>
      
      {/* Group Discounts Section */}
      <section className="space-y-4">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
          <div className="space-y-1">
             <h3 className="text-xl font-bold text-gray-900 dark:text-white flex items-center gap-2">
               <Percent className="w-5 h-5 text-amber-500" />
               用户分组计费倍率
             </h3>
             <p className="text-sm text-gray-500 max-w-2xl">
               您可以创建不同的分组（如 vip, svip）并赋予它们不同的结算倍率。API Key 在创建时可以分配到这些组。
             </p>
          </div>
          <Button 
             variant="outline"
             className="gap-2 border-amber-200 bg-amber-50 text-amber-700 hover:bg-amber-100 hover:text-amber-800 dark:border-amber-900/50 dark:bg-amber-900/20 dark:text-amber-400 dark:hover:bg-amber-900/40"
             onClick={() => {
               setGroupFormData({ group_name: "", discount_rate: "1.0" })
               setOpenGroupForm(true)
             }}
          >
            <Plus className="h-4 w-4" /> 新增特殊分组
          </Button>
        </div>

        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {loading ? (
             <div className="text-gray-500 col-span-3 text-sm p-4 text-center">加载分组中...</div>
          ) : userGroups.length === 0 ? (
             <div className="text-gray-500 col-span-3 text-sm p-4 text-center border rounded-xl border-dashed">
                尚未配置任何分组。系统所有计费将按原有 1.0 倍率标准执行。
             </div>
          ) : (
            userGroups.map((g) => (
              <Card key={g.group_name} className={`relative overflow-hidden group transition-all hover:shadow-md ${g.group_name === 'default' ? 'border-primary/20 bg-primary/5 dark:bg-primary/10' : ''}`}>
                 {g.group_name === 'default' && (
                   <div className="absolute top-0 right-0 py-1 px-3 bg-primary text-primary-foreground text-[10px] uppercase font-bold tracking-wider rounded-bl-xl shadow-sm z-10">
                      默认兜底组
                   </div>
                 )}
                 <CardContent className="p-5">
                   <div className="flex items-start justify-between">
                     <div className="flex items-center gap-3">
                        <div className={`w-12 h-12 rounded-xl flex items-center justify-center font-bold text-xl shadow-inner border border-white/10 ${g.group_name === 'default' ? 'bg-gradient-to-br from-indigo-500 to-purple-600 text-white' : 'bg-gradient-to-br from-amber-400 to-orange-500 text-white'}`}>
                           {g.group_name.substring(0, 1).toUpperCase()}
                        </div>
                        <div>
                           <h4 className="font-bold text-gray-900 dark:text-white text-lg leading-tight">
                             {g.group_name === 'default' ? 'Default' : g.group_name}
                           </h4>
                           <div className="flex items-center gap-1.5 mt-1">
                             <Badge variant="secondary" className="font-mono text-xs font-semibold">
                               x {g.discount_rate.toFixed(2)} 倍
                             </Badge>
                             {g.discount_rate < 1 ? (
                               <span className="text-[10px] text-emerald-500 font-medium border border-emerald-200 dark:border-emerald-800 bg-emerald-50 dark:bg-emerald-950/50 px-1 rounded">更便宜</span>
                             ) : g.discount_rate > 1 ? (
                               <span className="text-[10px] text-red-500 font-medium border border-red-200 dark:border-red-800 bg-red-50 dark:bg-red-950/50 px-1 rounded">更贵</span>
                             ) : null}
                           </div>
                        </div>
                     </div>
                   </div>

                   <div className="mt-4 pt-4 border-t border-border/50 flex justify-between items-center opacity-70 group-hover:opacity-100 transition-opacity">
                      <Button 
                         variant="ghost" 
                         size="sm"
                         className="h-8 text-xs font-medium"
                         onClick={() => {
                           setGroupFormData({ group_name: g.group_name, discount_rate: g.discount_rate.toString() })
                           setOpenGroupForm(true)
                         }}
                      >
                         <PencilLine className="w-3.5 h-3.5 mr-1" /> 编辑倍率
                      </Button>
                      {g.group_name !== 'default' && (
                         <Button 
                           variant="ghost" 
                           size="sm"
                           className="h-8 text-xs text-red-500 hover:text-red-700 hover:bg-red-50 dark:hover:bg-red-950/50"
                           onClick={() => handleDeleteGroup(g.group_name)}
                         >
                           删除分组
                         </Button>
                      )}
                   </div>
                 </CardContent>
              </Card>
            ))
          )}
        </div>
      </section>

      {/* Forms (Dialogs) */}
      <Dialog open={openGroupForm} onOpenChange={setOpenGroupForm}>
        <DialogContent className="sm:max-w-[400px]">
          <DialogHeader>
            <DialogTitle>设置分组计费倍率</DialogTitle>
            <DialogDescription>
              设定该分组用户的结算折扣或加成。1.0 为无折扣（原价）。
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSaveGroup} className="space-y-4 pt-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">分组名称</label>
              <Input 
                className="font-mono bg-gray-50 dark:bg-dark-900" 
                placeholder="例如: default, svip, internal" 
                value={groupFormData.group_name}
                onChange={e => setGroupFormData({...groupFormData, group_name: e.target.value})}
                required
                disabled={groupFormData.group_name === 'default' && groupFormData.discount_rate !== ''}
              />
              {groupFormData.group_name === 'default' && <p className="text-xs text-primary font-medium mt-1">"default" 是所有未指定分组用户的默认兜底倍率。</p>}
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">结算倍率 (Multiplier)</label>
              <Input 
                type="number"
                className="font-mono text-lg" 
                placeholder="1.0" 
                value={groupFormData.discount_rate}
                onChange={e => setGroupFormData({...groupFormData, discount_rate: e.target.value})}
                step="0.01"
                min="0"
                required
              />
              <div className="bg-gray-50 dark:bg-dark-900 p-3 rounded-lg text-xs text-gray-500 mt-2 space-y-1">
                 <p><span className="font-semibold text-gray-700 dark:text-gray-300">1.0</span> = 原价（基准价结账）</p>
                 <p><span className="font-semibold text-gray-700 dark:text-gray-300">0.5</span> = 五折（比原价便宜一半）</p>
                 <p><span className="font-semibold text-gray-700 dark:text-gray-300">2.0</span> = 双倍（比原价贵一倍）</p>
              </div>
            </div>
            <div className="flex justify-end gap-3 pt-4">
              <Button type="button" variant="outline" onClick={() => setOpenGroupForm(false)}>取消</Button>
              <Button type="submit">保存倍率配置</Button>
            </div>
          </form>
        </DialogContent>
      </Dialog>

    </div>
  )
}
