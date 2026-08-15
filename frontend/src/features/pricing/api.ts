// API functions for pricing feature module
import { apiClient } from "@/shared/api/client"
import type { UpdatePriceRequest, ModelsResponse } from "./types"

/**
 * Fetch all available models with pricing info for the current user.
 * GET /user/models
 */
export function fetchModels() {
  return apiClient.get<ModelsResponse>("/user/models")
}

/**
 * Update pricing for a specific model (admin only).
 * POST /admin/pricing/models
 */
export function updateModelPrice(payload: UpdatePriceRequest) {
  return apiClient.post("/admin/pricing/models", payload)
}
