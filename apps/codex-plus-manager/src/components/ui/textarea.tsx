import * as React from "react";

import { cn } from "@/lib/utils";

const Textarea = React.forwardRef<HTMLTextAreaElement, React.TextareaHTMLAttributes<HTMLTextAreaElement>>(
  ({ className, ...props }, ref) => (
    <textarea
      className={cn(
        "flex min-h-24 w-full rounded-lg border border-input/80 bg-[hsl(var(--surface-sunken))] px-2.5 py-2 text-[12.5px] transition-[border-color,box-shadow,background-color] duration-150 ease-out placeholder:text-muted-foreground/70 hover:border-input focus-visible:border-[hsl(var(--brand-accent)/0.7)] focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      ref={ref}
      {...props}
    />
  ),
);
Textarea.displayName = "Textarea";

export { Textarea };
