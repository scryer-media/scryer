import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { Slot } from "radix-ui";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-all disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0 outline-none focus-visible:ring-2 focus-visible:ring-offset-0 aria-invalid:ring-[var(--scry-danger-border)] aria-invalid:border-[var(--scry-danger-border-strong)]",
  {
    variants: {
      variant: {
        default:
          "border-0 bg-primary text-primary-foreground shadow-none hover:bg-primary/90 focus-visible:ring-[var(--scry-accent-ring)]",
        primary:
          "border-0 bg-primary text-primary-foreground shadow-none hover:bg-primary/90 focus-visible:ring-[var(--scry-accent-ring)]",
        destructive:
          "border border-[var(--scry-danger-border)] bg-[var(--scry-danger-solid)] text-[var(--scry-danger-on-solid)] hover:border-[var(--scry-danger-border-strong)] hover:bg-[var(--scry-danger-solid-hover)] focus-visible:ring-[var(--scry-danger-border-strong)]",
        warning:
          "border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)] hover:border-[var(--scry-warning-border-strong)] hover:bg-[var(--scry-warning-bg-strong)] focus-visible:ring-[var(--scry-warning-border-strong)]",
        success:
          "border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)] hover:border-[var(--scry-success-border-strong)] hover:bg-[var(--scry-success-bg-strong)] focus-visible:ring-[var(--scry-success-border-strong)]",
        outline:
          "border border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[var(--scry-ink2)] shadow-none hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:ring-[var(--scry-accent-ring)]",
        secondary:
          "border border-[var(--scry-border2)] bg-[var(--scry-soft)] text-[var(--scry-text2)] shadow-none hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:ring-[var(--scry-accent-ring)]",
        ghost:
          "text-[var(--scry-muted2)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:ring-[var(--scry-accent-ring)]",
        link: "text-[var(--scry-accent-text)] underline-offset-4 hover:underline focus-visible:ring-[var(--scry-accent-ring)]",
      },
      size: {
        default: "h-9 px-4 py-2 has-[>svg]:px-3",
        xs: "h-6 gap-1 rounded-md px-2 text-xs has-[>svg]:px-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-8 rounded-md gap-1.5 px-3 has-[>svg]:px-2.5",
        lg: "h-10 rounded-md px-6 has-[>svg]:px-4",
        icon: "size-9",
        "icon-xs": "size-6 rounded-md [&_svg:not([class*='size-'])]:size-3",
        "icon-sm": "size-8",
        "icon-lg": "size-10",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

function Button({
  className,
  variant = "default",
  size = "default",
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
  }) {
  const Comp = asChild ? Slot.Root : "button";

  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}

export { Button, buttonVariants };
