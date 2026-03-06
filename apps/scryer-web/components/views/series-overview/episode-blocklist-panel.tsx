import type { TitleReleaseBlocklistEntry } from "@/components/containers/series-overview-container";
import { formatDate } from "./helpers";

export function EpisodeBlocklistPanel({
  entries,
}: {
  entries: TitleReleaseBlocklistEntry[];
}) {
  if (entries.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No blocked releases recorded for this episode.
      </p>
    );
  }

  return (
    <div className="space-y-2">
      {entries.map((entry) => (
        <div
          key={`${entry.sourceHint ?? ""}-${entry.attemptedAt}-${entry.sourceTitle ?? ""}`}
          className="rounded-lg border border-border bg-background/35 p-3"
        >
          <p className="break-words text-sm text-card-foreground">
            {entry.sourceTitle || "Untitled release"}
          </p>
          {entry.sourceHint ? (
            <p className="mt-1 break-all font-mono text-xs text-muted-foreground/60">
              {entry.sourceHint}
            </p>
          ) : null}
          <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
            <span className="text-muted-foreground/60">{formatDate(entry.attemptedAt)}</span>
            {entry.errorMessage ? (
              <span className="rounded bg-red-950/40 px-2 py-0.5 text-red-200">
                {entry.errorMessage}
              </span>
            ) : null}
          </div>
        </div>
      ))}
    </div>
  );
}
