import * as React from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { scanTitleLibraryMutation } from "@/lib/graphql/mutations";

/**
 * A title overview's Refresh action: rescan the title's folder for files, then
 * reload the title. Both title overviews use this one action.
 */
export function useTitleRefreshAndScan({
  titleId,
  onScanned,
}: {
  titleId: string | null;
  /** Reloads the title after its folder has been scanned. */
  onScanned: () => Promise<void> | void;
}): { loading: boolean; refreshAndScan: () => Promise<void> } {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [loadingTitleId, setLoadingTitleId] = React.useState<string | null>(null);

  const refreshAndScan = React.useCallback(async () => {
    if (!titleId) {
      return;
    }
    const requestedTitleId = titleId;
    setLoadingTitleId(requestedTitleId);
    try {
      const { data, error } = await client
        .mutation<{
          scanTitleLibrary?: {
            imported?: number;
            skipped?: number;
            unmatched?: number;
          } | null;
        }>(scanTitleLibraryMutation, { titleId: requestedTitleId })
        .toPromise();
      if (error) {
        throw error;
      }
      const summary = data?.scanTitleLibrary;
      const skipped = summary?.skipped ?? 0;
      const unmatched = summary?.unmatched ?? 0;
      setGlobalStatus(
        t("status.titleScanSuccess", {
          imported: summary?.imported ?? 0,
          skipped,
          unmatched,
        }),
        // A scan that left files behind is worth a second look; one that
        // took everything is a plain success.
        { level: skipped > 0 || unmatched > 0 ? "WARNING" : "SUCCESS" },
      );
      await onScanned();
    } catch (error: unknown) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("settings.libraryScanFailed"),
        { level: "ERROR" },
      );
    } finally {
      setLoadingTitleId((current) =>
        current === requestedTitleId ? null : current,
      );
    }
  }, [client, onScanned, setGlobalStatus, t, titleId]);

  return {
    loading: titleId !== null && loadingTitleId === titleId,
    refreshAndScan,
  };
}
