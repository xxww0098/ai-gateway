import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X, ShieldAlert, Eye, EyeOff } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import type { AuthFileEditFields, AuthFileItem } from '../types'

interface AuthFileEditDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  item: AuthFileItem | null
  saving: boolean
  onSave: (fields: AuthFileEditFields) => void
}

interface BaseFields {
  label: string
  prefix: string
  proxy_url: string
  base_url: string
  project_id: string
  location: string
}

interface SecretFields {
  api_key: string
  access_token: string
  refresh_token: string
  id_token: string
  service_account: string
}

const emptyBase: BaseFields = {
  label: '',
  prefix: '',
  proxy_url: '',
  base_url: '',
  project_id: '',
  location: '',
}

const emptySecrets: SecretFields = {
  api_key: '',
  access_token: '',
  refresh_token: '',
  id_token: '',
  service_account: '',
}

export function AuthFileEditDialog({ open, onOpenChange, item, saving, onSave }: AuthFileEditDialogProps) {
  const [base, setBase] = useState<BaseFields>(emptyBase)
  const [secrets, setSecrets] = useState<SecretFields>(emptySecrets)
  const [editSecrets, setEditSecrets] = useState(false)
  const [showSecrets, setShowSecrets] = useState<Record<keyof SecretFields, boolean>>({
    api_key: false,
    access_token: false,
    refresh_token: false,
    id_token: false,
    service_account: false,
  })
  const [error, setError] = useState<string | null>(null)

  // Sync from item every time the dialog opens.
  useEffect(() => {
    if (!open || !item) return
    setBase({
      label: typeof item.label === 'string' ? item.label : '',
      prefix: typeof item.prefix === 'string' ? item.prefix : '',
      proxy_url: typeof item.proxy_url === 'string' ? item.proxy_url : '',
      base_url: typeof item.base_url === 'string' ? item.base_url : '',
      project_id: typeof item.project_id === 'string' ? item.project_id : '',
      location: typeof item.location === 'string' ? item.location : '',
    })
    setSecrets(emptySecrets)
    setEditSecrets(false)
    setShowSecrets({ api_key: false, access_token: false, refresh_token: false, id_token: false, service_account: false })
    setError(null)
  }, [open, item])

  const provider = (item?.provider || '').toLowerCase()
  const secretLayout = useMemo(() => {
    switch (provider) {
      case 'vertex':
        return { fields: ['service_account'] as Array<keyof SecretFields>, intro: '粘贴完整的 Google service account JSON。留空将清除。' }
      case 'openai':
      case 'anthropic':
      case 'claude-api-key':
        return { fields: ['api_key'] as Array<keyof SecretFields>, intro: '替换上游 API key。' }
      case 'gemini':
      case 'claude':
      case 'codex':
        return {
          fields: ['access_token', 'refresh_token', 'id_token', 'api_key'] as Array<keyof SecretFields>,
          intro: 'OAuth token 通常由刷新流程自动维护，仅在 token 永久失效时手动覆盖。',
        }
      default:
        return {
          fields: ['api_key', 'access_token', 'refresh_token', 'id_token', 'service_account'] as Array<keyof SecretFields>,
          intro: '按渠道类型选择需要更新的密钥。',
        }
    }
  }, [provider])

  const runtimeOnly = item?.runtime_only === true
  const targetID = item?.id || item?.auth_id || item?.name || ''

  const handleSave = () => {
    if (!item) return
    setError(null)
    const fields: AuthFileEditFields = {
      label: base.label.trim(),
      prefix: base.prefix.trim(),
      proxy_url: base.proxy_url.trim(),
      base_url: base.base_url.trim(),
      project_id: base.project_id.trim(),
      location: base.location.trim(),
    }
    if (editSecrets) {
      for (const key of secretLayout.fields) {
        const value = secrets[key]
        if (value === '') continue
        if (key === 'service_account') {
          try {
            JSON.parse(value)
          } catch {
            setError('service_account 必须是合法的 JSON')
            return
          }
        }
        fields[key] = value
      }
    }
    onSave(fields)
  }

  if (!item) return null

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content className="fixed left-[50%] top-[50%] z-50 w-full max-w-[560px] max-h-[90vh] overflow-y-auto translate-x-[-50%] translate-y-[-50%] border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 p-6 shadow-2xl sm:rounded-2xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95">
          <DialogPrimitive.Title className="text-lg font-semibold text-gray-900 dark:text-white">编辑凭证</DialogPrimitive.Title>
          <DialogPrimitive.Description className="text-sm text-gray-500 dark:text-dark-300 mt-1">
            渠道 <span className="font-medium text-gray-700 dark:text-gray-300">{item.provider || '未知'}</span> · ID{' '}
            <code className="text-xs bg-gray-100 dark:bg-dark-700 px-1 py-0.5 rounded">{targetID}</code>
          </DialogPrimitive.Description>

          {runtimeOnly && (
            <div className="mt-3 flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 dark:border-amber-900/60 dark:bg-amber-950/30 p-3 text-xs text-amber-700 dark:text-amber-300">
              <ShieldAlert className="h-4 w-4 mt-0.5 flex-shrink-0" />
              <div>此凭证由 config.yaml 注入，无法在数据库中持久化编辑。请直接修改配置文件。</div>
            </div>
          )}

          {/* Non-secret fields */}
          <fieldset disabled={saving || runtimeOnly} className="mt-4 grid grid-cols-1 gap-3 sm:grid-cols-2">
            <Field label="标签" placeholder="人类可读的名称" value={base.label} onChange={(v) => setBase((s) => ({ ...s, label: v }))} />
            <Field label="前缀 (prefix)" placeholder="可选" value={base.prefix} onChange={(v) => setBase((s) => ({ ...s, prefix: v }))} />
            <Field label="代理 (proxy_url)" placeholder="http://user:pass@host:port" value={base.proxy_url} onChange={(v) => setBase((s) => ({ ...s, proxy_url: v }))} className="sm:col-span-2" />
            <Field label="Base URL" placeholder="https://api.example.com" value={base.base_url} onChange={(v) => setBase((s) => ({ ...s, base_url: v }))} className="sm:col-span-2" />
            <Field label="Project ID" placeholder="GCP project id" value={base.project_id} onChange={(v) => setBase((s) => ({ ...s, project_id: v }))} />
            <Field label="Location" placeholder="us-central1" value={base.location} onChange={(v) => setBase((s) => ({ ...s, location: v }))} />
          </fieldset>

          {/* Secret editor */}
          <div className="mt-5 rounded-lg border border-gray-200 dark:border-dark-600 bg-gray-50 dark:bg-dark-800/40">
            <label className="flex cursor-pointer items-center gap-2 p-3 text-sm font-medium text-gray-700 dark:text-gray-200">
              <input
                type="checkbox"
                className="h-3.5 w-3.5 rounded border-gray-300 text-primary-600 focus:ring-primary-500 dark:border-dark-600 dark:bg-dark-800"
                checked={editSecrets}
                onChange={(e) => setEditSecrets(e.target.checked)}
                disabled={runtimeOnly || saving}
              />
              修改密钥 / Token
            </label>
            {editSecrets && (
              <div className="border-t border-gray-200 dark:border-dark-600 p-3 space-y-3">
                <p className="text-xs text-gray-500 dark:text-gray-400">{secretLayout.intro} 留空将保持原值不变。</p>
                {secretLayout.fields.map((key) => (
                  <SecretField
                    key={key}
                    label={SECRET_LABELS[key]}
                    value={secrets[key]}
                    visible={showSecrets[key]}
                    multiline={key === 'service_account'}
                    onChange={(v) => setSecrets((s) => ({ ...s, [key]: v }))}
                    onToggleVisible={() => setShowSecrets((s) => ({ ...s, [key]: !s[key] }))}
                  />
                ))}
              </div>
            )}
          </div>

          {error && <p className="mt-3 text-xs text-red-600 dark:text-red-400">{error}</p>}

          <div className="flex justify-end gap-3 mt-6">
            <DialogPrimitive.Close asChild>
              <button className="btn btn-ghost px-4 text-sm" disabled={saving}>
                取消
              </button>
            </DialogPrimitive.Close>
            <button className="btn btn-primary px-5 text-sm" onClick={handleSave} disabled={saving || runtimeOnly}>
              {saving ? '保存中...' : '保存修改'}
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

const SECRET_LABELS: Record<keyof SecretFields, string> = {
  api_key: 'API Key',
  access_token: 'Access Token',
  refresh_token: 'Refresh Token',
  id_token: 'ID Token',
  service_account: 'Service Account JSON',
}

function Field({ label, placeholder, value, onChange, className }: { label: string; placeholder?: string; value: string; onChange: (v: string) => void; className?: string }) {
  return (
    <label className={`block ${className || ''}`}>
      <span className="text-xs font-medium text-gray-600 dark:text-gray-300">{label}</span>
      <input
        type="text"
        className="mt-1 w-full rounded-lg border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 px-3 py-1.5 text-sm text-gray-900 dark:text-white placeholder:text-gray-400 focus:border-primary-400 focus:ring-1 focus:ring-primary-400/30 outline-none disabled:cursor-not-allowed disabled:opacity-60"
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  )
}

function SecretField({
  label,
  value,
  visible,
  multiline,
  onChange,
  onToggleVisible,
}: {
  label: string
  value: string
  visible: boolean
  multiline?: boolean
  onChange: (v: string) => void
  onToggleVisible: () => void
}) {
  const baseClass =
    'w-full rounded-lg border border-gray-200 dark:border-dark-600 bg-white dark:bg-dark-900 pl-3 pr-9 py-1.5 text-sm font-mono text-gray-900 dark:text-white placeholder:text-gray-400 focus:border-primary-400 focus:ring-1 focus:ring-primary-400/30 outline-none'
  return (
    <label className="block">
      <span className="text-xs font-medium text-gray-600 dark:text-gray-300">{label}</span>
      <div className="relative mt-1">
        {multiline ? (
          <textarea
            rows={6}
            className={`${baseClass} resize-y${visible ? '' : ' blur-sm focus:blur-none'}`}
            placeholder='{"type":"service_account",...}'
            value={value}
            onChange={(e) => onChange(e.target.value)}
            spellCheck={false}
          />
        ) : (
          <input
            type={visible ? 'text' : 'password'}
            className={baseClass}
            placeholder="留空保持不变"
            value={value}
            onChange={(e) => onChange(e.target.value)}
            spellCheck={false}
            autoComplete="off"
          />
        )}
        <button
          type="button"
          onClick={onToggleVisible}
          className="absolute right-2 top-2 rounded p-0.5 text-gray-400 hover:text-gray-700 dark:hover:text-gray-200"
          tabIndex={-1}
          aria-label={visible ? '隐藏' : '显示'}
        >
          {visible ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
        </button>
      </div>
    </label>
  )
}
