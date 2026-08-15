import { memo } from 'react'
import { AuthProviderBrandIcon } from './AuthProviderBrandIcon'
import { cn } from '@/shared/utils/utils'

/** OAuth card brand icon — delegates to AuthProviderBrandIcon (@lobehub/icons). */
export const OAuthProviderBrandIcon = memo(function OAuthProviderBrandIcon({
  providerKey,
  size = 24,
  className,
}: {
  providerKey: string
  size?: number
  className?: string
}) {
  return (
    <AuthProviderBrandIcon
      provider={providerKey}
      size={size}
      className={cn('shrink-0', className)}
    />
  )
})
