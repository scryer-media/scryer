import * as React from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { clearTitleReleaseBlocklistEntryMutation } from "@/lib/graphql/mutations";

/**
 * Remove one release from a title's blocklist. Both title overviews use this,
 * so the removal reports and refreshes the same way on each.
 */
export function useTitleReleaseBlocklistClear({
  onCleared,
}: {
  /**
   * Reloads the blocklist. The overview shows a capped window, so removing one
   * entry can uncover an older one; reload rather than splicing.
   */
  onCleared: () => Promise<void> | void;
}): {
  clearingEntryId: string | null;
  clear: (entryId: string) => Promise<void>;
} {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [clearingEntryId, setClearingEntryId] = React.useState<string | null>(null);

  const clear = React.useCallback(
    async (entryId: string) => {
      setClearingEntryId(entryId);
      try {
        const { error } = await client
          .mutation(clearTitleReleaseBlocklistEntryMutation, { id: entryId })
          .toPromise();
        if (error) {
          throw error;
        }
        setGlobalStatus(t("status.blocklistEntryCleared"));
        await onCleared();
      } catch (error: unknown) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"), { level: "ERROR" });
      } finally {
        setClearingEntryId((current) => (current === entryId ? null : current));
      }
    },
    [client, onCleared, setGlobalStatus, t],
  );

  return { clearingEntryId, clear };
}
