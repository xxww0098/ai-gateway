// Single source of truth for the light/dark theme.
//
// The actual switch is a `dark` class on <html> (Tailwind darkMode: 'class');
// the preference is persisted in localStorage. An inline boot script in
// index.html applies the stored (or system) theme BEFORE first paint to avoid a
// flash-of-wrong-theme and to survive refreshes — these helpers keep runtime
// toggles consistent with that boot logic.
export type Theme = 'light' | 'dark'

const STORAGE_KEY = 'theme'

/** Reads the live theme from the <html> class the boot script already applied. */
export function currentTheme(): Theme {
  return document.documentElement.classList.contains('dark') ? 'dark' : 'light'
}

/** Applies a theme to <html> and persists it. */
export function applyTheme(theme: Theme): void {
  document.documentElement.classList.toggle('dark', theme === 'dark')
  try {
    localStorage.setItem(STORAGE_KEY, theme)
  } catch {
    /* storage unavailable (private mode/quota) — the class still applies */
  }
}

/** Flips the current theme, persists it, and returns the new value. */
export function toggleTheme(): Theme {
  const next: Theme = currentTheme() === 'dark' ? 'light' : 'dark'
  applyTheme(next)
  return next
}
