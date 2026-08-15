import { memo } from 'react'
import { Bot } from 'lucide-react'
import { cn } from '@/shared/utils/utils'
import { LOBE_BRAND_ICON_ALIASES, LOBE_BRAND_ICONS } from './lobehubBrandIcons'

const normalizeProvider = (provider?: string): string =>
  (provider || '').trim().toLowerCase()

export const AuthProviderBrandIcon = memo(function AuthProviderBrandIcon({
  provider,
  size = 20,
  className,
}: {
  provider?: string
  size?: number
  className?: string
}) {
  const key = normalizeProvider(provider)
  if (!key) return null

  const resolved = LOBE_BRAND_ICON_ALIASES[key] || key
  const Icon = LOBE_BRAND_ICONS[resolved] ?? LOBE_BRAND_ICONS[key]
  if (Icon) {
    return <Icon size={size} className={cn('shrink-0', className)} />
  }

  return (
    <Bot
      size={size}
      aria-hidden
      className={cn('shrink-0 text-muted-foreground', className)}
    />
  )
})
