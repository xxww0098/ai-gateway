import type { ComponentType } from 'react'
import { getProviderColor } from '@/features/pricing/model_catalog'
import { LOBE_BRAND_ICONS, LOBE_BRAND_ICON_ALIASES } from '@/features/admin-proxy/components/lobehubBrandIcons'

export function getProviderStyle(provider: string) {
  return getProviderColor(provider)
}

export function getProviderBrandIcon(provider: string): ComponentType<{ size?: number; className?: string }> | null {
  const normalized = provider.toLowerCase().trim()
  const resolvedKey = LOBE_BRAND_ICON_ALIASES[normalized] ?? normalized
  return LOBE_BRAND_ICONS[resolvedKey] ?? null
}
