import { useState, useRef, useEffect } from "react"
import { createPortal } from "react-dom"
import { ChevronDown, Link2, Unlink, Loader2 } from "lucide-react"
import type { AvailableGroup } from "../types"

interface Props {
  currentGroupId: number | null | undefined
  currentGroupName?: string
  groups: AvailableGroup[]
  loading: boolean
  onRebind: (groupId: number | null) => void
  rebinding: boolean
}

export function GroupRebindDropdown({
  currentGroupId,
  currentGroupName,
  groups,
  loading,
  onRebind,
  rebinding,
}: Props) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [menuPos, setMenuPos] = useState({ top: 0, left: 0, width: 200 })
  const resolvedGroupName =
    currentGroupName?.trim() ||
    groups.find((g) => g.id === currentGroupId)?.name ||
    "未绑定"

  useEffect(() => {
    if (!open) return

    const updatePosition = () => {
      if (!buttonRef.current) return
      const rect = buttonRef.current.getBoundingClientRect()
      setMenuPos({
        top: rect.bottom + 4,
        left: rect.left,
        width: Math.max(200, rect.width),
      })
    }

    updatePosition()
    window.addEventListener("resize", updatePosition)
    window.addEventListener("scroll", updatePosition, true)
    return () => {
      window.removeEventListener("resize", updatePosition)
      window.removeEventListener("scroll", updatePosition, true)
    }
  }, [open])

  useEffect(() => {
    function handleClick(e: MouseEvent) {
      const target = e.target as Node
      const clickedInsideTrigger = !!ref.current?.contains(target)
      const clickedInsideMenu = !!menuRef.current?.contains(target)
      if (!clickedInsideTrigger && !clickedInsideMenu) {
        setOpen(false)
      }
    }
    document.addEventListener("mousedown", handleClick)
    return () => document.removeEventListener("mousedown", handleClick)
  }, [])

  return (
    <div className="relative" ref={ref}>
      <button
        ref={buttonRef}
        type="button"
        className="inline-flex items-center gap-1.5 rounded-lg border border-primary-200 dark:border-primary-800 bg-primary-50 dark:bg-primary-900/10 px-2.5 py-1 text-xs font-medium text-primary-600 dark:text-primary-400 hover:bg-primary-100 dark:hover:bg-primary-900/30 transition-colors disabled:opacity-50"
        onClick={() => setOpen(!open)}
        disabled={rebinding}
      >
        {rebinding ? (
          <Loader2 className="h-3 w-3 animate-spin" />
        ) : (
          <Link2 className="h-3 w-3" />
        )}
        {resolvedGroupName}
        <ChevronDown className="h-3 w-3" />
      </button>

      {open &&
        createPortal(
          <div
            ref={menuRef}
            style={{
              position: "fixed",
              top: menuPos.top,
              left: menuPos.left,
              minWidth: menuPos.width,
            }}
            className="z-[1000] rounded-xl border border-border bg-white dark:bg-dark-900 shadow-lg py-1 animate-in fade-in-0 zoom-in-95"
          >
            {loading ? (
              <div className="px-3 py-2 text-xs text-gray-400 flex items-center gap-2">
                <Loader2 className="h-3 w-3 animate-spin" />
                加载分组中...
              </div>
            ) : (
              <>
                {groups.map((g) => (
                  <button
                    key={g.id}
                    type="button"
                    className={`w-full text-left px-3 py-2 text-xs hover:bg-gray-50 dark:hover:bg-dark-800 transition-colors flex items-center justify-between ${
                      g.id === currentGroupId
                        ? "text-primary-600 dark:text-primary-400 font-semibold"
                        : "text-gray-700 dark:text-gray-300"
                    }`}
                    onClick={() => {
                      onRebind(g.id)
                      setOpen(false)
                    }}
                  >
                    <span>{g.name}</span>
                    {g.id === currentGroupId && (
                      <span className="text-[10px] text-primary-500">当前</span>
                    )}
                  </button>
                ))}
                {currentGroupId && (
                  <>
                    <div className="border-t border-border my-1" />
                    <button
                      type="button"
                      className="w-full text-left px-3 py-2 text-xs text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/20 transition-colors flex items-center gap-2"
                      onClick={() => {
                        onRebind(null)
                        setOpen(false)
                      }}
                    >
                      <Unlink className="h-3 w-3" />
                      解绑分组
                    </button>
                  </>
                )}
              </>
            )}
          </div>,
          document.body,
        )}
    </div>
  )
}
