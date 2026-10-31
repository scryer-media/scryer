import * as React from "react";
import { ChevronRight, Trash2 } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { TitleReleaseBlocklistEntry } from "@/lib/types/titles";
import { cn } from "@/lib/utils";
import { formatUiDate } from "@/lib/utils/date-format";

/**
 * A title's blocked releases, shared by both title overviews. The remove action
 * is offered only when the viewer may manage this title.
 */
export function TitleBlockedReleasesSection({
  entries,
  canManageTitle,
  clearingEntryId,
  onClear,
  className,
}: {
  entries: readonly TitleReleaseBlocklistEntry[];
  canManageTitle: boolean;
  clearingEntryId: string | null;
  onClear: (entryId: string) => Promise<void> | void;
  className?: string;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [open, setOpen] = React.useState(false);
  const empty = entries.length === 0;
  const sectionClassName = cn(
    "overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)]",
    className,
  );
  const header = (
    <>
      <ChevronRight
        className={cn(
          "h-4 w-4 shrink-0 text-[var(--scry-faint)] transition-transform",
          open && !empty && "rotate-90",
        )}
      />
      <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold text-[var(--scry-text2)]">
        {t("title.contextBlockedReleases")}
      </span>
      <span className="shrink-0 rounded-[7px] bg-white/[0.06] px-2 py-0.5 text-[11px] font-semibold text-[var(--scry-muted)]">
        {entries.length}
      </span>
    </>
  );

  if (empty) {
    return (
      <section
        className={cn(
          sectionClassName,
          "flex min-h-[3.25rem] items-center gap-2.5 px-4",
        )}
      >
        {header}
      </section>
    );
  }

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <section className={sectionClassName}>
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className="flex min-h-[3.25rem] w-full min-w-0 items-center gap-2.5 px-4 text-left transition hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--scry-focus)]"
          >
            {header}
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent className="border-t border-[var(--scry-line3)] p-4">
          <div className="space-y-2">
            {entries.map((entry) => {
              const attemptedAtLabel = formatUiDate(
                entry.attemptedAt,
                dateTimeFormat,
              );
              const releaseLabel =
                entry.releaseName.trim() || t("episode.untitledRelease");
              const clearing = clearingEntryId === entry.id;

              return (
                <div
                  key={entry.id}
                  className="rounded-[11px] border border-[var(--scry-line3)] bg-[var(--scry-inset)] p-3"
                >
                  <div className="flex min-w-0 items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="line-clamp-2 break-words text-[12px] font-semibold text-[var(--scry-ink2)]">
                        {releaseLabel}
                      </p>
                      {attemptedAtLabel ? (
                        <p className="mt-2 text-[11px] text-[var(--scry-muted3)]">
                          {attemptedAtLabel}
                        </p>
                      ) : null}
                    </div>
                    {canManageTitle ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        aria-label={t("title.clearBlockedRelease", {
                          releaseName: releaseLabel,
                        })}
                        title={t("title.clearBlockedReleaseHint")}
                        className="h-7 shrink-0 gap-1.5 px-2 text-[11px] font-semibold text-[var(--scry-muted)] hover:bg-[var(--scry-danger-bg)] hover:text-[var(--scry-danger-text)]"
                        disabled={clearing}
                        onClick={() => {
                          void onClear(entry.id);
                        }}
                      >
                        {clearing ? (
                          <LoadingMark className="h-3.5 w-3.5" />
                        ) : (
                          <Trash2 className="h-3.5 w-3.5" />
                        )}
                        <span>{t("label.remove")}</span>
                      </Button>
                    ) : null}
                  </div>
                  {entry.errorMessage ? (
                    <p className="mt-2 line-clamp-3 rounded-[8px] bg-[var(--scry-danger-bg)] px-2.5 py-1.5 text-[11px] leading-4 text-[var(--scry-danger-text)]">
                      {entry.errorMessage}
                    </p>
                  ) : null}
                </div>
              );
            })}
          </div>
        </CollapsibleContent>
      </section>
    </Collapsible>
  );
}
