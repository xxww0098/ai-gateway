import type { AuthFileItem } from './types'

const providerColors: Record<string, string> = {
  anthropic: 'bg-orange-100 text-orange-700 dark:bg-orange-950/40 dark:text-orange-300',
  openai: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300',
  google: 'bg-blue-100 text-blue-700 dark:bg-blue-950/40 dark:text-blue-300',
  codex: 'bg-violet-100 text-violet-700 dark:bg-violet-950/40 dark:text-violet-300',
  gemini: 'bg-sky-100 text-sky-700 dark:bg-sky-950/40 dark:text-sky-300',
  claude: 'bg-amber-100 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300',
  antigravity: 'bg-purple-100 text-purple-700 dark:bg-purple-950/40 dark:text-purple-300',
}

export interface AuthFileIdentity {
  primary: string
  primaryTitle: string
  email?: string
  sourceKind?: string
  hasStatusMessage: boolean
}

export function providerBadgeColor(provider?: string) {
  return providerColors[provider?.toLowerCase() || ''] || 'bg-slate-100 text-slate-600 dark:bg-dark-700 dark:text-slate-300'
}

function cleanText(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function sameText(a?: string, b?: string): boolean {
  const left = cleanText(a).toLowerCase()
  const right = cleanText(b).toLowerCase()
  return left !== '' && left === right
}

export function resolveAuthFileIdentity(file: AuthFileItem): AuthFileIdentity {
  const label = cleanText(file.label)
  const email = cleanText(file.email)
  const name = cleanText(file.name)
  const primary = label || email || name || '未命名凭证'
  const sourceKind = cleanText(file.source_kind) || cleanText(file.auth_type)

  return {
    primary,
    primaryTitle: name || primary,
    email: email && !sameText(email, primary) ? email : undefined,
    sourceKind,
    hasStatusMessage: cleanText(file.status_message) !== '',
  }
}

function getSelectedAuthFiles(files: AuthFileItem[], selectedNames: Set<string>): AuthFileItem[] {
  if (selectedNames.size === 0) return []
  return files.filter(file => selectedNames.has(file.name))
}

export function getBatchStatusTargets(files: AuthFileItem[], selectedNames: Set<string>) {
  const selected = getSelectedAuthFiles(files, selectedNames)
  return {
    selected,
    pauseTargets: selected.filter(file => !file.disabled),
    resumeTargets: selected.filter(file => !!file.disabled),
  }
}
