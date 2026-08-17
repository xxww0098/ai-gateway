# 007 — Catch Header/Sidebar crashes with a layout ErrorBoundary

- **Status**: TODO
- **Commit**: 34d21b3
- **Severity**: MEDIUM
- **Category**: Bugs & correctness
- **Rule**: Beyond the scan
- **Estimated scope**: 1 file (`App.tsx`)

## Problem

`ErrorBoundary` exists and wraps each page element (`src/App.tsx:50`). `UserLayout` and `AuthLayout` are outside those wrappers. A throw in `Header` or `Sidebar` still whitescreens the chrome, including navigation.

    // src/App.tsx:61 — current
    <Route element={<AuthLayout />}>
      <Route path="/login" element={eb(<Login />)} />
    </Route>
    <Route element={<UserLayout />}>
      <Route path="/dashboard" element={eb(<Dashboard />)} />

`src/main.tsx` has no root boundary either. `auth_store` init can still throw before React (plan 002). This plan is only the layout hole.

## Target

    // src/App.tsx — target
    <Route element={eb(<AuthLayout />)}>
      <Route path="/login" element={eb(<Login />)} />
      <Route path="/register" element={eb(<Register />)} />
    </Route>

    <Route element={eb(<UserLayout />)}>
      <Route path="/dashboard" element={eb(<Dashboard />)} />
      {/* keep per-page boundaries so a page throw does not replace the shell */}
    </Route>

Do not remove per-page `eb(...)`. Nested boundaries keep a page failure inside the outlet.

## Repo conventions to follow

- Reuse `eb()` and `ErrorBoundary` from `src/shared/components/ErrorBoundary.tsx`.
- Fallback copy already exists in that component.

## Steps

1. Wrap `AuthLayout` and `UserLayout` route elements with `eb(...)`.
2. Leave page-level `eb(...)` in place.
3. No new component.

## Boundaries

- Do NOT redesign the fallback UI.
- Do NOT add logging services.
- STOP if layouts are already wrapped.

## Verification

- **Mechanical**: existing `ErrorBoundary.test.tsx` still passes.
- **Behavior check**: Temporarily throw in `Header` in a local-only experiment, confirm the layout fallback (not a blank tab), then revert the throw. A throw inside Dashboard still shows the page fallback with sidebar visible.
- **Done when**: layout vs page failures are isolated as above.
