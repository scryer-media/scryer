import { Progress } from "@/components/ui/progress";
import { cn } from "@/lib/utils";

export function ActivityProgressBar({
  percent,
  remainingLabel,
  colorClass,
  compact = false,
}: {
  percent: number;
  remainingLabel: string | null;
  colorClass: string;
  compact?: boolean;
}) {
  return (
    <div>
      <div className="mb-1 flex items-center justify-between text-xs">
        <p className="font-semibold tabular-nums text-foreground">{percent}%</p>
        <p className="text-muted-foreground">{remainingLabel ?? "\u2014"}</p>
      </div>
      <Progress
        value={percent}
        className={cn("h-2.5 bg-muted/90", compact && "h-1.5")}
        indicatorClassName={colorClass}
      />
    </div>
  );
}
