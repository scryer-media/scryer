import * as React from "react";

import { Button } from "@/components/ui/button";
import {
  boxedActionButtonToneClass,
  boxedTextActionButtonBaseClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";
import { cn } from "@/lib/utils";

type TextActionButtonProps = Omit<
  React.ComponentProps<typeof Button>,
  "variant"
> & {
  tone?: BoxedActionButtonTone;
  label?: React.ReactNode;
  leadingIcon?: React.ReactNode;
  showTitleAttribute?: boolean;
};

function getTitleText(value: React.ReactNode): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function TextActionButton({
  tone = "neutral",
  size = "sm",
  label,
  leadingIcon,
  showTitleAttribute = false,
  className,
  title,
  children,
  ...props
}: TextActionButtonProps) {
  const content = children ?? label;
  const titleText =
    getTitleText(title) ?? getTitleText(label) ?? getTitleText(children);

  return (
    <Button
      type="button"
      variant="outline"
      size={size}
      title={showTitleAttribute ? titleText : undefined}
      className={cn(
        boxedTextActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {leadingIcon}
      {content}
    </Button>
  );
}

export { TextActionButton };
export type { TextActionButtonProps };
