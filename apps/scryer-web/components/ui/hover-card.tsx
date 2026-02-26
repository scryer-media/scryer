
import * as React from "react"
import * as HoverCardPrimitive from "@radix-ui/react-hover-card"

import { cn } from "@/lib/utils"

const HoverCard = HoverCardPrimitive.Root

const HoverCardTrigger = HoverCardPrimitive.Trigger

const HoverCardContent = React.forwardRef<
  React.ElementRef<typeof HoverCardPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof HoverCardPrimitive.Content>
>(({ className, align = "start", sideOffset = 8, children, ...props }, ref) => (
  <HoverCardPrimitive.Content
    ref={ref}
    align={align}
    sideOffset={sideOffset}
    className={cn(
      "z-50 w-80 rounded-md border border-border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-lg",
      className,
    )}
    {...props}
  >
    {children}
  </HoverCardPrimitive.Content>
))
HoverCardContent.displayName = "HoverCardContent"

export { HoverCard, HoverCardTrigger, HoverCardContent }
