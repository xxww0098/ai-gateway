import { useEffect, useState, memo } from "react"
import { fetchApiBlob } from "@/shared/api/client"
import { cn } from "@/shared/utils/utils"

type Seg = { type: "text"; value: string } | { type: "img"; url: string }

function splitTicketContent(s: string): Seg[] {
  const out: Seg[] = []
  let last = 0
  let m: RegExpExecArray | null
  const re = /!\[[^\]]*\]\((\/api\/panel\/user\/ticket-images\/[^)]+)\)/g
  while ((m = re.exec(s)) !== null) {
    if (m.index > last) {
      out.push({ type: "text", value: s.slice(last, m.index) })
    }
    out.push({ type: "img", url: m[1] })
    last = m.index + m[0].length
  }
  if (last < s.length) {
    out.push({ type: "text", value: s.slice(last) })
  }
  if (out.length === 0) {
    out.push({ type: "text", value: s })
  }
  return out
}

const TicketAuthImage = memo(function TicketAuthImage({ url }: { url: string }) {
  const [blobUrl, setBlobUrl] = useState<string | null>(null)
  const [err, setErr] = useState(false)

  useEffect(() => {
    let cancelled = false
    let created: string | null = null
    ;(async () => {
      try {
        const blob = await fetchApiBlob(url)
        if (cancelled) return
        created = URL.createObjectURL(blob)
        setBlobUrl(created)
        setErr(false)
      } catch {
        if (!cancelled) {
          setErr(true)
          setBlobUrl(null)
        }
      }
    })()
    return () => {
      cancelled = true
      if (created) URL.revokeObjectURL(created)
    }
  }, [url])

  if (err) {
    return <span className="text-xs text-amber-600 dark:text-amber-400">[图片加载失败]</span>
  }
  if (!blobUrl) {
    return <span className="text-xs text-gray-400 dark:text-dark-500">[图片加载中…]</span>
  }
  return (
    <a href={blobUrl} target="_blank" rel="noreferrer" className="mt-1 block max-w-full">
      <img src={blobUrl} alt="工单图片" className="max-h-64 max-w-full rounded-md border border-border object-contain" />
    </a>
  )
})

export const TicketRichContent = memo(function TicketRichContent({
  content,
  className,
}: {
  content: string
  className?: string
}) {
  const segments = splitTicketContent(content)
  return (
    <div className={cn("whitespace-pre-wrap break-words leading-relaxed", className)}>
      {segments.map((seg, i) =>
        seg.type === "text" ? (
          <span key={i}>{seg.value}</span>
        ) : (
          <TicketAuthImage key={i} url={seg.url} />
        )
      )}
    </div>
  )
})
