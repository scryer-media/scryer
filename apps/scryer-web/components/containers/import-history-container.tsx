
import { memo, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";

import { ImportHistoryView } from "@/components/views/import-history-view";
import { importHistoryQuery } from "@/lib/graphql/queries";
import type { ImportRecord } from "@/lib/types";

export const ImportHistoryContainer = memo(function ImportHistoryContainer() {
  const client = useClient();

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

  useEffect(() => {
    void refresh();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <ImportHistoryView
      records={records}
      loading={loading}
      error={error}
      limit={limit}
      onLimitChange={handleLimitChange}
      onRefresh={() => void refresh()}
    />
  );
});
