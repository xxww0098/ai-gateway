import * as React from "react"
import { Slot } from "@radix-ui/react-slot"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/shared/utils/utils"

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-xl text-sm font-semibold tracking-tight transition-all duration-150 ease-out focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground shadow-xs hover:bg-primary/90 active:bg-primary/85 active:scale-[0.98] disabled:bg-muted disabled:text-muted-foreground disabled:shadow-none",
        destructive:
          "bg-destructive text-destructive-foreground shadow-xs hover:bg-destructive/90 active:scale-[0.98] disabled:bg-muted disabled:text-muted-foreground disabled:shadow-none",
        outline:
          "border border-border bg-background hover:bg-accent hover:text-accent-foreground active:scale-[0.98] disabled:opacity-50",
        secondary:
          "bg-secondary text-secondary-foreground border border-border/60 hover:bg-secondary/80 active:scale-[0.98] disabled:opacity-50",
        ghost: "hover:bg-muted hover:text-foreground active:scale-[0.98] disabled:opacity-50",
        link: "text-primary underline-offset-4 hover:underline disabled:opacity-50",
        /**
         * 列表/表格内仅图标的删除：圆形微标、细边浅底，悬停才带一点玫瑰色，避免大块纯色。
         */
        dangerIcon:
          "!h-8 !w-8 !min-h-0 shrink-0 gap-0 rounded-full border border-border bg-background p-0 text-muted-foreground transition-all duration-200 hover:border-red-200 hover:bg-red-50 hover:text-red-600 active:scale-[0.97] dark:hover:border-red-900/50 dark:hover:bg-red-950/30 dark:hover:text-red-400 disabled:opacity-50 [&_svg]:!size-[15px]",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 rounded-xl px-3 text-xs",
        lg: "h-10 rounded-xl px-8",
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
