import * as React from "react";
import { Tooltip as TooltipPrimitive } from "radix-ui";

import { cn } from "@/lib/utils";

function TooltipProvider({
  delayDuration = 0,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Provider>) {
  return (
    <TooltipPrimitive.Provider
      data-slot="tooltip-provider"
      delayDuration={delayDuration}
      {...props}
    />
  );
}

function Tooltip({
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Root>) {
  return <TooltipPrimitive.Root data-slot="tooltip" {...props} />;
}

function TooltipTrigger({
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Trigger>) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />;
}

function TooltipContent({
  className,
  sideOffset = 0,
  children,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        className={cn(
          "bg-foreground text-background animate-in fade-in-0 zoom-in-95 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 z-[90] w-fit origin-(--radix-tooltip-content-transform-origin) rounded-md px-3 py-1.5 text-xs text-balance",
          className,
        )}
        {...props}
      >
        {children}
        <TooltipPrimitive.Arrow className="bg-foreground fill-foreground z-[90] size-2.5 translate-y-[calc(-50%_-_2px)] rotate-45 rounded-[2px]" />
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}

type ActionTooltipProps = {
  content?: React.ReactNode;
  children: React.ReactElement;
  side?: React.ComponentProps<typeof TooltipPrimitive.Content>["side"];
  sideOffset?: React.ComponentProps<
    typeof TooltipPrimitive.Content
  >["sideOffset"];
  collisionPadding?: React.ComponentProps<
    typeof TooltipPrimitive.Content
  >["collisionPadding"];
  delayDuration?: React.ComponentProps<
    typeof TooltipPrimitive.Provider
  >["delayDuration"];
  useProvider?: boolean;
  className?: string;
  wrapperClassName?: string;
  wrapperTabIndex?: number;
};

function ActionTooltip({
  content,
  children,
  side = "top",
  sideOffset = 8,
  collisionPadding = 8,
  delayDuration = 300,
  useProvider = true,
  className,
  wrapperClassName,
  wrapperTabIndex,
}: ActionTooltipProps) {
  if (content == null || content === false) {
    return children;
  }

  const tooltip = (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className={cn("inline-flex", wrapperClassName)}
          tabIndex={wrapperTabIndex}
        >
          {children}
        </span>
      </TooltipTrigger>
      <TooltipContent
        side={side}
        sideOffset={sideOffset}
        collisionPadding={collisionPadding}
        className={cn(
          "max-w-[18rem] whitespace-normal break-words text-left text-sm leading-snug",
          className,
        )}
      >
        {content}
      </TooltipContent>
    </Tooltip>
  );

  return useProvider ? (
    <TooltipProvider delayDuration={delayDuration}>{tooltip}</TooltipProvider>
  ) : (
    tooltip
  );
}

export {
  ActionTooltip,
  Tooltip,
  TooltipTrigger,
  TooltipContent,
  TooltipProvider,
};
