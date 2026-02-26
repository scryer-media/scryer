
import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group";

import { cn } from "@/lib/utils";

const toggleGroupVariants = cva(
  "inline-flex items-center rounded-xl border border-border/70 bg-background/80 p-1.5 shadow-sm",
  {
    variants: {
      variant: {
        default: "",
        outline: "border border-border bg-transparent p-0",
      },
      size: {
        default: "h-12",
        sm: "h-8",
        lg: "h-12",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

const toggleGroupItemVariants = cva(
  "inline-flex items-center justify-center whitespace-nowrap rounded-md font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        default:
          "bg-card/40 text-card-foreground hover:bg-accent/90 data-[state=on]:bg-primary data-[state=on]:text-primary-foreground data-[state=on]:border data-[state=on]:border-primary/60 data-[state=on]:shadow-[0_0_0_2px_rgba(132,204,22,0.4)]",
        outline:
          "px-2.5 data-[state=on]:bg-accent/70 data-[state=on]:text-foreground",
      },
      size: {
        default: "h-10 px-4 text-sm",
        sm: "h-6 px-2.5 text-xs",
        lg: "h-10 px-6 text-sm",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

const ToggleGroup = React.forwardRef<
  React.ElementRef<typeof ToggleGroupPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof ToggleGroupPrimitive.Root> &
    VariantProps<typeof toggleGroupVariants>
>(({ className, variant, size, ...props }, ref) => (
  <ToggleGroupPrimitive.Root
    ref={ref}
    className={cn(toggleGroupVariants({ variant, size }), className)}
    {...props}
  />
));

const ToggleGroupItem = React.forwardRef<
  React.ElementRef<typeof ToggleGroupPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof ToggleGroupPrimitive.Item> &
    VariantProps<typeof toggleGroupItemVariants>
>(({ className, variant, size, ...props }, ref) => (
  <ToggleGroupPrimitive.Item
    ref={ref}
    className={cn(
      toggleGroupItemVariants({
        variant,
        size,
      }),
      className,
    )}
    {...props}
  />
));

export { ToggleGroup, ToggleGroupItem };
