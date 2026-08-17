# 001 — Store panel session in an HttpOnly cookie

- **Status**: TODO
- **Commit**: 34d21b3
- **Severity**: HIGH
- **Category**: Security
- **Rule**: react-doctor/auth-token-in-web-storage
- **Estimated scope**: 4 files (frontend auth store + API client, panel login/register, bearer extractor)

## Problem

Every signed-in request reads a JWT from `localStorage`. Any XSS on the panel origin can steal it and call `/api/panel` as that user.

    // src/features/auth/auth_store.ts:18 — current
    export const useAuthStore = create<AuthState>((set, get) => ({
      token: localStorage.getItem('cpa_token'),
      user: JSON.parse(localStorage.getItem('cpa_user') || 'null'),
      setAuth: (token, user) => {
        localStorage.setItem('cpa_token', token)
        localStorage.setItem('cpa_user', JSON.stringify(user))
        set({ token, user })
      },

    // src/shared/api/client.ts:60 — current
    const token = useAuthStore.getState().token
    if (token) {
      headers.set('Authorization', `Bearer ${token}`)
    }

Canonical rule: do not persist JWTs in `localStorage`/`sessionStorage`. Use an `HttpOnly` cookie set by the server.

Login today returns `{ token, user }` in JSON (`crates/gw-panel/src/identity/auth.rs:357`) and `bearer_token` only reads `Authorization` (`crates/gw-panel/src/lib.rs:225`). No cookie path exists.

## Target

    // crates/gw-panel/src/lib.rs — bearer_token
    pub fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
        if let Some(raw) = headers.get(axum::http::header::AUTHORIZATION) {
            let raw = raw.to_str().ok()?;
            let token = raw
                .strip_prefix("Bearer ")
                .or_else(|| raw.strip_prefix("bearer "))?;
            let token = token.trim();
            if !token.is_empty() {
                return Some(token);
            }
        }
        // Cookie name is the only client-visible session key. Not readable from JS.
        let cookie = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix("agw_session=") {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
        None
    }

On login and register success, set:

    Set-Cookie: agw_session=<jwt>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=<jwt expiry seconds>

On logout, clear that cookie (`Max-Age=0`). Stop putting `token` in the JSON body (hard cutover, no `cpa-`/`Authorization` fallback after this lands).

    // src/shared/api/client.ts — target
    const response = await fetch(buildUrl(basePrefix, endpoint), {
      ...options,
      headers,
      credentials: "include",
    })
    // do not set Authorization from localStorage

    // src/features/auth/auth_store.ts — target
    // keep user profile cache only; never store the JWT
    setAuth: (user) => { /* persist user record only, see plan 002 */ }

## Repo conventions to follow

- Panel routes stay under `/api/panel`. Do not change `/v1` proxy contracts.
- Imitate `crates/gw-panel/src/identity/auth.rs` login/register response helpers (`ok(...)`).
- Frontend client already centralizes fetch in `src/shared/api/client.ts:56`.

## Steps

1. Extend `bearer_token` in `crates/gw-panel/src/lib.rs:225` to read `agw_session` when `Authorization` is absent.
2. On login/register success in `crates/gw-panel/src/identity/auth.rs`, append the HttpOnly cookie and drop `token` from the JSON body. Keep `user`.
3. Add a logout handler (or extend the existing one) that expires `agw_session`.
4. In `src/shared/api/client.ts`, send `credentials: "include"` and stop attaching `Authorization`.
5. In `src/features/auth/auth_store.ts` and `src/features/auth/hooks.ts`, stop persisting or forwarding `res.token`. Treat "has session" as `user !== null` after a profile fetch, or a dedicated `/auth/session` ping.
6. Re-read the diff and remove any `cpa_token` leftover.

## Boundaries

- Do NOT change `/v1` JSON or Hold/Settle/Release.
- Do NOT add dependencies.
- Do NOT keep writing the JWT to `localStorage` as a fallback.
- STOP if `34d21b3` has drifted (especially if PR #7 already renamed keys); rebase and re-read `auth_store.ts` before editing.

## Verification

- **Mechanical**:
  - `npx react-doctor@latest --scope changed` clears `auth-token-in-web-storage`.
  - Panel + frontend typecheck/tests for auth.
- **Behavior check**: Log in at `/login`, confirm DevTools Application has no JWT in Local Storage, Application cookies shows `agw_session` HttpOnly, and `/dashboard` still loads. Log out and confirm the cookie is gone. A hard refresh still stays signed in.
- **Done when**: diagnostic is clear, score is not lower, login/logout/refresh work, JWT is not readable from JS.
