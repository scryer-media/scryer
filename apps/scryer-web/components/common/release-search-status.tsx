import * as React from "react";
import { LoadingMark } from "@/components/common/loading-mark";
import { useTranslate } from "@/lib/context/translate-context";
import type { Release } from "@/lib/types";
import type { InteractiveSearchPresentation } from "@/lib/utils/interactive-search-presentation";

/**
 * The line above an interactive search's results: a small spinner and the
 * running release and indexer counts while the search runs, then what the run
 * did, with any failed or skipped indexers called out below it. Every
 * interactive search (movie, season, episode) shows the same status.
 */
export function ReleaseSearchSummary({
  presentation,
  results,
  dataUi,
}: {
  presentation: InteractiveSearchPresentation;
  results: Release[];
  dataUi: string;
}) {
  const t = useTranslate();
  const description = React.useMemo(() => {
    if (presentation.showProgress) {
      return t("title.contextReleaseSearchProgress", {
        releaseCount: results.length,
        done: presentation.completedIndexerCount,
        total: presentation.totalIndexerCount,
      });
    }
    // Report what the run did, not which sources happened to return
    // results: an indexer that answered with nothing still searched, and one
    // that failed or was skipped is called out on its own line below.
    if (presentation.totalIndexerCount > 0) {
      return t("title.contextReleaseSearchSummaryDetailed", {
        releaseCount: results.length,
        searched: presentation.searchedIndexerCount,
        total: presentation.totalIndexerCount,
      });
    }
    const sourceCount = new Set(
      results
        .map((release) => release.source?.trim())
        .filter((source): source is string => Boolean(source)),
    ).size;
    return t("title.contextReleaseSearchSummary", {
      releaseCount: results.length,
      indexerCount: sourceCount,
    });
  }, [presentation, results, t]);

  if (!presentation.showProgress && !presentation.showFinalSummary) {
    return null;
  }

  return (
    <div className="min-w-0 flex-1">
      <p
        className="flex items-center gap-1.5 truncate text-[11.5px] text-[var(--scry-faint)]"
        data-ui={dataUi}
        data-search-state={presentation.showProgress ? "searching" : "done"}
      >
        {presentation.showProgress ? (
          <LoadingMark
            className="h-3 w-3 shrink-0"
            label={t("label.searching")}
          />
        ) : null}
        <span className="truncate">{description}</span>
      </p>
      {presentation.showFinalSummary &&
      presentation.failedIndexerNames.length > 0 ? (
        <p className="mt-0.5 truncate text-[11.5px] text-[var(--scry-danger-text)]">
          {t("title.contextReleaseSearchIndexerFailures", {
            count: presentation.failedIndexerNames.length,
            names: presentation.failedIndexerNames.join(", "),
          })}
        </p>
      ) : null}
      {presentation.showFinalSummary &&
      presentation.skippedIndexers.length > 0 ? (
        <p
          className="mt-0.5 truncate text-[11.5px] text-[var(--scry-faint)]"
          title={presentation.skippedIndexers
            .map((indexer) =>
              indexer.reason
                ? `${indexer.name}: ${indexer.reason}`
                : indexer.name,
            )
            .join("\n")}
        >
          {t("title.contextReleaseSearchIndexerSkipped", {
            count: presentation.skippedIndexers.length,
            names: presentation.skippedIndexers
              .map((indexer) => indexer.name)
              .join(", "),
          })}
        </p>
      ) : null}
    </div>
  );
}

/** The large spinner shown in place of results until the first release arrives. */
export function ReleaseSearchLoader() {
  const t = useTranslate();
  return (
    <div className="flex flex-col items-center justify-center gap-4 py-16">
      <LoadingMark className="h-10 w-10 text-[var(--scry-accent-text)]" />
      <p className="text-lg text-muted-foreground">{t("label.searching")}</p>
    </div>
  );
}
