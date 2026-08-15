import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/components/ui/tabs"
import { LogsPanel } from '@/features/admin-proxy/components/LogsPanel'

export default function AdminProxyLogsPage() {
  return (
    <div className="space-y-6 max-w-5xl">
      <Card className="border-0 shadow-sm ring-1 ring-slate-200/50 dark:ring-white/10">
        <CardHeader className="border-b border-slate-100 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-900/50 rounded-t-xl">
          <CardTitle>日志监控面板</CardTitle>
          <CardDescription>查证底层代理网关的生命周期事件与细颗粒度的 HTTP 请求错误跟踪。</CardDescription>
        </CardHeader>
        <CardContent className="p-6">
          <Tabs defaultValue="system" className="w-full">
            <TabsList className="grid w-full grid-cols-2 max-w-lg mb-6">
              <TabsTrigger value="system">系统运行日志 (System)</TabsTrigger>
              <TabsTrigger value="errors">请求错误追溯 (Errors)</TabsTrigger>
            </TabsList>
            <TabsContent value="system" className="focus-visible:outline-none">
              <LogsPanel endpoint="/logs" allowClear={true} />
            </TabsContent>
            <TabsContent value="errors" className="focus-visible:outline-none">
              <LogsPanel endpoint="/request-error-logs" allowClear={false} />
            </TabsContent>
          </Tabs>
        </CardContent>
      </Card>
    </div>
  )
}
