import type { TitleReleaseBlocklistEntry } from "@/components/containers/series-overview-container";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { Loader2 } from "lucide-react";
import { formatDate } from "./helpers";

export function EpisodeBlocklistPanel({
  entries,
  canClear = false,
  clearingEntryId = null,
  onClear,
}: {
  entries: TitleReleaseBlocklistEntry[];
  canClear?: boolean;
  clearingEntryId?: string | null;
  onClear?: (entryId: string) => Promise<void> | void;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  if (entries.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        {t("episode.noBlockedReleases")}
      </p>
    );
  }

  return (
    <div className="space-y-2">
      {entries.map((entry) => (
        <div
          key={entry.id}
          className="rounded-lg border border-border bg-background/35 p-3"
        >
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0 flex-1">
              <p className="break-words text-sm text-card-foreground">
                {entry.releaseName || t("episode.untitledRelease")}
              </p>
              <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                <span className="text-muted-foreground/60">
                  {formatDate(entry.attemptedAt, dateTimeFormat)}
                </span>
                {entry.errorMessage ? (
                  <span className="rounded bg-[var(--scry-danger-bg)] px-2 py-0.5 text-[var(--scry-danger-text)]">
                    {entry.errorMessage}
                  </span>
                ) : null}
              </div>
            </div>
            {canClear && onClear ? (
              <Button
                type="button"
                variant="destructive"
                size="sm"
                className="h-8 shrink-0 px-3"
                disabled={clearingEntryId === entry.id}
                onClick={() => onClear(entry.id)}
              >
                {clearingEntryId === entry.id ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : null}
                <span>{t("label.clear")}</span>
              </Button>
            ) : null}
          </div>
        </div>
      ))}
    </div>
  );
}
