import type { ComponentProps } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { cn } from "@/lib/utils";

type TableDisclosureButtonProps = Omit<
  ComponentProps<"button">,
  "aria-expanded" | "children" | "type"
> & {
  expanded: boolean;
};

export function TableDisclosureButton({
  expanded,
  className,
  ...props
}: TableDisclosureButtonProps) {
  return (
    <button
      type="button"
      data-slot="table-disclosure-button"
      aria-expanded={expanded}
      className={cn(
        "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border/60 bg-card/80 text-muted-foreground transition hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        className,
      )}
      {...props}
    >
      {expanded ? (
        <ChevronUp className="h-4 w-4" aria-hidden="true" />
      ) : (
        <ChevronDown className="h-4 w-4" aria-hidden="true" />
      )}
    </button>
  );
}
