import { useTranslate } from "@/lib/context/translate-context";
import type { ReleaseDecisionExplanationEntry } from "@/lib/utils/release-decision-explanation";

export function WantedScoringBreakdown({
  entries,
}: {
  entries: ReleaseDecisionExplanationEntry[];
}) {
  const t = useTranslate();

  return (
    <div className="mt-3 rounded-md border border-border/70 bg-background/60 p-3">
      <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        <span>{t("wanted.scoreCode")}</span>
        <span>{t("wanted.decDelta")}</span>
      </div>
      <div className="mt-2 space-y-1">
        {entries.map((entry, index) => (
          <div
            key={`${entry.code}-${index}`}
            className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 font-[var(--font-code)] text-xs text-foreground"
          >
            <span className="truncate" title={entry.code}>
              {entry.code}
            </span>
            <span>{entry.delta > 0 ? `+${entry.delta}` : entry.delta}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
