
import { memo, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";

import { ImportHistoryView } from "@/components/views/import-history-view";
import { importHistoryQuery } from "@/lib/graphql/queries";
import { retryImportMutation } from "@/lib/graphql/mutations";
import { useImportHistorySubscription } from "@/lib/hooks/use-import-history-subscription";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { ImportRecord } from "@/lib/types";

export const ImportHistoryContainer = memo(function ImportHistoryContainer() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();

  const [records, setRecords] = useState<ImportRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [limit, setLimit] = useState(100);

  const refresh = useCallback(async (nextLimit?: number) => {
    setLoading(true);
    setError(null);
    try {
      const { data, error: gqlError } = await client
        .query(importHistoryQuery, { limit: nextLimit ?? limit })
        .toPromise();
      if (gqlError) throw gqlError;
      setRecords(data?.importHistory ?? []);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load import history");
    } finally {
      setLoading(false);
    }
  }, [client, limit]);

  const handleLimitChange = useCallback((nextLimit: number) => {
    setLimit(nextLimit);
    void refresh(nextLimit);
  }, [refresh]);

  const handleRetry = useCallback(async (importId: string, password?: string) => {
    try {
      const { error: retryError } = await client
        .mutation(retryImportMutation, {
          input: { importId, password: password || null },
        })
        .toPromise();
      if (retryError) throw retryError;
      setGlobalStatus(t("importHistory.retrySuccess"));
      void refresh();
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.apiError"));
    }
  }, [client, refresh, setGlobalStatus, t]);

  useEffect(() => {
    void refresh();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useImportHistorySubscription(() => void refresh());

  return (
    <ImportHistoryView
      records={records}
      loading={loading}
      error={error}
      limit={limit}
      onLimitChange={handleLimitChange}
      onRefresh={() => void refresh()}
      onRetry={handleRetry}
    />
  );
});
