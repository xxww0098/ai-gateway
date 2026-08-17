# 009 — Stop Home from forcing dark mode

- **Status**: TODO
- **Commit**: 34d21b3
- **Severity**: MEDIUM
- **Category**: Bugs & correctness
- **Rule**: Beyond the scan
- **Estimated scope**: 1 file

## Problem

Home runs a mount effect that adds `dark` if the document already has it **or** the OS prefers dark. It never removes `dark`. A user who saved light theme still gets dark on `/` when the OS is dark.

    // src/pages/public/HomePage.tsx:11 — current
    useEffect(() => {
      const isDark = document.documentElement.classList.contains('dark') || window.matchMedia('(prefers-color-scheme: dark)').matches
      if (isDark) {
        document.documentElement.classList.add('dark')
      }
    }, [])

AuthLayout already uses `currentTheme()` / `toggleTheme()` (`src/pages/public/AuthLayout.tsx:7`). Home should not have a second theme policy.

## Target

Delete the `useEffect` and the unused `useEffect` import if nothing else needs it. Do not touch `className` on the page; Tailwind `dark:` variants follow the document class set by the existing boot script / `currentTheme()`.

    // src/pages/public/HomePage.tsx — target
    export default function Home() {
      const token = useAuthStore(s => s.token)
      const isAuthenticated = !!token
      const dashboardPath = '/dashboard'
      return (
        // existing markup unchanged
      )
    }

## Repo conventions to follow

- Theme helpers: `src/shared/theme`.
- AuthLayout is the exemplar for public pages.

## Steps

1. Remove the effect (and `useEffect` import) from `HomePage.tsx`.
2. No other files.

## Boundaries

- Do NOT rewrite the landing layout in this plan.
- Do NOT add a theme toggle on Home unless you are also doing the UX pass.
- STOP if the effect is already gone.

## Verification

- **Mechanical**: file no longer imports `useEffect`.
- **Behavior check**: Set the app to light, set OS to dark, open `/`. The page stays light. `/login` theme toggle still works. Dark users still see the dark landing.
- **Done when**: Home does not write `document.documentElement.classList`.
