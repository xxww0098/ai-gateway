import { apiCallRequest, fetchMgmtApi } from '@/features/admin-proxy/api';

export interface ModelInfo {
  name: string;
  alias?: string;
  description?: string;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);

export function normalizeModelList(payload: unknown, { dedupe = false } = {}): ModelInfo[] {
  const toModel = (entry: unknown): ModelInfo | null => {
    if (typeof entry === 'string') {
      return { name: entry };
    }
    if (!isRecord(entry)) {
      return null;
    }
    const name = entry.id || entry.name || entry.model || entry.value;
    if (!name) return null;

    const alias = entry.alias || entry.display_name || entry.displayName;
    const description = entry.description || entry.note || entry.comment;
    const model: ModelInfo = { name: String(name) };
    if (alias && alias !== name) {
      model.alias = String(alias);
    }
    if (description) {
      model.description = String(description);
    }
    return model;
  };

  let models: (ModelInfo | null)[] = [];

  if (Array.isArray(payload)) {
    models = payload.map(toModel);
  } else if (isRecord(payload)) {
    if (Array.isArray(payload.data)) {
      models = payload.data.map(toModel);
    } else if (Array.isArray(payload.models)) {
      models = payload.models.map(toModel);
    }
  }

  const normalized = models.filter(Boolean) as ModelInfo[];
  if (!dedupe) {
    return normalized;
  }

  const seen = new Set<string>();
  return normalized.filter((model) => {
    const key = (model?.name || '').toLowerCase();
    if (!key || seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

// ------------------------------------
// Endpoint builders
// ------------------------------------

function normalizeApiBase(baseUrl: string): string {
  let trimmed = String(baseUrl ?? '').trim();
  // Automatically strip `/v1/chat/completions` or `/chat/completions` which users often copy mistakenly
  trimmed = trimmed.replace(/\/?v1\/chat\/completions\/?$/i, '');
  trimmed = trimmed.replace(/\/?chat\/completions\/?$/i, '');
  if (trimmed.endsWith('/')) {
    trimmed = trimmed.replace(/\/+$/g, '');
  }
  return trimmed;
}

const buildV1ModelsEndpoint = (baseUrl: string): string => {
  const normalized = normalizeApiBase(baseUrl);
  if (!normalized) return '';
  const trimmed = normalized.replace(/\/+$/g, '');
  if (/\/v1\/models$/i.test(trimmed)) return trimmed;
  if (/\/v1$/i.test(trimmed)) return `${trimmed}/models`;
  return `${trimmed}/v1/models`;
};

const buildOpenAIModelsEndpoint = (baseUrl: string, modelsUrl?: string): string => {
  const custom = String(modelsUrl ?? '').trim();
  if (!custom) return buildV1ModelsEndpoint(baseUrl);
  if (/^https?:\/\//i.test(custom)) return custom;

  const normalizedBase = normalizeApiBase(baseUrl);
  if (!normalizedBase) return custom;
  const base = normalizedBase.replace(/\/+$/g, '');
  if (custom.startsWith('/')) {
    try {
      return `${new URL(base).origin}${custom}`;
    } catch {
      return `${base}${custom}`;
    }
  }
  return `${base}/${custom.replace(/^\/+/g, '')}`;
};

const DEFAULT_CLAUDE_BASE_URL = 'https://api.anthropic.com';
const buildClaudeModelsEndpoint = (baseUrl: string): string => {
  const normalized = normalizeApiBase(baseUrl);
  const fallback = normalized || DEFAULT_CLAUDE_BASE_URL;
  let trimmed = fallback.replace(/\/+$/g, '');
  trimmed = trimmed.replace(/\/v1\/models$/i, '');
  trimmed = trimmed.replace(/\/v1(?:\/.*)?$/i, '');
  return `${trimmed}/v1/models`;
};

const DEFAULT_GEMINI_BASE_URL = 'https://generativelanguage.googleapis.com';
const buildGeminiModelsEndpoint = (baseUrl: string): string => {
  const normalized = normalizeApiBase(baseUrl);
  const fallback = normalized || DEFAULT_GEMINI_BASE_URL;
  let trimmed = fallback.replace(/\/+$/g, '');
  trimmed = trimmed.replace(/\/v1beta\/models$/i, '');
  trimmed = trimmed.replace(/\/v1beta(?:\/.*)?$/i, '');
  return `${trimmed}/v1beta/models`;
};

const stripGeminiModelResourceName = (value: string): string => {
  const trimmed = String(value ?? '').trim();
  if (!trimmed) return '';
  return trimmed.replace(/^\/?models\//i, '');
};

const hasHeader = (headers: Record<string, string>, name: string) => {
  const target = name.toLowerCase();
  return Object.keys(headers).some((key) => key.toLowerCase() === target);
};

const resolveBearerTokenFromAuthorization = (headers: Record<string, string>): string => {
  const entry = Object.entries(headers).find(([key]) => key.toLowerCase() === 'authorization');
  if (!entry) return '';
  const value = String(entry[1] ?? '').trim();
  if (!value) return '';
  const match = value.match(/^Bearer\s+(.+)$/i);
  return match?.[1]?.trim() || '';
};

// ------------------------------------
// fetchers
// ------------------------------------

// ------------------------------------
// Provider → SDK channel mapping
// ------------------------------------

const providerToChannelMap: Record<string, string> = {
  claude: 'claude',
  gemini: 'gemini',
  vertex: 'vertex',
  codex: 'codex',
  'gemini-cli': 'gemini-cli',
  aistudio: 'aistudio',
  kimi: 'kimi',
  antigravity: 'antigravity',
};

/**
 * Map a UI provider label (e.g. "Claude", "Gemini", "OpenAI (兼容格式)") to
 * the SDK model-definitions channel slug.
 * Returns empty string when there's no static channel mapping.
 */
export function resolveSDKChannel(provider: string): string {
  const lower = provider.toLowerCase();
  if (lower.includes('openai')) return '';
  for (const [keyword, channel] of Object.entries(providerToChannelMap)) {
    if (lower.includes(keyword)) return channel;
  }
  return '';
}

// ------------------------------------
// SDK management model info (richer than ModelInfo)
// ------------------------------------

export interface SDKModelDefinition {
  id: string;
  object?: string;
  owned_by?: string;
  type?: string;
  display_name?: string;
  name?: string;
  description?: string;
  version?: string;
  inputTokenLimit?: number;
  outputTokenLimit?: number;
  context_length?: number;
  max_completion_tokens?: number;
  supportedInputModalities?: string[];
  supportedOutputModalities?: string[];
  thinking?: {
    min?: number;
    max?: number;
    zero_allowed?: boolean;
    dynamic_allowed?: boolean;
    levels?: string[];
  };
}

export const modelsApi = {
  buildV1ModelsEndpoint,
  buildOpenAIModelsEndpoint,
  buildClaudeModelsEndpoint,
  buildGeminiModelsEndpoint,

  // ── SDK management endpoints ─────────────────────────────────

  /**
   * Fetch static model definitions from SDK's embedded models.json.
   * GET /v0/management/model-definitions/:channel
   */
  async fetchStaticModelDefinitions(channel: string): Promise<SDKModelDefinition[]> {
    if (!channel) return [];
    const data = (await fetchMgmtApi(`/model-definitions/${encodeURIComponent(channel)}`)) as { models?: unknown } | undefined;
    const models = data?.models;
    if (!Array.isArray(models)) return [];
    return models as SDKModelDefinition[];
  },

  /**
   * Fetch models dynamically registered for a specific auth file.
   * GET /v0/management/auth-files/models?name=:name
   */
  async fetchAuthFileModels(authName: string): Promise<SDKModelDefinition[]> {
    if (!authName) return [];
    const data = (await fetchMgmtApi(`/auth-files/models?name=${encodeURIComponent(authName)}`)) as { models?: unknown } | undefined;
    const models = data?.models;
    if (!Array.isArray(models)) return [];
    return models as SDKModelDefinition[];
  },

  /**
   * Convert SDKModelDefinition[] to ModelInfo[] for UI display.
   */
  sdkModelsToModelInfo(models: SDKModelDefinition[]): ModelInfo[] {
    return models
      .filter(m => m && (m.id || m.name))
      .map(m => {
        const info: ModelInfo = { name: m.id || m.name || '' };
        if (m.display_name && m.display_name !== info.name) {
          info.alias = m.display_name;
        } else if (m.name && m.name !== info.name) {
          info.alias = m.name;
        }
        if (m.description) {
          info.description = m.description;
        }
        return info;
      });
  },

  // ── Direct upstream API probing (legacy) ────────────────────

  async fetchOpenAIModelsViaApiCall(
    baseUrl: string,
    apiKey?: string,
    headers: Record<string, string> = {},
    modelsUrl?: string
  ) {
    const endpoint = buildOpenAIModelsEndpoint(baseUrl, modelsUrl);
    const resolvedHeaders = { ...headers };
    const hasAuthHeader = Object.keys(resolvedHeaders).some(k => k.toLowerCase() === 'authorization');
    if (apiKey && !hasAuthHeader) {
      resolvedHeaders.Authorization = `Bearer ${apiKey}`;
    }

    // Inject User-Agent and Accept headers to bypass basic WAF and proxy blocks
    if (!Object.keys(resolvedHeaders).some(k => k.toLowerCase() === 'user-agent')) {
      resolvedHeaders['User-Agent'] = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36';
    }
    if (!Object.keys(resolvedHeaders).some(k => k.toLowerCase() === 'accept')) {
      resolvedHeaders['Accept'] = 'application/json';
    }

    const result = await apiCallRequest({
      method: 'GET',
      url: endpoint,
      header: Object.keys(resolvedHeaders).length ? resolvedHeaders : undefined
    });

    if (result.statusCode < 200 || result.statusCode > 299) {
      const bodyStr = result.bodyText || '';
      const serverHeaders = result.header?.['server'] || result.header?.['Server'] || [];
      const isCloudflare = serverHeaders.some(v => v.toLowerCase().includes('cloudflare')) || bodyStr.includes('Just a moment...');

      let errorMsg = `HTTP ${result.statusCode}: ${bodyStr || '未知错误'}`;
      if (result.statusCode === 403 && isCloudflare) {
        errorMsg = `HTTP 403: 触发了 Cloudflare 等 WAF 人机验证或防爬虫拦截，无法通过代理获取模型 (${endpoint})。`;
      } else if (result.statusCode === 404) {
        errorMsg = `HTTP 404: 接口不存在 (${endpoint})。`;
      }
      throw new Error(errorMsg);
    }

    const payload = result.body ?? result.bodyText;
    let normalized = normalizeModelList(payload, { dedupe: true });
    if (normalized.length === 0 && typeof payload === 'string') {
      try {
        normalized = normalizeModelList(JSON.parse(payload), { dedupe: true });
      } catch { /* ignore */ }
    }

    if (normalized.length === 0) {
      throw new Error(`探测成功但模型列表为空 (${endpoint})`);
    }
    return normalized;
  },

  async fetchClaudeModelsViaApiCall(
    baseUrl: string,
    apiKey?: string,
    headers: Record<string, string> = {}
  ) {
    const endpoint = buildClaudeModelsEndpoint(baseUrl);
    const resolvedHeaders = { ...headers };
    let resolvedApiKey = String(apiKey ?? '').trim();
    if (!resolvedApiKey && !hasHeader(resolvedHeaders, 'x-api-key')) {
      resolvedApiKey = resolveBearerTokenFromAuthorization(resolvedHeaders);
    }
    if (resolvedApiKey && !hasHeader(resolvedHeaders, 'x-api-key')) {
      resolvedHeaders['x-api-key'] = resolvedApiKey;
    }
    if (!hasHeader(resolvedHeaders, 'anthropic-version')) {
      resolvedHeaders['anthropic-version'] = '2023-06-01';
    }
    if (!Object.keys(resolvedHeaders).some(k => k.toLowerCase() === 'user-agent')) {
      resolvedHeaders['User-Agent'] = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36';
    }
    if (!Object.keys(resolvedHeaders).some(k => k.toLowerCase() === 'accept')) {
      resolvedHeaders['Accept'] = 'application/json';
    }

    const result = await apiCallRequest({
      method: 'GET',
      url: endpoint,
      header: Object.keys(resolvedHeaders).length ? resolvedHeaders : undefined
    });

    if (result.statusCode < 200 || result.statusCode > 299) {
      const bodyStr = result.bodyText || '';
      const serverHeaders = result.header?.['server'] || result.header?.['Server'] || [];
      const isCloudflare = serverHeaders.some(v => v.toLowerCase().includes('cloudflare')) || bodyStr.includes('Just a moment...');

      let errorMsg = `HTTP ${result.statusCode}: ${bodyStr || '未知错误'}`;
      if (result.statusCode === 403 && isCloudflare) {
        errorMsg = `HTTP 403: 触发了 Cloudflare 等 WAF 人机验证或防爬虫拦截，无法通过代理获取模型 (${endpoint})。`;
      } else if (result.statusCode === 404) {
        errorMsg = `HTTP 404: 接口不存在 (${endpoint})。`;
      }
      throw new Error(errorMsg);
    }

    const payload = result.body ?? result.bodyText;
    let normalized = normalizeModelList(payload, { dedupe: true });
    if (normalized.length === 0 && typeof payload === 'string') {
      try {
        normalized = normalizeModelList(JSON.parse(payload), { dedupe: true });
      } catch { /* ignore */ }
    }

    if (normalized.length === 0) {
      throw new Error(`探测成功但模型列表为空 (${endpoint})`);
    }
    return normalized;
  },

  async fetchGeminiModelsViaApiCall(
    baseUrl: string,
    apiKey?: string,
    headers: Record<string, string> = {}
  ) {
    const endpoint = buildGeminiModelsEndpoint(baseUrl);
    const resolvedHeaders = { ...headers };
    const resolvedApiKey = String(apiKey ?? '').trim();
    if (resolvedApiKey && !hasHeader(resolvedHeaders, 'x-goog-api-key')) {
      resolvedHeaders['x-goog-api-key'] = resolvedApiKey;
    }
    if (!Object.keys(resolvedHeaders).some(k => k.toLowerCase() === 'user-agent')) {
      resolvedHeaders['User-Agent'] = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36';
    }
    if (!Object.keys(resolvedHeaders).some(k => k.toLowerCase() === 'accept')) {
      resolvedHeaders['Accept'] = 'application/json';
    }

    const urlObj = new URL(endpoint);
    const result = await apiCallRequest({
      method: 'GET',
      url: urlObj.toString(),
      header: Object.keys(resolvedHeaders).length ? resolvedHeaders : undefined
    });

    if (result.statusCode < 200 || result.statusCode >= 300) {
      const bodyStr = result.bodyText || '';
      const serverHeaders = result.header?.['server'] || result.header?.['Server'] || [];
      const isCloudflare = serverHeaders.some(v => v.toLowerCase().includes('cloudflare')) || bodyStr.includes('Just a moment...');

      let errorMsg = `HTTP ${result.statusCode}: ${bodyStr || '未知错误'}`;
      if (result.statusCode === 403 && isCloudflare) {
        errorMsg = `HTTP 403: 触发了 Cloudflare 等 WAF 人机验证或防爬虫拦截，无法通过代理获取模型 (${endpoint})。`;
      } else if (result.statusCode === 404) {
        errorMsg = `HTTP 404: 接口不存在 (${endpoint})。`;
      }
      throw new Error(errorMsg);
    }

    const payload = result.body ?? result.bodyText;
    let normalized = normalizeModelList(payload, { dedupe: false });
    if (normalized.length === 0 && typeof payload === 'string') {
      try {
        normalized = normalizeModelList(JSON.parse(payload), { dedupe: false });
      } catch { /* ignore */ }
    }

    if (normalized.length === 0) {
      throw new Error(`探测成功但模型列表为空 (${endpoint})`);
    }

    const collected: ReturnType<typeof normalizeModelList> = [];
    normalized.forEach((model) => {
      const name = stripGeminiModelResourceName(model.name);
      if (!name) return;
      const resolved = { ...model, name };
      if (resolved.alias && resolved.alias.trim() === name) {
        resolved.alias = undefined;
      }
      collected.push(resolved);
    });
    return collected;
  }
};
