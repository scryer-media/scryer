import { ArrowLeft } from "lucide-react";
import { cn } from "@/lib/utils";

type Props = {
  label: string;
  onClick?: () => void;
  className?: string;
};

export function OverviewBackLink({ label, onClick, className }: Props) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "group inline-flex w-fit items-center gap-2 rounded-full border border-border/70 bg-card/70 px-2.5 py-1.5 text-sm font-medium text-muted-foreground shadow-sm backdrop-blur-sm transition-colors hover:border-border hover:bg-card hover:text-foreground",
        className,
      )}
    >
      <span className="flex size-6 items-center justify-center rounded-full bg-background/85 text-foreground/80 transition-transform group-hover:-translate-x-0.5">
        <ArrowLeft className="h-3.5 w-3.5" />
      </span>
      <span className="leading-none">{label}</span>
    </button>
  );
}
