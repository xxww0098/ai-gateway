# 006 — Label the create-key dialog and always clear its loading flag

- **Status**: DONE
- **Commit**: 34d21b3
- **Severity**: MEDIUM
- **Category**: Accessibility
- **Rule**: react-doctor/label-has-associated-control, react-doctor/no-placeholder-only-field
- **Estimated scope**: 1 file

## Problem

Creating an API key is the core user action. Labels are not associated with inputs (scan hotspot: 17 diagnostics on this file). `handleSubmit` sets `creating` true and only clears it after `await onCreate(...)`. If `onCreate` throws, the button stays disabled.

    // src/features/user-api-keys/components/CreateApiKeyDialog.tsx:59 — current
    const handleSubmit = async (e: { preventDefault: () => void }) => {
      e.preventDefault()
      setCreating(true)
      const form: CreateKeyForm = { name, quota, rate_5h: rate5h, rate_1d: rate1d, rate_7d: rate7d, rate_30d: rate30d }
      ...
      await onCreate(form, () => {
        setOpen(false)
        resetForm()
      })
      setCreating(false)
    }

    // src/features/user-api-keys/components/CreateApiKeyDialog.tsx:105 — current
    <label className="input-label">名称</label>
    <input className="input" placeholder="例如：本地开发测试环境" value={name} ... />

Same pattern for quota (`:119`), group select (`:133`), and the rate / expiration fields below.

## Target

    // handleSubmit — target
    const handleSubmit = async (e: { preventDefault: () => void }) => {
      e.preventDefault()
      setCreating(true)
      try {
        const form: CreateKeyForm = {
          name,
          quota,
          rate_5h: rate5h,
          rate_1d: rate1d,
          rate_7d: rate7d,
          rate_30d: rate30d,
        }
        if (selectedGroupId) {
          form.group_id = parseInt(selectedGroupId, 10)
        }
        const expiresInDays = getExpiresInDays()
        if (expiresInDays !== undefined) {
          form.expires_in_days = expiresInDays
        }
        await onCreate(form, () => {
          setOpen(false)
          resetForm()
        })
      } finally {
        setCreating(false)
      }
    }

    // name field — target
    <label htmlFor="create-key-name" className="input-label">名称</label>
    <input id="create-key-name" className="input" placeholder="例如：本地开发测试环境" value={name} onChange={(e) => setName(e.target.value)} maxLength={50} required />

    <label htmlFor="create-key-quota" className="input-label">总额度上限 (可选)</label>
    <input id="create-key-quota" type="number" className="input" ... />

    <label htmlFor="create-key-group" className="input-label">绑定分组 (可选)</label>
    <select id="create-key-group" className="input appearance-none pr-8 cursor-pointer" ...>

Give every remaining labeled control in this dialog a matching `id` (`create-key-rate-5h`, `create-key-rate-1d`, `create-key-rate-7d`, `create-key-rate-30d`, `create-key-custom-days`). Icon-only close / chevron stays decorative (Radix Title already names the dialog).

## Repo conventions to follow

- Dialog uses Radix primitives like other feature dialogs.
- Keep `input-label` / `input` classes.

## Steps

1. Wrap `onCreate` in `try/finally` and always `setCreating(false)`.
2. Add `htmlFor`/`id` pairs for every labeled field in this file.
3. Do not change `CreateKeyForm` shape or `onCreate` signature.

## Boundaries

- Do NOT extract a form library here.
- Do NOT restyle the dialog.
- STOP if the file already associates labels and uses `finally`.

## Verification

- **Mechanical**: `npx react-doctor@latest --scope changed` clears label diagnostics on this file.
- **Behavior check**: Open `/keys` → 新建 Key. Click each label; the matching field focuses. Submit a valid key (happy path). Force `onCreate` to reject (network off) and confirm the submit button enables again.
- **Done when**: labels are associated, creating cannot stick true, key creation still works.
