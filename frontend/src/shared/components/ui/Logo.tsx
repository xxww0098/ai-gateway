import React from 'react'
import { cn } from '@/shared/utils/utils'

export type LogoSize = 'xs' | 'sm' | 'md' | 'lg' | 'xl' | number

export interface LogoProps extends React.HTMLAttributes<HTMLDivElement> {
  size?: LogoSize
  variant?: 'icon' | 'mark'
  showText?: boolean
  textClassName?: string
}

const sizeMap: Record<string, { box: string; px: number; textSize: string }> = {
  xs: { box: 'w-6 h-6', px: 24, textSize: 'text-sm font-semibold' },
  sm: { box: 'w-8 h-8', px: 32, textSize: 'text-base font-bold' },
  md: { box: 'w-10 h-10', px: 40, textSize: 'text-lg font-bold' },
  lg: { box: 'w-12 h-12', px: 48, textSize: 'text-xl font-bold' },
  xl: { box: 'w-16 h-16', px: 64, textSize: 'text-2xl font-bold' },
}

export const Logo: React.FC<LogoProps> = ({
  size = 'md',
  variant = 'icon',
  showText = false,
  textClassName,
  className,
  ...props
}) => {
  const sizeConfig = typeof size === 'number'
    ? { box: '', px: size, textSize: 'text-base font-bold' }
    : (sizeMap[size] || sizeMap.md)

  const customStyle = typeof size === 'number' ? { width: `${size}px`, height: `${size}px` } : undefined

  return (
    <div className={cn('inline-flex items-center gap-2.5 select-none shrink-0', className)} {...props}>
      <div
        className={cn('relative flex items-center justify-center shrink-0', sizeConfig.box)}
        style={customStyle}
      >
        {variant === 'mark' ? (
          <svg
            viewBox="0 0 512 512"
            className="w-full h-full"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
          >
            {/* Outer Gateway Arc 'C' */}
            <path
              d="M 360 152 A 148 148 0 1 0 360 360"
              className="stroke-slate-900 dark:stroke-white transition-colors"
              strokeWidth="28"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            {/* Inner Concentric Stream Conduit */}
            <path
              d="M 315 197 A 84 84 0 1 0 315 315"
              stroke="#0D9488"
              strokeWidth="22"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            {/* Central Precision Data Beam */}
            <path
              d="M 180 256 L 275 256"
              className="stroke-slate-900 dark:stroke-white transition-colors"
              strokeWidth="20"
              strokeLinecap="round"
            />
            {/* Focal Core Node */}
            <circle cx="308" cy="256" r="15" fill="#0D9488" />
            <circle cx="308" cy="256" r="6" fill="#FFFFFF" />
          </svg>
        ) : (
          <svg
            viewBox="0 0 512 512"
            className="w-full h-full drop-shadow-sm rounded-xl"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
          >
            {/* Clean White Container */}
            <rect width="512" height="512" rx="112" fill="#FFFFFF" />
            <rect x="2" y="2" width="508" height="508" rx="110" stroke="#E2E8F0" strokeWidth="3" fill="none" />

            {/* Outer Gateway Arc 'C' */}
            <path
              d="M 360 152 A 148 148 0 1 0 360 360"
              stroke="#0F172A"
              strokeWidth="26"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            {/* Inner Concentric Stream Conduit */}
            <path
              d="M 315 197 A 84 84 0 1 0 315 315"
              stroke="#0D9488"
              strokeWidth="20"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            {/* Central Precision Data Beam */}
            <path
              d="M 180 256 L 275 256"
              stroke="#0F172A"
              strokeWidth="18"
              strokeLinecap="round"
            />
            {/* Focal Core Node */}
            <circle cx="308" cy="256" r="14" fill="#0D9488" />
            <circle cx="308" cy="256" r="6" fill="#FFFFFF" />
          </svg>
        )}
      </div>

      {showText && (
        <span
          className={cn(
            'tracking-tight font-bold bg-clip-text text-transparent bg-gradient-to-r from-gray-900 via-gray-800 to-gray-600 dark:from-white dark:via-gray-100 dark:to-gray-300',
            sizeConfig.textSize,
            textClassName
          )}
        >
          AI-GateWay
        </span>
      )}
    </div>
  )
}

export default Logo
