# 002 — Version the user cache and stop crashing on bad JSON

- **Status**: DONE
- **Commit**: 34d21b3
- **Severity**: HIGH
- **Category**: Bugs & correctness
- **Rule**: react-doctor/client-localstorage-no-version
- **Estimated scope**: 1 file (`auth_store.ts`)

## Problem

Module init parses `cpa_user` with no try/catch. Corrupt or leftover JSON throws before `ErrorBoundary` mounts (`src/main.tsx` imports `App` which imports the store), so the panel is a white screen.

    // src/features/auth/auth_store.ts:18 — current
    export const useAuthStore = create<AuthState>((set, get) => ({
      token: localStorage.getItem('cpa_token'),
      user: JSON.parse(localStorage.getItem('cpa_user') || 'null'),
      setAuth: (token, user) => {
        localStorage.setItem('cpa_token', token)
        localStorage.setItem('cpa_user', JSON.stringify(user))
        set({ token, user })
      },
      updateUser: (userUpdate) => {
        const current = get().user
        if (current) {
          const updated = { ...current, ...userUpdate }
          localStorage.setItem('cpa_user', JSON.stringify(updated))
          set({ user: updated })
        }
      },

Keys are unversioned (`client-localstorage-no-version` at lines 23 and 30). Product rule is hard cutover to `agw-` / AI-GateWay: do not read `cpa_*`.

Canonical fix: suffix the key (`agw_user:v1`) and drop unreadable old data instead of throwing.

## Target

    // src/features/auth/auth_store.ts — target
    const USER_KEY = "agw_user:v1"

    function readCachedUser(): User | null {
      const raw = localStorage.getItem(USER_KEY)
      if (!raw) return null
      try {
        const parsed: unknown = JSON.parse(raw)
        if (!parsed || typeof parsed !== "object") return null
        const row = parsed as Partial<User>
        if (typeof row.id !== "number" || typeof row.email !== "string" || typeof row.role !== "string") {
          return null
        }
        return {
          id: row.id,
          email: row.email,
          role: row.role,
          balance: typeof row.balance === "number" ? row.balance : undefined,
        }
      } catch {
        localStorage.removeItem(USER_KEY)
        return null
      }
    }

    export const useAuthStore = create<AuthState>((set, get) => ({
      token: null, // JWT lives in HttpOnly cookie after plan 001; until then keep token in memory only if 001 is not selected
      user: readCachedUser(),
      setAuth: (token, user) => {
        localStorage.setItem(USER_KEY, JSON.stringify(user))
        localStorage.removeItem("cpa_token")
        localStorage.removeItem("cpa_user")
        set({ token, user })
      },
      updateUser: (userUpdate) => {
        const current = get().user
        if (!current) return
        const updated = { ...current, ...userUpdate }
        localStorage.setItem(USER_KEY, JSON.stringify(updated))
        set({ user: updated })
      },
      logout: () => {
        localStorage.removeItem(USER_KEY)
        localStorage.removeItem("cpa_token")
        localStorage.removeItem("cpa_user")
        set({ token: null, user: null })
      },
    }))

If plan 001 is executed first, do not persist `token` at all. If 001 is deferred, keep `token` in the Zustand store only (memory), not `localStorage`.

## Repo conventions to follow

- Zustand store in `src/features/auth/auth_store.ts` is the single auth source.
- Imitate safe JSON parse already used in `src/shared/api/client.ts:70`.

## Steps

1. Add `readCachedUser` and `USER_KEY = "agw_user:v1"` in `auth_store.ts`.
2. Replace every `cpa_token` / `cpa_user` read/write. Delete those keys on `setAuth` and `logout`.
3. If 001 is not in this batch, stop writing the JWT to `localStorage` (memory only).
4. No other files unless a test imports the old key names.

## Boundaries

- Do NOT migrate `cpa_user` into the new key (hard cutover).
- Do NOT add dependencies.
- STOP if `auth_store.ts` already uses `agw_*` (PR #7); then only add the parse guard + `:v1` suffix.

## Verification

- **Mechanical**: `npx react-doctor@latest --scope changed` clears `client-localstorage-no-version`.
- **Behavior check**: Set `localStorage.agw_user:v1` to `"{not-json"` and reload `/` — the app renders, user is logged out. Happy-path login still survives a refresh (cookie if 001, or in-memory session until next refresh if 001 is deferred).
- **Done when**: boot never throws on bad cache, keys are versioned, `cpa_*` is gone.
