import { Badge } from "@/components/ui/badge";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { decidedByLabel } from "@/lib/utils/quality-decision-pipeline";

/**
 * Names the decision-order step that settled a recorded comparison. Renders
 * nothing when no step was recorded.
 */
export function DecidedByBadge({
  id,
  step,
  className,
}: {
  id: string;
  step: string | null | undefined;
  className?: string;
}) {
  const t = useTranslate();
  const label = decidedByLabel(step, t);
  if (!label) return null;
  return (
    <Badge id={id} tone="info" className={cn("max-w-full", className)}>
      <span className="truncate">{label}</span>
    </Badge>
  );
}
