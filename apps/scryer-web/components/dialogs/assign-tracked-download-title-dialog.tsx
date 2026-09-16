import * as React from "react";
import { CheckCircle2, Search } from "lucide-react";
import { useClient } from "urql";

import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { useTranslate } from "@/lib/context/translate-context";
import { titlesQuery } from "@/lib/graphql/queries";
import type { DownloadQueueItem } from "@/lib/types";
import { selectorId } from "@/lib/utils/dom-ids";
import { LoadingMark } from "@/components/common/loading-mark";

type TitleSearchResult = {
  id: string;
  name: string;
  facet: string;
  year?: number | null;
};

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  queueItem: DownloadQueueItem | null;
  onAssign: (queueItem: DownloadQueueItem, titleId: string) => Promise<void>;
};

function facetLabel(facet: string | null | undefined): string {
  switch ((facet ?? "").toLowerCase()) {
    case "movie":
      return "Movie";
    case "series":
      return "Series";
    case "anime":
      return "Anime";
    default:
      return "Title";
  }
}

export function AssignTrackedDownloadTitleDialog({
  open,
  onOpenChange,
  queueItem,
  onAssign,
}: Props) {
  const client = useClient();
  const t = useTranslate();
  const [query, setQuery] = React.useState("");
  const [results, setResults] = React.useState<TitleSearchResult[]>([]);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [assigningId, setAssigningId] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open || !queueItem) {
      setQuery("");
      setResults([]);
      setError(null);
      setAssigningId(null);
      return;
    }
    setQuery("");
  }, [open, queueItem]);

  React.useEffect(() => {
    if (!open || !queueItem) {
      return undefined;
    }

    let cancelled = false;

    setLoading(true);
    setError(null);
    client
      .query(titlesQuery, { limit: 300 })
      .toPromise()
      .then(({ data, error: queryError }) => {
        if (cancelled) {
          return;
        }
        if (queryError) throw queryError;
        setResults((data?.titles?.items ?? []) as TitleSearchResult[]);
      })
      .catch((err: unknown) => {
        if (cancelled) {
          return;
        }
        setResults([]);
        setError(err instanceof Error ? err.message : t("status.failedToLoad"));
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [client, open, queueItem, t]);

  const handleSelect = React.useCallback(
    async (titleId: string) => {
      if (!queueItem) {
        return;
      }
      setAssigningId(titleId);
      try {
        await onAssign(queueItem, titleId);
        onOpenChange(false);
      } finally {
        setAssigningId(null);
      }
    },
    [onAssign, onOpenChange, queueItem],
  );

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("queue.assignTitleTitle")}
      description={t("queue.assignTitleDescription")}
      className="sm:max-w-2xl"
    >
      <div id="activity-assign-title-dialog">
      <CommandInput
        id="activity-assign-title-search"
        value={query}
        onValueChange={setQuery}
        placeholder={t("queue.assignTitlePlaceholder")}
      />
      <CommandList id="activity-assign-title-list">
        {error ? (
          <div id="activity-assign-title-error" className="px-4 py-3 text-sm text-[var(--scry-danger-text-soft)]">
            {error}
          </div>
        ) : null}
        {loading ? (
          <div
            id="activity-assign-title-loading"
            className="flex items-center gap-2 px-4 py-3 text-sm text-muted-foreground"
          >
            <LoadingMark className="h-4 w-4" />
            <span>{t("label.loading")}</span>
          </div>
        ) : null}
        <CommandEmpty>{t("queue.assignTitleEmpty")}</CommandEmpty>
        <CommandGroup heading={t("queue.assignTitleResults") }>
          {results.map((title) => {
            const assigning = assigningId === title.id;
            const optionId = selectorId(
              "activity-assign-title-item",
              title.name,
              title.year ?? "",
              title.facet,
            );
            return (
              <CommandItem
                key={title.id}
                value={`${title.name} ${title.year ?? ""} ${title.facet}`}
                onSelect={() => {
                  void handleSelect(title.id);
                }}
                disabled={assigningId !== null}
              >
                <Search className="h-4 w-4 text-muted-foreground" />
                <div
                  id={optionId}
                  className="flex min-w-0 flex-1 flex-col gap-0.5"
                >
                  <span className="truncate font-medium text-foreground">
                    {title.name}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {facetLabel(title.facet)}
                    {title.year ? ` • ${title.year}` : ""}
                  </span>
                </div>
                {assigning ? (
                  <LoadingMark className="ml-auto h-4 w-4" />
                ) : (
                  <CheckCircle2 className="ml-auto h-4 w-4 text-muted-foreground/50" />
                )}
              </CommandItem>
            );
          })}
        </CommandGroup>
      </CommandList>
      </div>
    </CommandDialog>
  );
}
