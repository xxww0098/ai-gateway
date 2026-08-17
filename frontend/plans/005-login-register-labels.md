# 005 — Associate login/register fields and name the password toggle

- **Status**: DONE
- **Commit**: 34d21b3
- **Severity**: MEDIUM
- **Category**: Accessibility
- **Rule**: react-doctor/label-has-associated-control, react-doctor/control-has-associated-label, react-doctor/no-placeholder-only-field
- **Estimated scope**: 2 files

## Problem

Login and register are every-session forms. Visible `<label>` text is not tied to the input (`htmlFor`/`id` missing). The show-password control is icon-only with no accessible name.

    // src/pages/public/LoginPage.tsx:46 — current
    <div className="space-y-1">
      <label className="input-label">邮箱</label>
      <div className="relative">
        ...
        <input
          type="email"
          className="input pl-10"
          placeholder="admin@example.com"
          value={email}
          onChange={e => setEmail(e.target.value)}
          required
        />

    // src/pages/public/LoginPage.tsx:77 — current
    <button
      type="button"
      onClick={() => setShowPassword(!showPassword)}
      className="absolute inset-y-0 right-0 pr-3 flex items-center text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 transition-colors"
    >
      {showPassword ? <EyeOff className="h-5 w-5" /> : <Eye className="h-5 w-5" />}
    </button>

Register mirrors this at `RegisterPage.tsx:50,55,67,81`.

Canonical fix: persistent name via associated `<label>`; `aria-label` for the icon-only toggle. A visible label plus `htmlFor` also clears `no-placeholder-only-field`.

## Target

    // LoginPage.tsx — target (email + password + toggle)
    <label htmlFor="login-email" className="input-label">邮箱</label>
    <input
      id="login-email"
      type="email"
      autoComplete="email"
      className="input pl-10"
      placeholder="admin@example.com"
      value={email}
      onChange={e => setEmail(e.target.value)}
      required
    />

    <label htmlFor="login-password" className="input-label">密码</label>
    <input
      id="login-password"
      type={showPassword ? "text" : "password"}
      autoComplete="current-password"
      className="input pl-10 pr-10"
      placeholder="••••••••"
      value={password}
      onChange={e => setPassword(e.target.value)}
      required
    />
    <button
      type="button"
      aria-label={showPassword ? "隐藏密码" : "显示密码"}
      onClick={() => setShowPassword(!showPassword)}
      className="absolute inset-y-0 right-0 pr-3 flex items-center text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 transition-colors"
    >

Register: `register-email`, `register-password`, `autoComplete="new-password"`. Same `aria-label` on the toggle.

## Repo conventions to follow

- AuthLayout theme toggle already uses `aria-label` (`src/pages/public/AuthLayout.tsx:13`).
- Keep `input-label` / `input` / `btn` classes.

## Steps

1. Wire `htmlFor`/`id` and `autoComplete` on Login.
2. Add `aria-label` on the Login password toggle.
3. Repeat on Register.
4. Do not change submit logic.

## Boundaries

- Do NOT restyle the form.
- Do NOT add a component library.
- STOP if these fields already have matching `htmlFor`/`id`.

## Verification

- **Mechanical**: `npx react-doctor@latest --scope changed` clears the three rules on these two files.
- **Behavior check**: Tab through `/login` and `/register`. Labels click-focus the inputs. Screen reader / accessibility tree names the email, password, and toggle. Submit still works.
- **Done when**: diagnostics gone, keyboard and click-label behavior unchanged except the association.
