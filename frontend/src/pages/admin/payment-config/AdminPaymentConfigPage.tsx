import { useState, useEffect, useCallback } from "react"
import { fetchApi } from "@/shared/api/client"
import { toast } from "sonner"
import { zodResolver } from "@hookform/resolvers/zod"
import { useForm, Controller } from "react-hook-form"
import * as z from "zod"

import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/shared/components/ui/card"
import { Button } from "@/shared/components/ui/button"
import { Input } from "@/shared/components/ui/input"
import { Switch } from "@/shared/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/components/ui/select"
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/shared/components/ui/form"

import {
  CreditCard,
  Eye,
  EyeOff,
  Save,
  Loader2,
  CircleDollarSign,
  Wallet,
  Landmark,
} from "lucide-react"

interface PaymentConfigItem {
  id?: number
  provider: string
  app_id?: string | null
  app_secret: string
  webhook_secret: string
  mode: string
  enabled: boolean
  extra?: string | null
}

const PROVIDERS = [
  { key: "stripe", label: "Stripe", icon: CreditCard },
  { key: "alipay", label: "支付宝", icon: CircleDollarSign },
  { key: "wechat", label: "微信支付", icon: Wallet },
] as const

const makeSchema = (hasExistingSecret: boolean) =>
  z
    .object({
      app_id: z.string(),
      app_secret: z.string(),
      webhook_secret: z.string(),
      mode: z.enum(["sandbox", "prod"]),
      enabled: z.boolean(),
    })
    .refine(
      (data) => {
        if (!data.enabled) return true
        return data.app_id.trim() !== ""
      },
      { message: "启用时必须填写 App ID", path: ["app_id"] }
    )
    .refine(
      (data) => {
        if (!data.enabled) return true
        if (hasExistingSecret) return true
        return data.app_secret.trim() !== ""
      },
      { message: "启用时必须填写 App Secret", path: ["app_secret"] }
    )

type FormValues = z.infer<ReturnType<typeof makeSchema>>

function ProviderCard({
  provider,
  config,
  onSaved,
}: {
  provider: string
  config?: PaymentConfigItem
  onSaved: () => void
}) {
  const providerMeta = PROVIDERS.find((p) => p.key === provider)
  const Icon = providerMeta?.icon || Landmark

  const hasAppSecret = config?.app_secret === "***"
  const hasWebhookSecret = config?.webhook_secret === "***"

  const [saving, setSaving] = useState(false)
  const [showAppSecret, setShowAppSecret] = useState(false)
  const [showWebhookSecret, setShowWebhookSecret] = useState(false)

  const form = useForm<FormValues>({
    resolver: zodResolver(makeSchema(hasAppSecret)),
    defaultValues: {
      app_id: config?.app_id || "",
      app_secret: "",
      webhook_secret: "",
      mode: (config?.mode as "sandbox" | "prod") || "sandbox",
      enabled: config?.enabled || false,
    },
  })

  useEffect(() => {
    form.reset({
      app_id: config?.app_id || "",
      app_secret: "",
      webhook_secret: "",
      mode: (config?.mode as "sandbox" | "prod") || "sandbox",
      enabled: config?.enabled || false,
    })
  }, [config, form])

  const onSubmit = async (values: FormValues) => {
    setSaving(true)
    try {
      const payload: Record<string, unknown> = {
        provider,
        app_id: values.app_id.trim() || null,
        mode: values.mode,
        enabled: values.enabled,
      }

      if (values.app_secret.trim() !== "") {
        payload.app_secret = values.app_secret.trim()
      } else if (!hasAppSecret) {
        payload.app_secret = values.app_secret
      } else {
        payload.app_secret = null
      }

      if (values.webhook_secret.trim() !== "") {
        payload.webhook_secret = values.webhook_secret.trim()
      } else if (!hasWebhookSecret) {
        payload.webhook_secret = values.webhook_secret
      } else {
        payload.webhook_secret = null
      }

      await fetchApi("/admin/payment-config", {
        method: "PUT",
        body: JSON.stringify(payload),
      })
      toast.success(`${providerMeta?.label || provider} 配置已保存`)
      onSaved()
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "保存失败")
    } finally {
      setSaving(false)
    }
  }

  return (
    <Card className="shadow-sm border-border">
      <CardHeader className="pb-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-primary/10 text-primary flex items-center justify-center">
              <Icon className="w-5 h-5" />
            </div>
            <div>
              <CardTitle className="text-lg">{providerMeta?.label || provider}</CardTitle>
              <CardDescription>
                {provider === "stripe" ? "信用卡 / 借记卡支付" : provider === "alipay" ? "支付宝扫码支付" : "微信支付扫码支付"}
              </CardDescription>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-sm text-muted-foreground">
              {form.watch("enabled") ? "已启用" : "已禁用"}
            </span>
            <Controller
              control={form.control}
              name="enabled"
              render={({ field }) => (
                <Switch
                  checked={field.value}
                  onCheckedChange={field.onChange}
                  aria-label="启用支付渠道"
                />
              )}
            />
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-5">
            <FormField
              control={form.control}
              name="app_id"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>App ID</FormLabel>
                  <FormControl>
                    <Input
                      placeholder={provider === "stripe" ? "pk_live_... / pk_test_..." : "应用 ID"}
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="app_secret"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>App Secret</FormLabel>
                  <FormControl>
                    <div className="relative">
                      <Input
                        type={showAppSecret ? "text" : "password"}
                        placeholder={hasAppSecret ? "••• (已设置，填写则覆盖)" : "请输入 App Secret"}
                        {...field}
                      />
                      <button
                        type="button"
                        onClick={() => setShowAppSecret((v) => !v)}
                        className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                        tabIndex={-1}
                      >
                        {showAppSecret ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                      </button>
                    </div>
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="webhook_secret"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Webhook Secret</FormLabel>
                  <FormControl>
                    <div className="relative">
                      <Input
                        type={showWebhookSecret ? "text" : "password"}
                        placeholder={hasWebhookSecret ? "••• (已设置，填写则覆盖)" : "请输入 Webhook Secret"}
                        {...field}
                      />
                      <button
                        type="button"
                        onClick={() => setShowWebhookSecret((v) => !v)}
                        className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                        tabIndex={-1}
                      >
                        {showWebhookSecret ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                      </button>
                    </div>
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="mode"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>运行模式</FormLabel>
                  <Select onValueChange={field.onChange} value={field.value}>
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue placeholder="选择运行模式" />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="sandbox">测试模式 (Sandbox)</SelectItem>
                      <SelectItem value="prod">生产模式 (Production)</SelectItem>
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />

            <div className="flex justify-end pt-2">
              <Button type="submit" disabled={saving} className="gap-2">
                {saving ? <Loader2 className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
                {saving ? "保存中..." : "保存配置"}
              </Button>
            </div>
          </form>
        </Form>
      </CardContent>
    </Card>
  )
}

type PaymentConfigProps = {
  /** When true, omit page-level chrome (embedded in System settings tab). */
  embedded?: boolean
}

export default function PaymentConfig({ embedded = true }: PaymentConfigProps) {
  const [configs, setConfigs] = useState<PaymentConfigItem[]>([])
  const [loading, setLoading] = useState(true)

  const loadConfigs = useCallback(async () => {
    setLoading(true)
    try {
      const res = await fetchApi("/admin/payment-config")
      setConfigs(res.data || [])
    } catch (err: unknown) {
      toast.error(err instanceof Error ? err.message : "加载配置失败")
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadConfigs()
  }, [loadConfigs])

  const getConfig = (provider: string) => configs.find((c) => c.provider === provider)

  return (
    <div
      className={`space-y-6 max-w-4xl mx-auto ${embedded ? '' : 'animate-in fade-in duration-500 px-4 sm:px-6'}`}
      style={embedded ? undefined : { willChange: 'transform, opacity' }}
    >
      {!embedded && (
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white">支付配置</h2>
          <p className="text-gray-500 dark:text-dark-300 mt-1">
            管理各支付渠道的接入参数、运行模式与启用状态。敏感信息将以加密形式存储。
          </p>
        </div>
      )}
      {embedded && (
        <p className="text-sm text-gray-500 dark:text-dark-300">
          配置各支付渠道参数与启用状态。密钥以加密形式存储。
        </p>
      )}

      {loading ? (
        <div className="space-y-4">
          {PROVIDERS.map((p) => (
            <Card key={p.key} className="h-64 animate-pulse bg-muted/40" />
          ))}
        </div>
      ) : (
        <div className="grid gap-6">
          {PROVIDERS.map((p) => (
            <ProviderCard
              key={p.key}
              provider={p.key}
              config={getConfig(p.key)}
              onSaved={loadConfigs}
            />
          ))}
        </div>
      )}
    </div>
  )
}
