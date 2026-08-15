import { useRef, useState } from "react"
import { ImagePlus, Loader2 } from "lucide-react"
import { Button } from "@/shared/components/ui/button"
import { fetchApiFormData } from "@/shared/api/client"
import { toast } from "sonner"

const maxBytes = 4 * 1024 * 1024

export type TicketImageUploadButtonProps = {
  onInsert: (markdown: string) => void
  disabled?: boolean
  /** 小尺寸仅图标，用于窄栏 */
  compact?: boolean
}

export function TicketImageUploadButton({ onInsert, disabled, compact }: TicketImageUploadButtonProps) {
  const inputRef = useRef<HTMLInputElement>(null)
  const [busy, setBusy] = useState(false)

  const pick = () => inputRef.current?.click()

  const onChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    e.target.value = ""
    if (!file) return
    if (!file.type.startsWith("image/")) {
      toast.error("请选择图片文件")
      return
    }
    if (file.size > maxBytes) {
      toast.error("图片不能超过 4MB")
      return
    }
    const fd = new FormData()
    fd.append("image", file)
    setBusy(true)
    try {
      const res = (await fetchApiFormData("/user/ticket-images", fd)) as {
        data?: { markdown?: string }
      }
      const data = res?.data
      const md = typeof data?.markdown === "string" ? data.markdown : ""
      if (!md) throw new Error("上传失败")
      onInsert(md)
      toast.success("图片已插入")
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "上传失败")
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <input
        ref={inputRef}
        type="file"
        accept="image/jpeg,image/png,image/gif,image/webp"
        className="sr-only"
        aria-hidden
        tabIndex={-1}
        onChange={(ev) => void onChange(ev)}
      />
      <Button
        type="button"
        variant="outline"
        size={compact ? "icon" : "sm"}
        onClick={pick}
        disabled={disabled || busy}
        title="上传图片（JPEG / PNG / GIF / WebP，最大 4MB）"
        className={compact ? "h-9 w-9 shrink-0" : "gap-1.5"}
      >
        {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <ImagePlus className="h-4 w-4" />}
        {!compact && <span>图片</span>}
      </Button>
    </>
  )
}
