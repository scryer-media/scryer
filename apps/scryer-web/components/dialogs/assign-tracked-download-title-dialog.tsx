import * as React from "react";
import { CheckCircle2, Loader2, Search } from "lucide-react";
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
    case "tv":
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
    setQuery(queueItem.titleName ?? "");
  }, [open, queueItem]);

  React.useEffect(() => {
    if (!open || !queueItem) {
      return undefined;
    }

    const searchTerm = query.trim() || queueItem.titleName || "";
    const timeoutId = window.setTimeout(() => {
      setLoading(true);
      setError(null);
      client
        .query(titlesQuery, {
          facet: queueItem.facet,
          query: searchTerm,
        })
        .toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) throw queryError;
          setResults((data?.titles ?? []) as TitleSearchResult[]);
        })
        .catch((err: unknown) => {
          setResults([]);
          setError(err instanceof Error ? err.message : t("status.failedToLoad"));
        })
        .finally(() => setLoading(false));
    }, 180);

    return () => window.clearTimeout(timeoutId);
  }, [client, open, query, queueItem, t]);

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
      <CommandInput
        value={query}
        onValueChange={setQuery}
        placeholder={t("queue.assignTitlePlaceholder")}
      />
      <CommandList>
        {error ? <div className="px-4 py-3 text-sm text-rose-400">{error}</div> : null}
        {loading ? (
          <div className="flex items-center gap-2 px-4 py-3 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        ) : null}
        <CommandEmpty>{t("queue.assignTitleEmpty")}</CommandEmpty>
        <CommandGroup heading={t("queue.assignTitleResults") }>
          {results.map((title) => {
            const assigning = assigningId === title.id;
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
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate font-medium text-foreground">
                    {title.name}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {facetLabel(title.facet)}
                    {title.year ? ` • ${title.year}` : ""}
                  </span>
                </div>
                {assigning ? (
                  <Loader2 className="ml-auto h-4 w-4 animate-spin" />
                ) : (
                  <CheckCircle2 className="ml-auto h-4 w-4 text-muted-foreground/50" />
                )}
              </CommandItem>
            );
          })}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}