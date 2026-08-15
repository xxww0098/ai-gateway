// Types for pricing feature module

/** Update price request payload for POST /admin/pricing/models */
export interface UpdatePriceRequest {
  model_id: string
  input_price_per_1m: number
  output_price_per_1m: number
  cached_input_price_per_1m: number
  reasoning_price_per_1m: number
}

/** Response shape from GET /user/models */
export interface ModelsResponse {
  models: ModelCatalogRaw[]
  rate_multiplier: number
}

/** Raw model item from the backend before enrichment */
interface ModelCatalogRaw {
  id: string
  object?: string
  owned_by?: string
  type?: string
  name?: string
  display_name?: string
  description?: string
  version?: string
  created?: number
  context_length?: number
  max_completion_tokens?: number
  inputTokenLimit?: number
  outputTokenLimit?: number
  thinking?: {
    min?: number
    max?: number
    zero_allowed?: boolean
    dynamic_allowed?: boolean
    levels?: string[]
  }
  supported_parameters?: string[]
  supportedGenerationMethods?: string[]
  supportedInputModalities?: string[]
  supportedOutputModalities?: string[]
  input_price_per_1m?: number
  output_price_per_1m?: number
  reasoning_price_per_1m?: number
  cached_input_price_per_1m?: number
  base_price_input?: number
  rate_multiplier?: number
}
