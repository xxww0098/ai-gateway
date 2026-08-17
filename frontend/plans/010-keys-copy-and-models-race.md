# 010 — Fix API key copy timeout and model-list fetch race

- **Status**: TODO
- **Commit**: 34d21b3
- **Severity**: MEDIUM
- **Category**: Bugs & correctness
- **Rule**: Beyond the scan
- **Estimated scope**: 2 files

## Problem

Copying key B within 2s of key A still lets A’s timeout clear B’s “已复制” state.

    // src/features/user-api-keys/hooks.ts:94 — current
    const handleCopy = (id: number, keyStr: string) => {
      navigator.clipboard.writeText(keyStr)
      setCopiedId(id)
      setTimeout(() => setCopiedId(null), 2000)
      toast.success("已复制到剪贴板")
    }

Opening “路由白名单” then switching keys (or closing before the request ends) lets an older `fetchKeyModels` write into the new dialog.

    // src/features/user-api-keys/components/ModelListDialog.tsx:18 — current
    useEffect(() => {
      if (!open) return
      setLoading(true)
      setModels([])
      fetchKeyModels(key_.id)
        .then(res => setModels(res.models || []))
        .catch((err: unknown) => toast.error(errorMessage(err, "无法加载可用模型")))
        .finally(() => setLoading(false))
    }, [open, key_.id])

## Target

    // hooks.ts — target
    const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
    const handleCopy = (id: number, keyStr: string) => {
      navigator.clipboard.writeText(keyStr)
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current)
      setCopiedId(id)
      copyTimerRef.current = setTimeout(() => {
        setCopiedId((current) => (current === id ? null : current))
        copyTimerRef.current = null
      }, 2000)
      toast.success("已复制到剪贴板")
    }

    // ModelListDialog.tsx — target
    useEffect(() => {
      if (!open) return
      let cancelled = false
      setLoading(true)
      setModels([])
      fetchKeyModels(key_.id)
        .then((res) => {
          if (!cancelled) setModels(res.models || [])
        })
        .catch((err: unknown) => {
          if (!cancelled) toast.error(errorMessage(err, "无法加载可用模型"))
        })
        .finally(() => {
          if (!cancelled) setLoading(false)
        })
      return () => {
        cancelled = true
      }
    }, [open, key_.id])

Imitate the cancel flag already used in `src/features/tickets/TicketRichContent.tsx:32`.

## Repo conventions to follow

- Keys hooks own copy state (`src/features/user-api-keys/hooks.ts`).
- Ticket image fetch is the cancel-flag exemplar.

## Steps

1. Store and clear the copy timeout in `hooks.ts`. Clear it on unmount.
2. Add the `cancelled` flag in `ModelListDialog.tsx`.
3. Do not change `fetchKeyModels` or the table layout.

## Boundaries

- Do NOT convert this dialog to react-query in this plan.
- Do NOT add dependencies.
- STOP if both races are already guarded.

## Verification

- **Mechanical**: typecheck. No new diagnostics on these files.
- **Behavior check**: On `/keys`, copy key A then key B within 1s. B stays “已复制” for ~2s. Open 路由白名单 on key A, close immediately, open on key B — the list is B’s models (or loading), not A’s.
- **Done when**: copy highlight tracks the last copied id; model list cannot apply a stale response.
