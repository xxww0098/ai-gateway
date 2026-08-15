import * as React from "react"
import { Slot } from "@radix-ui/react-slot"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/shared/utils/utils"

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground shadow hover:bg-primary/90",
        destructive:
          "bg-destructive text-destructive-foreground shadow-sm hover:bg-destructive/90",
        outline:
          "border border-input bg-background shadow-sm hover:bg-accent hover:text-accent-foreground",
        secondary:
          "bg-secondary text-secondary-foreground shadow-sm hover:bg-secondary/80",
        ghost: "hover:bg-accent hover:text-accent-foreground",
        link: "text-primary underline-offset-4 hover:underline",
        /**
         * 列表/表格内仅图标的删除：圆形微标、细边浅底，悬停才带一点玫瑰色，避免大块纯色。
         */
        dangerIcon:
          "!h-8 !w-8 !min-h-0 shrink-0 gap-0 rounded-full border border-zinc-200/90 bg-white p-0 text-zinc-400 shadow-[0_1px_2px_rgba(0,0,0,0.04)] transition-[color,background-color,border-color,box-shadow,transform] duration-200 hover:border-rose-200/90 hover:bg-rose-50/95 hover:text-rose-600 hover:shadow-[0_2px_6px_rgba(225,29,72,0.08)] active:scale-[0.97] dark:border-zinc-700 dark:bg-zinc-900/90 dark:text-zinc-500 dark:hover:border-rose-900/45 dark:hover:bg-rose-950/35 dark:hover:text-rose-400 [&_svg]:!size-[15px]",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 rounded-md px-3 text-xs",
        lg: "h-10 rounded-md px-8",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button"
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    )
  }
)
Button.displayName = "Button"

export { Button }
