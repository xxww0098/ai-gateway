import { useEffect, useState, useCallback } from 'react'
import { Button } from '@/shared/components/ui/button'
import { Loader2, RefreshCcw, Trash2, AlertCircle } from 'lucide-react'
import { toast } from 'sonner'
import { fetchProviderConfig, deleteProviderConfig } from '../api'
import { Link } from 'react-router-dom'
import { adminSettingsTab } from '@/shared/routes/admin'

interface LogsPanelProps {
  endpoint: string
  allowClear?: boolean
}

export function LogsPanel({ endpoint, allowClear }: LogsPanelProps) {
  const [logs, setLogs] = useState<string>('')
  const [loading, setLoading] = useState(true)
  const [isDisabled, setIsDisabled] = useState(false)

  const fetchLogs = useCallback(async () => {
    setLoading(true)
    setIsDisabled(false)
    try {
      const data = await fetchProviderConfig(endpoint)

      let logsText = ''
      if (typeof data === 'string') {
        logsText = data
      } else if (Array.isArray(data)) {
        logsText = data.map(i => typeof i === 'object' ? JSON.stringify(i) : i).join('\n')
      } else {
        logsText = JSON.stringify(data, null, 2)
      }

      setLogs(logsText || '暂无日志内容...')
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e)
      if (msg.includes('logging to file disabled') || msg.includes('not enabled')) {
        setIsDisabled(true)
      } else {
        toast.error(`获取日志失败: ${msg}`)
        setLogs(`Error: ${msg}`)
      }
    } finally {
      setLoading(false)
    }
  }, [endpoint])

  const handleClear = async () => {
    try {
      await deleteProviderConfig(endpoint)
      toast.success('已清空日志')
      setLogs('暂无日志内容...')
    } catch (e: unknown) {
      toast.error(`清空日志失败: ${e instanceof Error ? e.message : String(e)}`)
    }
  }

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void fetchLogs()
    }, 0)
    return () => window.clearTimeout(timer)
  }, [fetchLogs])

  return (
    <div className="space-y-4 mt-4">
      <div className="flex gap-2 justify-end">
        <Button variant="outline" onClick={fetchLogs} disabled={loading || isDisabled}>
          <RefreshCcw className={`h-4 w-4 mr-2 ${loading ? 'animate-spin' : ''}`} />
          刷新日志
        </Button>
        {allowClear && !isDisabled && (
          <Button variant="destructive" onClick={handleClear} disabled={loading}>
            <Trash2 className="h-4 w-4 mr-2" />
            清空日志
          </Button>
        )}
      </div>

      <div className="border rounded-xl bg-slate-50/50 dark:bg-slate-900/50 p-4 h-[60vh] overflow-y-auto">
        {loading && !logs && !isDisabled ? (
          <div className="flex justify-center items-center h-full text-slate-400">
            <Loader2 className="h-6 w-6 animate-spin" />
            <span className="ml-2">读取离线日志中...</span>
          </div>
        ) : isDisabled ? (
          <div className="flex flex-col justify-center items-center h-full text-slate-500 p-6 text-center space-y-4 animate-in fade-in duration-500" style={{ willChange: 'transform, opacity' }}>
            <div className="w-16 h-16 rounded-full bg-amber-100 dark:bg-amber-900/30 flex items-center justify-center">
              <AlertCircle className="h-8 w-8 text-amber-500" />
            </div>
            <div className="space-y-1.5 max-w-sm">
              <h3 className="font-semibold text-slate-800 dark:text-slate-200">磁盘日志留存未开启</h3>
              <p className="text-sm text-slate-500 dark:text-slate-400 leading-relaxed">
                引擎为了降低磁盘磨损与保护性能，默认没有直接向物理文件写入追踪日志。
              </p>
            </div>
            <Link to={adminSettingsTab('config')}>
              <Button variant="secondary" className="mt-4 shadow-sm">
                 <AlertCircle className="h-4 w-4 mr-2 opacity-70" />
                 前往「系统设置」一键开启
              </Button>
            </Link>
          </div>
        ) : (
          <pre className="text-xs font-mono text-slate-700 dark:text-slate-300 whitespace-pre-wrap break-words">{logs}</pre>
        )}
      </div>
    </div>
  )
}
