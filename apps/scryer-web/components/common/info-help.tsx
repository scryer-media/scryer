
import { Info } from "lucide-react";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";

type InfoHelpProps = {
  text: string;
  ariaLabel: string;
};

export function InfoHelp({ text, ariaLabel }: InfoHelpProps) {
  return (
    <HoverCard openDelay={150} closeDelay={75}>
      <HoverCardTrigger asChild>
        <button
          type="button"
          className="rounded p-0.5 text-muted-foreground transition hover:text-card-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400/70"
          aria-label={ariaLabel}
        >
          <Info className="h-3.5 w-3.5" />
        </button>
      </HoverCardTrigger>
      <HoverCardContent>
        <p className="max-w-[28rem] whitespace-normal break-words">{text}</p>
      </HoverCardContent>
    </HoverCard>
  );
}
