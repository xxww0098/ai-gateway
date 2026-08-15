import type { ConfirmModalHandler } from "./types"

let handler: ConfirmModalHandler | null = null

export function setConfirmModalHandler(next: ConfirmModalHandler | null) {
  handler = next
}

export function getConfirmModalHandler(): ConfirmModalHandler | null {
  return handler
}
