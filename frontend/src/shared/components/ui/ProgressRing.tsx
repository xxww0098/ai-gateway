import { useId } from 'react';

interface ProgressRingProps {
  percentage: number; // 0 to 100
  size?: number;
  strokeWidth?: number;
  label?: string;
  value?: string | number;
  subValue?: string;
  gradientFrom?: string;
  gradientTo?: string;
}

export function ProgressRing({
  percentage,
  size = 176, // 44 * 4
  strokeWidth = 10,
  label,
  value,
  subValue,
  gradientFrom = '#3b82f6',
  gradientTo = '#8b5cf6',
}: ProgressRingProps) {
  const radius = (size - strokeWidth) / 2;
  const circumference = radius * 2 * Math.PI;
  const offset = circumference - (Math.max(0, Math.min(100, percentage)) / 100) * circumference;

  const gradientId = useId();

  return (
    <div className="relative flex justify-center items-center">
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="currentColor"
          className="text-muted/20"
          strokeWidth={strokeWidth}
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke={`url(#${gradientId})`}
          strokeWidth={strokeWidth}
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          className="transition-all duration-1000 ease-out transform -rotate-90 origin-center"
        />
        <defs>
          <linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor={gradientFrom} />
            <stop offset="100%" stopColor={gradientTo} />
          </linearGradient>
        </defs>
      </svg>
      <div className="absolute flex flex-col items-center justify-center text-center inset-0">
        {value !== undefined && (
          <span className="text-3xl font-bold tabular-nums text-foreground">
            {value}
          </span>
        )}
        {label && (
          <span className="text-xs text-muted-foreground mt-0.5">
            {label}
          </span>
        )}
        {subValue && (
          <span
            className="text-sm font-semibold mt-1 tabular-nums"
            style={{ color: gradientFrom }}
          >
            {subValue}
          </span>
        )}
      </div>
    </div>
  );
}
