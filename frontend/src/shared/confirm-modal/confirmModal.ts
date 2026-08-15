import { getConfirmModalHandler } from "./registry"
import type { ConfirmModalOptions } from "./types"

/**
 * 全局确认弹窗（需在 React 树中挂载 `<ConfirmModalProvider />`）。
 * 返回 `true` 表示用户点击确认，`false` 表示取消 / 关闭。
 */
export function confirmModal(options: ConfirmModalOptions): Promise<boolean> {
  const run = getConfirmModalHandler()
  if (!run) {
    if (import.meta.env.DEV) {
      console.warn(
        "[confirmModal] ConfirmModalProvider 未挂载，已回退为 window.confirm"
      )
    }
    const msg = options.message
    const ok = window.confirm(
      options.title ? `${options.title}\n\n${msg}` : msg
    )
    return Promise.resolve(ok)
  }
  return run(options)
}
