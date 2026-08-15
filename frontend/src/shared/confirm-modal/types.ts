export interface ConfirmModalOptions {
  /** 标题，默认：确认操作 */
  title?: string
  /** 正文 */
  message: string
  confirmText?: string
  cancelText?: string
  /** 视觉语义，危险操作请使用 danger */
  variant?: "default" | "danger" | "warning"
  /** 点击遮罩或按 Esc 是否视为取消，默认 true */
  dismissViaOverlayOrEscape?: boolean
}

export type ConfirmModalHandler = (
  options: ConfirmModalOptions
) => Promise<boolean>
