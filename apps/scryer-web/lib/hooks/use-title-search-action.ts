import * as React from "react";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useAutomaticSearch } from "@/lib/hooks/use-automatic-search";
import { reportAutomaticSearchFailure } from "@/lib/utils/auto-search-outcome";

/**
 * A title overview's Search action. Both title overviews use it, so each shows
 * the same progress, the same failure messages, and the same notice when no
 * download client is set up.
 */
export function useTitleSearchAction({
  titleId,
  titleName,
  hasDownloadClients,
}: {
  titleId: string | null;
  titleName: string;
  /** `null` while that is still being checked; the search is then allowed. */
  hasDownloadClients: boolean | null;
}): {
  searching: boolean;
  search: () => Promise<void>;
  showDownloadClientNotice: boolean;
} {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const { startAutomaticSearch, isSearching } = useAutomaticSearch();
  const [pendingTitleId, setPendingTitleId] = React.useState<string | null>(null);
  // Remembered per title, so the notice goes away on the next title and once a
  // client is configured.
  const [noticeTitleId, setNoticeTitleId] = React.useState<string | null>(null);

  const search = React.useCallback(async () => {
    if (!titleId) {
      return;
    }
    if (hasDownloadClients === false) {
      setNoticeTitleId(titleId);
      return;
    }
    const requestedTitleId = titleId;
    setPendingTitleId(requestedTitleId);
    try {
      await startAutomaticSearch(requestedTitleId);
    } catch (error: unknown) {
      reportAutomaticSearchFailure(setGlobalStatus, t, error, titleName);
    } finally {
      setPendingTitleId((current) =>
        current === requestedTitleId ? null : current,
      );
    }
  }, [hasDownloadClients, setGlobalStatus, startAutomaticSearch, t, titleId, titleName]);

  return {
    searching:
      titleId !== null && (pendingTitleId === titleId || isSearching(titleId)),
    search,
    showDownloadClientNotice:
      titleId !== null && noticeTitleId === titleId && hasDownloadClients === false,
  };
}
