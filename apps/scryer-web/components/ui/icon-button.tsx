import * as React from "react";

import { Button } from "@/components/ui/button";
import { ActionTooltip } from "@/components/ui/tooltip";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";
import { cn } from "@/lib/utils";

type IconButtonProps = Omit<
  React.ComponentProps<typeof Button>,
  "size" | "variant"
> & {
  label: string;
  appearance?: "boxed" | "ghost";
  tone?: BoxedActionButtonTone;
  showTitleAttribute?: boolean;
  tooltip?: React.ReactNode | false;
  tooltipSide?: React.ComponentProps<typeof ActionTooltip>["side"];
  tooltipClassName?: string;
  tooltipUseProvider?: boolean;
};

function IconButton({
  label,
  appearance = "boxed",
  tone = "neutral",
  showTitleAttribute = true,
  tooltip,
  tooltipSide,
  tooltipClassName,
  tooltipUseProvider,
  className,
  title,
  children,
  ...props
}: IconButtonProps) {
  const ariaLabel = props["aria-label"] ?? label;
  let tooltipContent = tooltip;
  if (tooltipContent === undefined) {
    tooltipContent = showTitleAttribute === false ? false : (title ?? label);
  }

  const button = (
    <Button
      type="button"
      size="icon-sm"
      variant={appearance === "boxed" ? "secondary" : "ghost"}
      aria-label={ariaLabel}
      className={cn(
        appearance === "boxed"
          ? [boxedActionButtonBaseClass, boxedActionButtonToneClass[tone]]
          : "text-[var(--scry-muted2)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );

  return (
    <ActionTooltip
      content={tooltipContent}
      side={tooltipSide}
      className={tooltipClassName}
      useProvider={tooltipUseProvider}
      wrapperTabIndex={
        props.disabled && tooltipContent !== false && tooltipContent != null
          ? 0
          : undefined
      }
    >
      {button}
    </ActionTooltip>
  );
}

export { IconButton };
export type { IconButtonProps };
