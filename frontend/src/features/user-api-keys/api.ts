// API functions for user API keys management
import { apiClient } from "@/shared/api/client"
import type { ApiKey, AvailableGroup, CreateKeyPayload, ModelInfo } from "./types"

interface ApiKeyListResponse {
  items: ApiKey[]
}

interface ModelsResponse {
  models: ModelInfo[]
}

// ── API Keys ────────────────────────────────────────────────────────────────

export function fetchApiKeys() {
  return apiClient.get<ApiKeyListResponse>("/user/api-keys")
}

export function createApiKey(payload: CreateKeyPayload) {
  return apiClient.post<ApiKey>("/user/api-keys", payload)
}

export function deleteApiKey(id: number) {
  return apiClient.delete(`/user/api-keys/${id}`)
}

export function rebindApiKeyGroup(keyId: number, groupId: number | null) {
  return apiClient.patch(`/user/api-keys/${keyId}/group`, { group_id: groupId })
}

// ── Groups ──────────────────────────────────────────────────────────────────

export function fetchAvailableGroups() {
  return apiClient.get<AvailableGroup[]>("/user/available-groups")
}

// ── Models ──────────────────────────────────────────────────────────────────

export function fetchKeyModels(keyId: number) {
  return apiClient.get<ModelsResponse>(`/user/models?key_id=${keyId}`)
}
