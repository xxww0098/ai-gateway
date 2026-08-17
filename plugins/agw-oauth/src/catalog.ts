/** Parse AI-GateWay GET /v1/models into Harness resolveModel facts. */

export const PROVIDER = 'ai-gateway'

export type Modality = 'text' | 'image'

export interface GatewayEffort {
  id: string
  name: string
}

export interface GatewayReasoning {
  efforts: GatewayEffort[]
  defaultEffort?: string
}

export interface GatewayModel {
  id: string
  name: string
  contextLength?: number
  maxOutputTokens?: number
  inputModalities: Modality[]
  reasoning?: GatewayReasoning
}

function asPositiveInt(value: unknown): number | undefined {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return undefined
  return Math.trunc(value)
}

function modalitiesOf(value: unknown): Modality[] {
  if (!Array.isArray(value)) return []
  const out: Modality[] = []
  for (const item of value) {
    if (item === 'text' || item === 'image') out.push(item)
  }
  return out
}

function reasoningOf(value: unknown): GatewayReasoning | undefined {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return undefined
  const raw = value as { efforts?: unknown, default_effort?: unknown }
  if (!Array.isArray(raw.efforts)) return undefined
  const efforts: GatewayEffort[] = []
  for (const item of raw.efforts) {
    if (item === null || typeof item !== 'object') continue
    const id = typeof (item as { id?: unknown }).id === 'string'
      ? (item as { id: string }).id.trim()
      : ''
    if (id.length === 0) continue
    const nameRaw = (item as { name?: unknown }).name
    const name = typeof nameRaw === 'string' && nameRaw.trim().length > 0 ? nameRaw.trim() : id
    efforts.push({ id, name })
  }
  if (efforts.length === 0) return undefined
  const defaultRaw = raw.default_effort
  const defaultEffort = typeof defaultRaw === 'string' && defaultRaw.trim().length > 0
    ? defaultRaw.trim()
    : undefined
  return defaultEffort === undefined ? { efforts } : { efforts, defaultEffort }
}

/** Map one /v1/models row. Unknown extra fields are ignored. */
export function parseGatewayModel(raw: unknown): GatewayModel | undefined {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return undefined
  const row = raw as Record<string, unknown>
  const id = typeof row.id === 'string' ? row.id.trim() : ''
  if (id.length === 0) return undefined
  const name = typeof row.name === 'string' && row.name.trim().length > 0 ? row.name.trim() : id
  const contextLength = asPositiveInt(row.context_length)
  const maxOutputTokens = asPositiveInt(row.max_output_tokens)
  const inputModalities = modalitiesOf(row.input_modalities)
  const reasoning = reasoningOf(row.reasoning)
  return {
    id,
    name,
    ...contextLength === undefined ? {} : { contextLength },
    ...maxOutputTokens === undefined ? {} : { maxOutputTokens },
    inputModalities,
    ...reasoning === undefined ? {} : { reasoning },
  }
}

/** Parse the OpenAI-shaped `{ object, data }` envelope. */
export function parseModelsPayload(json: unknown): GatewayModel[] {
  if (json === null || typeof json !== 'object' || Array.isArray(json)) return []
  const data = (json as { data?: unknown }).data
  if (!Array.isArray(data)) return []
  const models: GatewayModel[] = []
  for (const item of data) {
    const model = parseGatewayModel(item)
    if (model !== undefined) models.push(model)
  }
  return models
}

/** Harness listModels() row. */
export function toListModel(provider: string, model: GatewayModel) {
  return {
    provider,
    id: model.id,
    name: model.name,
    ...model.inputModalities.length === 0 ? {} : { inputModalities: model.inputModalities },
  }
}

/** Harness resolveModel() row. Token limits come from the catalog, never guesses. */
export function toResolvedModel(provider: string, model: GatewayModel) {
  return {
    ...toListModel(provider, model),
    ...model.contextLength === undefined ? {} : { context: { contextWindow: model.contextLength } },
    ...model.maxOutputTokens === undefined ? {} : { defaultMaxTokens: model.maxOutputTokens },
    ...model.reasoning === undefined
      ? {}
      : {
          reasoning: {
            efforts: model.reasoning.efforts,
            ...model.reasoning.defaultEffort === undefined
              ? {}
              : { defaultEffort: model.reasoning.defaultEffort },
          },
        },
  }
}
