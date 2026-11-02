import { useQuery } from "urql";
import { HistoryEventTable } from "@/components/common/history-event-table";
import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { recycledItemHistoryQuery } from "@/lib/graphql/queries";
import type { TitleHistoryEvent } from "@/lib/types";

/** Mounted on expansion: each row loads only its own correlated history. */
export function RecycleBinHistory({ id }: { id: string }) {
  const t = useTranslate();
  const [{ data, error, fetching }, retry] = useQuery<{
    recycledItemHistory: { items: TitleHistoryEvent[]; truncated: boolean };
  }>({
    query: recycledItemHistoryQuery,
    variables: { id },
    requestPolicy: "network-only",
  });
  const summary = data?.recycledItemHistory;

  return (
    <div
      className="min-w-0 w-full space-y-3"
      role="region"
      aria-label={t("settings.recycleBinHistory")}
      aria-busy={fetching}
    >
      {error ? (
        <div
          role="alert"
          className="flex min-w-0 flex-wrap items-center gap-2 p-3 text-sm text-destructive [overflow-wrap:anywhere]"
        >
          <span className="min-w-0">
            {userFacingGraphQlErrorMessage(error, t("history.loadError"), t)}
          </span>
          <Button
            size="sm"
            variant="outline"
            disabled={fetching}
            onClick={() => retry({ requestPolicy: "network-only" })}
          >
            {t("history.retry")}
          </Button>
        </div>
      ) : null}
      {fetching && !summary ? (
        <div
          role="status"
          className="flex min-h-24 items-center justify-center gap-2 text-sm text-muted-foreground"
        >
          <LoadingMark className="h-4 w-4" />
          {t("label.loading")}
        </div>
      ) : summary ? (
        <>
          <div className="overflow-hidden bg-background [&>p]:px-3">
            <HistoryEventTable
              events={summary.items}
              showActor
              emptyMessage={t("settings.recycleBinHistoryEmpty")}
            />
          </div>
          {summary.truncated ? (
            <p className="px-3 pb-3 text-xs text-muted-foreground">
              {t("settings.recycleBinHistoryTruncated")}
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
