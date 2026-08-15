/**
 * Auth feature module API functions.
 * Uses the typed apiClient for login, register, and profile endpoints.
 */

import { apiClient } from '@/shared/api/client'
import type { LoginRequest, RegisterRequest, AuthResponse, ProfileResponse } from './types'

/** POST /auth/login — authenticate user and receive JWT + user data */
export function login(data: LoginRequest): Promise<AuthResponse> {
  return apiClient.post<AuthResponse>('/auth/login', data)
}

/** POST /auth/register — create account and receive JWT + user data */
export function register(data: RegisterRequest): Promise<AuthResponse> {
  return apiClient.post<AuthResponse>('/auth/register', data)
}

/** GET /user/profile — fetch current user profile (requires auth) */
export function getProfile(): Promise<ProfileResponse> {
  return apiClient.get<ProfileResponse>('/user/profile')
}

/**
 * POST /auth/logout — server-side "log out everywhere": bumps the user's token
 * version so every outstanding JWT (this device and any other) is rejected on
 * its next request. Call before clearing local state on an explicit logout.
 */
export function logout(): Promise<{ ok: boolean }> {
  return apiClient.post<{ ok: boolean }>('/auth/logout', {})
}
