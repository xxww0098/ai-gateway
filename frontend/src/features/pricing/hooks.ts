// React-query hooks for pricing feature module
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import { errorMessage } from "@/shared/api/errors"
import { toast } from "sonner"
import { fetchModels, updateModelPrice } from "./api"
import { enrichModelCatalogItem, type ModelCatalogItem } from "./model_catalog"
import { loadModelRegistries } from "./model_provider"
import type { UpdatePriceRequest } from "./types"

// ── useModels ───────────────────────────────────────────────────────────────

export interface UseModelsResult {
  models: ModelCatalogItem[]
  rateMultiplier: number
  isLoading: boolean
  refetch: () => void
}

/**
 * Fetches the model catalog with pricing, enriching each item with registry metadata.
 * Loads model registry JSON asynchronously before enrichment (Requirement 6.4).
 */
export function useModels(): UseModelsResult {
  const query = useQuery({
    queryKey: queryKeys.pricing.models(),
    queryFn: async () => {
      // Ensure registries are loaded before fetching/enriching models
      await loadModelRegistries()
      return fetchModels()
    },
    select: (data) => ({
      models: (data?.models || []).map((m) => enrichModelCatalogItem(m)),
      rateMultiplier: data?.rate_multiplier || 1,
    }),
  })

  return {
    models: query.data?.models || [],
    rateMultiplier: query.data?.rateMultiplier || 1,
    isLoading: query.isLoading,
    refetch: () => query.refetch(),
  }
}

// ── useUpdatePrice ──────────────────────────────────────────────────────────

/**
 * Mutation hook for updating a model's pricing (admin only).
 * Invalidates the pricing cache on success.
 */
export function useUpdatePrice() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (payload: UpdatePriceRequest) => updateModelPrice(payload),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.pricing.all() })
    },
    onError: (err) => toast.error(errorMessage(err, "保存定价失败")),
  })
}
