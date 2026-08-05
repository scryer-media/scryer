import { Info } from "lucide-react";
import type { ReactNode } from "react";
import { ActionTooltip } from "@/components/ui/tooltip";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useIsMobile } from "@/lib/hooks/use-mobile";

type InfoHelpProps = {
  text: ReactNode;
  ariaLabel: string;
};

export function InfoHelp({ text, ariaLabel }: InfoHelpProps) {
  const isMobile = useIsMobile();
  const trigger = (
    <button
      type="button"
      className="rounded p-0.5 text-muted-foreground transition hover:text-card-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-info-border-strong)]"
      aria-label={ariaLabel}
    >
      <Info className="h-3.5 w-3.5" />
    </button>
  );

  if (isMobile) {
    return (
      <Popover>
        <PopoverTrigger asChild>{trigger}</PopoverTrigger>
        <PopoverContent align="start">
          <p className="max-w-[28rem] whitespace-normal break-words">{text}</p>
        </PopoverContent>
      </Popover>
    );
  }

  return (
    <ActionTooltip
      content={text}
      className="max-w-[28rem] whitespace-normal break-words"
    >
      {trigger}
    </ActionTooltip>
  );
}
