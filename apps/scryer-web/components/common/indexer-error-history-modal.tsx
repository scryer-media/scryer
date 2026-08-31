import * as React from "react";
import { AlertTriangle, Loader2, RefreshCw } from "lucide-react";
import { useClient } from "urql";
import { IndexerErrorTable } from "@/components/common/indexer-error-table";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  indexerErrorDetailQuery,
  indexerErrorsQuery,
} from "@/lib/graphql/indexer-errors";
import type {
  IndexerErrorConnection,
  IndexerErrorDetail,
} from "@/lib/types";

const PAGE_SIZE = 50;

export type IndexerErrorHistoryScope = {
  id: string;
  name: string;
};

export function IndexerErrorHistoryModal({
  open,
  onOpenChange,
  indexer,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  indexer?: IndexerErrorHistoryScope | null;
}) {
  const client = useClient();
  const t = useTranslate();
  const [page, setPage] = React.useState<IndexerErrorConnection>({
    items: [],
    nextCursor: null,
  });
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [details, setDetails] = React.useState<Record<string, IndexerErrorDetail | undefined>>({});
  const [detailLoading, setDetailLoading] = React.useState<Record<string, boolean | undefined>>({});
  const [detailErrors, setDetailErrors] = React.useState<Record<string, string | undefined>>({});
  const [tableVersion, setTableVersion] = React.useState(0);
  const requestGeneration = React.useRef(0);
  const activeDetailId = React.useRef<string | null>(null);

  const fetchPage = React.useCallback(async (after: string | null, append: boolean) => {
    const generation = requestGeneration.current;
    setLoading(true);
    setError(null);
    try {
      const result = await client.query<{ indexerErrors: IndexerErrorConnection }>(
        indexerErrorsQuery,
        { indexerId: indexer?.id ?? null, first: PAGE_SIZE, after },
        { requestPolicy: "network-only" },
      ).toPromise();
      if (generation !== requestGeneration.current) return;
      if (result.error) {
        setError(userFacingGraphQlErrorMessage(result.error, t("indexerErrors.loadError")));
        return;
      }
      const next = result.data?.indexerErrors;
      if (!next) {
        setError(t("indexerErrors.loadError"));
        return;
      }
      setPage((current) => {
        if (!append) return next;
        const existing = new Set(current.items.map((item) => item.id));
        return {
          items: [...current.items, ...next.items.filter((item) => !existing.has(item.id))],
          nextCursor: next.nextCursor,
        };
      });
    } catch (requestError) {
      if (generation === requestGeneration.current) {
        setError(userFacingGraphQlErrorMessage(requestError, t("indexerErrors.loadError")));
      }
    } finally {
      if (generation === requestGeneration.current) setLoading(false);
    }
  }, [client, indexer?.id, t]);

  React.useEffect(() => {
    requestGeneration.current += 1;
    activeDetailId.current = null;
    setPage({ items: [], nextCursor: null });
    setDetails({});
    setDetailLoading({});
    setDetailErrors({});
    setTableVersion((current) => current + 1);
    setError(null);
    if (open) void fetchPage(null, false);
  }, [fetchPage, open]);

  const fetchDetail = React.useCallback(async (id: string) => {
    const generation = requestGeneration.current;
    activeDetailId.current = id;
    setDetails({});
    setDetailLoading({ [id]: true });
    setDetailErrors({});
    try {
      const result = await client.query<{ indexerError: IndexerErrorDetail | null }>(
        indexerErrorDetailQuery,
        { id },
        { requestPolicy: "network-only" },
      ).toPromise();
      if (
        generation !== requestGeneration.current ||
        activeDetailId.current !== id
      ) return;
      if (result.error || !result.data?.indexerError) {
        setDetailErrors({
          [id]: result.error
            ? userFacingGraphQlErrorMessage(result.error, t("indexerErrors.detailLoadError"))
            : t("indexerErrors.detailMissing"),
        });
        return;
      }
      setDetails({ [id]: result.data.indexerError });
    } catch (requestError) {
      if (
        generation === requestGeneration.current &&
        activeDetailId.current === id
      ) {
        setDetailErrors({
          [id]: userFacingGraphQlErrorMessage(requestError, t("indexerErrors.detailLoadError")),
        });
      }
    } finally {
      if (
        generation === requestGeneration.current &&
        activeDetailId.current === id
      ) {
        setDetailLoading({ [id]: false });
      }
    }
  }, [client, t]);

  const handleDetailToggle = React.useCallback((id: string, expanded: boolean) => {
    if (expanded) {
      void fetchDetail(id);
      return;
    }
    if (activeDetailId.current === id) activeDetailId.current = null;
    setDetails({});
    setDetailLoading({});
    setDetailErrors({});
  }, [fetchDetail]);

  const refresh = React.useCallback(() => {
    requestGeneration.current += 1;
    activeDetailId.current = null;
    setPage({ items: [], nextCursor: null });
    setDetails({});
    setDetailLoading({});
    setDetailErrors({});
    setTableVersion((current) => current + 1);
    void fetchPage(null, false);
  }, [fetchPage]);

  const title = indexer
    ? t("indexerErrors.scopedTitle", { indexer: indexer.name })
    : t("indexerErrors.title");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] w-[calc(100%-1rem)] max-w-[96vw] flex-col overflow-hidden sm:max-w-6xl lg:max-w-[90rem]">
        <DialogHeader>
          <div className="flex items-center justify-between gap-4 pr-8">
            <DialogTitle>{title}</DialogTitle>
            <Button type="button" size="sm" variant="secondary" disabled={loading} className="gap-2" onClick={refresh}>
              <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
              {t("indexerErrors.refresh")}
            </Button>
          </div>
          <p className="text-left text-xs text-muted-foreground">
            {t("indexerErrors.retentionNotice")}
          </p>
        </DialogHeader>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {error ? (
            <div role="alert" className="mb-3 flex flex-wrap items-center gap-3 rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 text-sm text-[var(--scry-danger-text)]">
              <AlertTriangle className="h-4 w-4 shrink-0" />
              <span className="min-w-0 flex-1">{error}</span>
              <Button type="button" size="sm" variant="secondary" disabled={loading} onClick={() => void fetchPage(null, false)}>
                {t("indexerErrors.retry")}
              </Button>
            </div>
          ) : null}
          {loading && page.items.length === 0 ? (
            <div className="flex items-center gap-2 py-8 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("indexerErrors.loading")}
            </div>
          ) : error && page.items.length === 0 ? null : (
            <>
              <IndexerErrorTable
                key={tableVersion}
                items={page.items}
                showIndexer={!indexer}
                details={details}
                detailLoading={detailLoading}
                detailErrors={detailErrors}
                onToggleDetail={handleDetailToggle}
                emptyMessage={t("indexerErrors.empty")}
              />
              {!error && page.nextCursor ? (
                <div className="flex justify-center py-4">
                  <Button type="button" size="sm" variant="secondary" disabled={loading} onClick={() => void fetchPage(page.nextCursor, true)}>
                    {loading ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                    {t("indexerErrors.loadMore")}
                  </Button>
                </div>
              ) : page.items.length > 0 ? (
                <p className="py-4 text-center text-xs text-muted-foreground">
                  {t("indexerErrors.noMore")}
                </p>
              ) : null}
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
