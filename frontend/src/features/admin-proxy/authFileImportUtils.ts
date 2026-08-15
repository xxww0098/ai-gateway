/** 凭证文件合并 / 去重（SDK multipart 要求每个 part 的文件名为 .json） */

function stableStringify(value: unknown): string {
  if (value === null || typeof value !== 'object') {
    return JSON.stringify(value)
  }
  if (Array.isArray(value)) {
    return `[${value.map(stableStringify).join(',')}]`
  }
  const obj = value as Record<string, unknown>
  const keys = Object.keys(obj).sort()
  return `{${keys.map(k => `${JSON.stringify(k)}:${stableStringify(obj[k])}`).join(',')}}`
}

/** 用于判断两段文本是否为「同一凭证」：尽量 JSON 语义去重，否则按规范化原文 */
function authFileContentDedupeKey(text: string): string {
  const t = text.trim().replace(/\r\n/g, '\n').replace(/\r/g, '\n')
  try {
    return stableStringify(JSON.parse(t))
  } catch {
    return t
  }
}

export interface DedupeAuthFilesResult {
  files: File[]
  removedDuplicates: number
}

/** 按内容去重，保留首次出现的文件 */
export async function dedupeAuthFiles(files: File[]): Promise<DedupeAuthFilesResult> {
  const seen = new Set<string>()
  const out: File[] = []
  let removedDuplicates = 0

  for (const f of files) {
    const text = await f.text()
    const key = authFileContentDedupeKey(text)
    if (seen.has(key)) {
      removedDuplicates++
      continue
    }
    seen.add(key)
    out.push(new File([text], f.name, { type: f.type || 'application/octet-stream' }))
  }

  return { files: out, removedDuplicates }
}

/** 保证文件名以 .json 结尾且在批次内唯一（避免 SDK 按文件名覆盖） */
function assignUniqueJsonFilenames(files: File[]): File[] {
  const used = new Set<string>()

  return files.map((f, i) => {
    let raw = f.name.trim() || `import-${i + 1}.json`
    if (!/\.json$/i.test(raw)) {
      const dot = raw.lastIndexOf('.')
      const stem = dot > 0 ? raw.slice(0, dot) : raw
      raw = `${stem || 'import'}.json`
    }

    let candidate = raw
    let n = 2
    while (used.has(candidate.toLowerCase())) {
      const base = raw.replace(/\.json$/i, '')
      candidate = `${base}-${n}.json`
      n++
    }
    used.add(candidate.toLowerCase())

    return new File([f], candidate, { type: f.type || 'application/json' })
  })
}

/** 构建 SDK 所需的 multipart：字段名均为 `file` */
export function buildAuthFilesFormData(files: File[]): FormData {
  const form = new FormData()
  const named = assignUniqueJsonFilenames(files)
  for (const f of named) {
    form.append('file', f, f.name)
  }
  return form
}
