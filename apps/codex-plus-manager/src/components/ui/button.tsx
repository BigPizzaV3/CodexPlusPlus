import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg text-[12.5px] font-medium tracking-[-0.005em] transition-[background-color,border-color,color,box-shadow,transform] duration-150 ease-out focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/30 active:scale-[0.985] disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-[15px] [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground shadow-[0_1px_2px_hsl(var(--shadow-color)/0.28),inset_0_1px_0_hsl(0_0%_100%/0.14)] hover:bg-primary/90 active:bg-primary/95",
        secondary:
          "border border-border/70 bg-secondary text-secondary-foreground hover:border-border hover:bg-secondary/70",
        outline:
          "border border-input/80 bg-transparent text-secondary-foreground hover:border-input hover:bg-accent/60 hover:text-accent-foreground",
        ghost: "text-muted-foreground hover:bg-accent/70 hover:text-accent-foreground",
      },
      size: {
        default: "h-8 px-3",
        sm: "h-7 rounded-md px-2.5 text-[12px]",
        icon: "h-8 w-8",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return <Comp className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props} />;
  },
);
Button.displayName = "Button";

export { Button, buttonVariants };
