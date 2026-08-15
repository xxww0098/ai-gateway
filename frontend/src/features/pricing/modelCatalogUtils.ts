import { getProviderColor } from '@/features/pricing/model_catalog'

export function getProviderStyle(provider: string) {
  return getProviderColor(provider)
}
