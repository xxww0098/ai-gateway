import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { Button } from '@/shared/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/shared/components/ui/dialog';
import { BookOpen, Check, Copy, Cpu, Loader2, Plus, Radio, RefreshCw, Search, Trash2 } from 'lucide-react';
import { Input } from '@/shared/components/ui/input';
import { modelsApi, normalizeModelList, type ModelInfo } from '@/features/pricing/model_prices';
import type { ModelCatalogItem } from '@/features/pricing/model_catalog';
import { configuredModelToCatalogItem } from '@/features/pricing/model_catalog';
import { toast } from 'sonner';
import type { ProviderKind } from '@/features/admin-proxy/providerConfig';
import { copyTextToClipboard } from '@/shared/utils/clipboard';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  provider: string; // "OpenAI (兼容格式)" etc
  providerKind: ProviderKind;
  baseUrl?: string;
  modelsUrl?: string;
  apiKey?: string;
  configuredModels?: unknown;
  onSaveConfiguredModels?: (models: ModelInfo[]) => Promise<void>;
  onSaveModelsUrl?: (modelsUrl: string) => Promise<void>;
}

function resolveChannelProviderKey(providerKind: ProviderKind, providerLabel: string): string {
  if (providerKind === 'claude') return 'anthropic';
  if (providerKind === 'gemini' || providerKind === 'vertex') return 'google';
  if (providerKind === 'codex') return 'openai';

  const lower = providerLabel.toLowerCase();
  if (lower.includes('minimax')) return 'minimax';
  if (lower.includes('deepseek')) return 'deepseek';
  if (lower.includes('moonshot') || lower.includes('kimi')) return 'kimi';
  if (lower.includes('zhipu') || lower.includes('bigmodel') || lower.includes('glm')) return 'glm';
  if (lower.includes('xiaomi') || lower.includes('mimo')) return 'mimo';
  if (lower.includes('claude') || lower.includes('anthropic')) return 'anthropic';
  if (lower.includes('gemini') || lower.includes('google')) return 'google';
  if (lower.includes('openai')) return 'openai';

  return providerKind;
}

function modelKey(model: ModelInfo): string {
  return String(model.name || '').trim().toLowerCase();
}

function mergeModelInfoList(base: ModelInfo[], additions: ModelInfo[]): ModelInfo[] {
  const seen = new Set<string>();
  const out: ModelInfo[] = [];
  [...base, ...additions].forEach((model) => {
    const key = modelKey(model);
    if (!key || seen.has(key)) return;
    seen.add(key);
    out.push(model);
  });
  return out;
}

export function ProviderModelsDialog({
  open,
  onOpenChange,
  provider,
  providerKind,
  baseUrl = '',
  modelsUrl = '',
  apiKey = '',
  configuredModels,
  onSaveConfiguredModels,
  onSaveModelsUrl,
}: Props) {
  const channelProviderKey = useMemo(() => resolveChannelProviderKey(providerKind, provider), [providerKind, provider]);

  const configuredList = useMemo(
    () => normalizeModelList(configuredModels, { dedupe: true }),
    [configuredModels]
  );

  const [search, setSearch] = useState('');

  // Live probe state
  const [draftConfiguredModels, setDraftConfiguredModels] = useState<ModelInfo[]>([]);
  const [probeModels, setProbeModels] = useState<ModelInfo[]>([]);
  const [probeLoading, setProbeLoading] = useState(false);
  const [probeError, setProbeError] = useState('');
  const [savingModelKey, setSavingModelKey] = useState('');
  const [modelsUrlInput, setModelsUrlInput] = useState(modelsUrl);
  const [savingModelsUrl, setSavingModelsUrl] = useState(false);
  const wasOpenRef = useRef(false);
  const defaultOpenAIModelsUrl = useMemo(
    () => modelsApi.buildOpenAIModelsEndpoint(baseUrl || 'https://api.openai.com'),
    [baseUrl]
  );
  const effectiveOpenAIModelsUrl = useMemo(
    () => modelsApi.buildOpenAIModelsEndpoint(baseUrl || 'https://api.openai.com', modelsUrlInput),
    [baseUrl, modelsUrlInput]
  );
  const customOpenAIModelsUrl = modelsUrlInput.trim() !== '' && modelsUrlInput.trim() !== defaultOpenAIModelsUrl;
  const openAIModelsUrlMode = customOpenAIModelsUrl ? '自定义' : '默认推导';

  const fetchProbeModels = useCallback(async (modelsUrlOverride?: string) => {
    setProbeLoading(true);
    setProbeError('');
    try {
      let list: ModelInfo[] = [];
      if (providerKind === 'claude') {
        list = await modelsApi.fetchClaudeModelsViaApiCall(baseUrl, apiKey);
      } else if (providerKind === 'gemini' || providerKind === 'vertex') {
        list = await modelsApi.fetchGeminiModelsViaApiCall(baseUrl, apiKey);
      } else {
        list = await modelsApi.fetchOpenAIModelsViaApiCall(
          baseUrl || 'https://api.openai.com',
          apiKey,
          {},
          modelsUrlOverride ?? modelsUrlInput
        );
      }
      setProbeModels(list);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : '探测模型失败';
      const hinted =
        /HTTP\s+404\b/.test(msg)
          ? `${msg}\n💡 提示：该上游未提供模型列表 API（部分兼容平台常见情况）。\n这不影响功能使用。您可放心直接在客户端中指定如 \`gpt-4\` 等模型名发起调用即可。`
          : msg;
      setProbeError(hinted);
      toast.error('探测模型中断：' + msg);
    } finally {
      setProbeLoading(false);
    }
  }, [providerKind, baseUrl, modelsUrlInput, apiKey]);

  const handleSaveModelsUrl = useCallback(async () => {
    if (!onSaveModelsUrl) return;
    setSavingModelsUrl(true);
    try {
      const nextModelsUrl = customOpenAIModelsUrl ? modelsUrlInput.trim() : '';
      await onSaveModelsUrl(nextModelsUrl);
      toast.success('模型列表 URL 已保存');
    } catch (e: unknown) {
      toast.error(`保存模型列表 URL 失败: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setSavingModelsUrl(false);
    }
  }, [customOpenAIModelsUrl, modelsUrlInput, onSaveModelsUrl]);

  const handleCopyEffectiveModelsUrl = useCallback(async () => {
    if (!effectiveOpenAIModelsUrl) return;
    const ok = await copyTextToClipboard(effectiveOpenAIModelsUrl);
    if (ok) {
      toast.success('实际请求 URL 已复制');
    } else {
      toast.error('复制失败，请手动选择 URL');
    }
  }, [effectiveOpenAIModelsUrl]);

  const persistConfiguredModels = useCallback(async (nextModels: ModelInfo[], successMessage: string) => {
    if (!onSaveConfiguredModels) return;
    setSavingModelKey('__all__');
    try {
      await onSaveConfiguredModels(nextModels);
      setDraftConfiguredModels(nextModels);
      toast.success(successMessage);
    } catch (e: unknown) {
      toast.error(`保存模型配置失败: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setSavingModelKey('');
    }
  }, [onSaveConfiguredModels]);

  const handleAddProbeModel = useCallback(async (model: ModelInfo) => {
    if (!onSaveConfiguredModels) return;
    const key = modelKey(model);
    if (!key) return;
    const nextModels = mergeModelInfoList(draftConfiguredModels, [model]);
    if (nextModels.length === draftConfiguredModels.length) return;
    setSavingModelKey(key);
    try {
      await onSaveConfiguredModels(nextModels);
      setDraftConfiguredModels(nextModels);
      toast.success(`已加入 ${model.name}`);
    } catch (e: unknown) {
      toast.error(`加入模型失败: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setSavingModelKey('');
    }
  }, [draftConfiguredModels, onSaveConfiguredModels]);

  const handleRemoveConfiguredModel = useCallback(async (model: ModelInfo) => {
    if (!onSaveConfiguredModels) return;
    const key = modelKey(model);
    const nextModels = draftConfiguredModels.filter((item) => modelKey(item) !== key);
    setSavingModelKey(key);
    try {
      await onSaveConfiguredModels(nextModels);
      setDraftConfiguredModels(nextModels);
      toast.success(`已移除 ${model.name}`);
    } catch (e: unknown) {
      toast.error(`移除模型失败: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setSavingModelKey('');
    }
  }, [draftConfiguredModels, onSaveConfiguredModels]);

  // Reset state when dialog opens.
  useEffect(() => {
    const wasOpen = wasOpenRef.current;
    wasOpenRef.current = open;
    if (!open || wasOpen) return;

    setSearch('');
    setDraftConfiguredModels(configuredList);
    setProbeModels([]);
    setProbeError('');
    setSavingModelKey('');
    const initialModelsUrl = modelsUrl.trim() || defaultOpenAIModelsUrl;
    setModelsUrlInput(initialModelsUrl);
    setSavingModelsUrl(false);
    void fetchProbeModels(initialModelsUrl);
  }, [open, configuredList, modelsUrl, defaultOpenAIModelsUrl, fetchProbeModels]);

  // Filter helpers
  const configuredModelItems = useMemo(() => {
    return draftConfiguredModels.map((model) => ({
      key: modelKey(model),
      raw: model,
      model: {
        ...configuredModelToCatalogItem(model, { provider: channelProviderKey }),
        owned_by: channelProviderKey,
      } as ModelCatalogItem,
    })).filter((entry) => entry.key);
  }, [draftConfiguredModels, channelProviderKey]);

  const configuredModelKeySet = useMemo(() => new Set(configuredModelItems.map((entry) => entry.key)), [configuredModelItems]);

  const probeModelItems = useMemo(() => {
    return probeModels.map((model) => ({
      key: modelKey(model),
      raw: model,
      model: {
        ...configuredModelToCatalogItem(model, { provider: channelProviderKey }),
        owned_by: channelProviderKey,
      } as ModelCatalogItem,
    })).filter((entry) => entry.key);
  }, [probeModels, channelProviderKey]);

  const filteredProbe = useMemo(() => {
    if (!search.trim()) return probeModelItems;
    const q = search.toLowerCase();
    return probeModelItems.filter(({ raw, model }) =>
      (raw.name || '').toLowerCase().includes(q) ||
      (raw.alias || '').toLowerCase().includes(q) ||
      (model.description || '').toLowerCase().includes(q)
    );
  }, [probeModelItems, search]);

  const filteredConfigured = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return configuredModelItems;
    return configuredModelItems.filter(({ raw, model }) =>
      (raw.name || '').toLowerCase().includes(q) ||
      (raw.alias || '').toLowerCase().includes(q) ||
      (model.id || '').toLowerCase().includes(q) ||
      (model.display_name || '').toLowerCase().includes(q) ||
      (model.description || '').toLowerCase().includes(q)
    );
  }, [configuredModelItems, search]);

  const unconfiguredProbeModels = useMemo(
    () => probeModels.filter((model) => !configuredModelKeySet.has(modelKey(model))),
    [probeModels, configuredModelKeySet]
  );

  const renderLoading = (text: string) => (
    <div className="flex flex-col items-center justify-center p-10 h-full text-gray-400 dark:text-dark-500 gap-2">
      <Loader2 className="h-6 w-6 animate-spin" />
      <span className="text-sm">{text}</span>
    </div>
  );

  const renderError = (error: string) => (
    <div className="p-4 text-sm text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-950/20 border border-red-200 dark:border-red-800/40 rounded-lg whitespace-pre-wrap">
      <strong>请求失败：</strong> {error}
    </div>
  );

  const renderEmpty = (text: string) => (
    <div className="p-10 text-center text-sm text-gray-400 dark:text-dark-500">{text}</div>
  );

  const renderModelRow = (
    entry: { key: string; raw: ModelInfo; model: ModelCatalogItem },
    side: 'configured' | 'probe'
  ) => {
    const exists = configuredModelKeySet.has(entry.key);
    const busy = savingModelKey === entry.key || savingModelKey === '__all__';
    const title = entry.model.display_name || entry.model.id;
    const subtitle = entry.raw.alias && entry.raw.alias !== entry.raw.name ? entry.raw.name : entry.model.description;

    return (
      <div
        key={`${side}-${entry.key}`}
        className={`group flex min-h-[76px] items-center gap-3 rounded-lg border bg-white px-3 py-2.5 transition-colors dark:bg-dark-900 ${
          side === 'probe' && !exists && onSaveConfiguredModels
            ? 'cursor-pointer border-gray-200 hover:border-primary-300 hover:bg-primary-50/40 dark:border-dark-700 dark:hover:border-primary-700 dark:hover:bg-primary-950/20'
            : exists && side === 'probe'
              ? 'border-emerald-200 bg-emerald-50/40 dark:border-emerald-900/50 dark:bg-emerald-950/10'
              : 'border-gray-200 dark:border-dark-700'
        }`}
        role={side === 'probe' && !exists && onSaveConfiguredModels ? 'button' : undefined}
        tabIndex={side === 'probe' && !exists && onSaveConfiguredModels ? 0 : undefined}
        onClick={() => {
          if (side === 'probe' && !exists && onSaveConfiguredModels && !busy) {
            void handleAddProbeModel(entry.raw);
          }
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && side === 'probe' && !exists && onSaveConfiguredModels && !busy) {
            event.preventDefault();
            void handleAddProbeModel(entry.raw);
          }
        }}
      >
        <div className="min-w-0 flex-1">
          <div className="mb-1 flex flex-wrap items-center gap-1.5">
            <span className="rounded bg-emerald-50 px-1.5 py-0.5 text-[10px] font-semibold uppercase text-emerald-700 dark:bg-emerald-950/30 dark:text-emerald-300">
              {channelProviderKey}
            </span>
            <span className="rounded bg-gray-100 px-1.5 py-0.5 text-[10px] font-medium text-gray-500 dark:bg-dark-800 dark:text-dark-300">
              {side === 'configured' ? '前端展示' : 'URL 探测'}
            </span>
          </div>
          <p className="break-all text-sm font-semibold leading-5 text-gray-900 dark:text-white">{title}</p>
          {subtitle && (
            <p className="mt-1 line-clamp-2 break-all text-xs leading-5 text-gray-500 dark:text-dark-300">{subtitle}</p>
          )}
        </div>
        {side === 'configured' ? (
          onSaveConfiguredModels && (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8 shrink-0 text-gray-400 hover:text-red-600"
              disabled={busy}
              onClick={(event) => {
                event.stopPropagation();
                void handleRemoveConfiguredModel(entry.raw);
              }}
              aria-label={`移除 ${entry.raw.name}`}
            >
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Trash2 className="h-4 w-4" />}
            </Button>
          )
        ) : exists ? (
          <span className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-emerald-100 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300">
            <Check className="h-4 w-4" />
          </span>
        ) : (
          onSaveConfiguredModels && (
            <Button
              type="button"
              variant="outline"
              size="icon"
              className="h-8 w-8 shrink-0"
              disabled={busy}
              onClick={(event) => {
                event.stopPropagation();
                void handleAddProbeModel(entry.raw);
              }}
              aria-label={`加入 ${entry.raw.name}`}
            >
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
            </Button>
          )
        )}
      </div>
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[1120px] max-h-[88vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Cpu className="h-5 w-5 text-primary-500" />
            模型探测 · {provider}
          </DialogTitle>
          <DialogDescription>
            左侧是用户模型页会展示的模型，右侧是当前 URL 实时探测到的模型。
          </DialogDescription>
        </DialogHeader>

        {/* Search bar */}
        <div className="flex gap-2 mt-1">
          <div className="relative flex-1">
            <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-gray-400" />
            <Input
              placeholder="搜索模型名称或描述..."
              className="pl-8"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>
        </div>

        {providerKind === 'openai' && (
          <div className="rounded-lg border border-border bg-muted/20 p-3">
            <div className="space-y-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <label className="text-xs font-semibold text-muted-foreground">
                  模型列表请求地址
                  <span className="ml-2 rounded bg-background px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                    {openAIModelsUrlMode}
                  </span>
                </label>
                <button
                  type="button"
                  className="text-xs font-medium text-primary hover:text-primary/80 disabled:pointer-events-none disabled:opacity-50"
                  disabled={!customOpenAIModelsUrl}
                  onClick={() => setModelsUrlInput(defaultOpenAIModelsUrl)}
                >
                  使用默认
                </button>
              </div>
              <div className="grid gap-2 sm:grid-cols-[1fr_auto_auto]">
                <Input
                  className="font-mono text-xs"
                  value={modelsUrlInput}
                  onChange={(e) => setModelsUrlInput(e.target.value)}
                  placeholder={defaultOpenAIModelsUrl || '请输入模型列表请求地址'}
                />
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-9"
                  disabled={!effectiveOpenAIModelsUrl}
                  onClick={() => void handleCopyEffectiveModelsUrl()}
                >
                  <Copy className="mr-1 h-3.5 w-3.5" />
                  复制
                </Button>
                {onSaveModelsUrl && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-9"
                    disabled={savingModelsUrl}
                    onClick={() => void handleSaveModelsUrl()}
                  >
                    {savingModelsUrl ? <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" /> : null}
                    保存
                  </Button>
                )}
              </div>
              <p className="text-[11px] leading-5 text-muted-foreground">
                默认地址：<code className="break-all font-mono">{defaultOpenAIModelsUrl || '未配置'}</code>
              </p>
            </div>
          </div>
        )}

        <div className="grid flex-1 min-h-0 gap-3 lg:grid-cols-2">
          <section className="flex min-h-[420px] flex-col rounded-lg border border-gray-200 bg-gray-50/50 p-3 dark:border-dark-700 dark:bg-dark-950/30">
            <div className="mb-3 flex items-center justify-between gap-2">
              <div className="min-w-0">
                <h3 className="flex items-center gap-1.5 text-sm font-semibold text-gray-900 dark:text-white">
                  <BookOpen className="h-4 w-4 text-primary-500" />
                  模型页展示
                  <span className="rounded bg-black/5 px-1.5 py-0.5 text-[10px] font-medium tabular-nums text-gray-500 dark:bg-white/10 dark:text-dark-300">
                    {configuredModelItems.length}
                  </span>
                </h3>
                <p className="mt-1 text-xs text-gray-500 dark:text-dark-300">这些模型会写入供应商配置，并出现在用户模型界面。</p>
              </div>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto pr-1">
              {configuredModelItems.length === 0 ? (
                <div className="flex h-full min-h-[320px] items-center justify-center rounded-lg border border-dashed border-gray-200 bg-white text-center text-sm text-gray-400 dark:border-dark-700 dark:bg-dark-900 dark:text-dark-500">
                  从右侧探测结果点击模型加入这里。
                </div>
              ) : filteredConfigured.length === 0 ? (
                renderEmpty(`没有匹配 "${search}" 的模型。`)
              ) : (
                <div className="space-y-2">
                  {filteredConfigured.map((entry) => renderModelRow(entry, 'configured'))}
                </div>
              )}
            </div>
            {configuredModelItems.length > 0 && (
              <div className="mt-2 text-right text-xs tabular-nums text-gray-400 dark:text-dark-500">
                {filteredConfigured.length === configuredModelItems.length
                  ? `共 ${configuredModelItems.length} 个展示模型`
                  : `${filteredConfigured.length} / ${configuredModelItems.length} 个展示模型`}
              </div>
            )}
          </section>

          <section className="flex min-h-[420px] flex-col rounded-lg border border-gray-200 bg-gray-50/50 p-3 dark:border-dark-700 dark:bg-dark-950/30">
            <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
              <div className="min-w-0">
                <h3 className="flex items-center gap-1.5 text-sm font-semibold text-gray-900 dark:text-white">
                  <Radio className="h-4 w-4 text-primary-500" />
                  URL 探测结果
                  {probeModels.length > 0 && (
                    <span className="rounded bg-black/5 px-1.5 py-0.5 text-[10px] font-medium tabular-nums text-gray-500 dark:bg-white/10 dark:text-dark-300">
                      {probeModels.length}
                    </span>
                  )}
                </h3>
                <p className="mt-1 text-xs text-gray-500 dark:text-dark-300">点击右侧模型即可加入左侧展示列表。</p>
              </div>
              <div className="flex items-center gap-2">
                {onSaveConfiguredModels && probeModels.length > 0 && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8 text-xs"
                    disabled={savingModelKey !== '' || probeLoading || unconfiguredProbeModels.length === 0}
                    onClick={() => {
                      void persistConfiguredModels(
                        mergeModelInfoList(draftConfiguredModels, unconfiguredProbeModels),
                        `已加入 ${unconfiguredProbeModels.length} 个模型`
                      );
                    }}
                  >
                    {savingModelKey === '__all__' ? <Loader2 className="mr-1 h-3 w-3 animate-spin" /> : <Plus className="mr-1 h-3 w-3" />}
                    全部加入
                  </Button>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-8 text-xs"
                  disabled={probeLoading || savingModelKey !== ''}
                  onClick={() => void fetchProbeModels()}
                >
                  <RefreshCw className={`mr-1 h-3 w-3 ${probeLoading ? 'animate-spin' : ''}`} />
                  重新探测
                </Button>
              </div>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto pr-1">
              {probeLoading ? renderLoading('正在向上游服务探测当前凭证可用模型...') :
               probeError ? renderError(probeError) :
               probeModels.length === 0 ? renderEmpty('暂无发现任何模型支持或探测超时。') :
               filteredProbe.length === 0 ? renderEmpty(`没有匹配 "${search}" 的模型。`) : (
                <div className="space-y-2">
                  {filteredProbe.map((entry) => renderModelRow(entry, 'probe'))}
                </div>
              )}
            </div>
            {probeModels.length > 0 && !probeLoading && !probeError && (
              <div className="mt-2 text-right text-xs tabular-nums text-gray-400 dark:text-dark-500">
                {filteredProbe.length === probeModels.length
                  ? `共 ${probeModels.length} 个探测模型`
                  : `${filteredProbe.length} / ${probeModels.length} 个探测模型`}
              </div>
            )}
          </section>
        </div>
      </DialogContent>
    </Dialog>
  );
}
