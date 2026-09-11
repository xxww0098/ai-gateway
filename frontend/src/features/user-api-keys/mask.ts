/**
 * API Key 展示脱敏：仅保留前缀与末四位之外的遮罩。
 * 独立成文件以便组件文件保持「只导出组件」（fast-refresh 约束）。
 */
export function maskApiKeyDisplay(key: string): string {
  if (key.startsWith("sk-agw-")) return "sk-agw-****"
  if (key.startsWith("agw-")) return "agw-****"
  if (key.startsWith("sk-")) return "sk-****"
  return "****"
}
