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
    <div className="min-w-0 space-y-3 p-2" aria-busy={fetching}>
      <p className="text-sm font-medium">{t("settings.recycleBinHistory")}</p>
      <p className="text-xs text-muted-foreground">
        {t("settings.recycleBinHistoryHelp")}
      </p>
      {error ? (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-2 text-sm text-destructive"
        >
          <span>
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
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <LoadingMark className="h-4 w-4" />
          {t("label.loading")}
        </div>
      ) : summary ? (
        <>
          <HistoryEventTable
            events={summary.items}
            showActor
            emptyMessage={t("settings.recycleBinHistoryEmpty")}
          />
          {summary.truncated ? (
            <p className="text-xs text-muted-foreground">
              {t("settings.recycleBinHistoryTruncated")}
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
