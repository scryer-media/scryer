import * as React from "react";
import { Loader2, Search } from "lucide-react";
import { useClient } from "urql";

import { TitlePoster } from "@/components/title-poster";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useTranslate } from "@/lib/context/translate-context";
import { fixTitleMatchMutation } from "@/lib/graphql/mutations";
import { searchMetadataQuery } from "@/lib/graphql/queries";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";

type FixableTitle = {
  id: string;
  name: string;
  facet: string;
  externalIds: { source: string; value: string }[];
};

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: FixableTitle | null;
  onFixed?: (warnings: string[]) => Promise<void> | void;
};

function metadataTypeForFacet(facet: string | null | undefined): "movie" | "series" {
  return (facet ?? "").toLowerCase() === "movie" ? "movie" : "series";
}

function currentTvdbId(title: FixableTitle | null): string | null {
  return (
    title?.externalIds.find((entry) => entry.source.toLowerCase() === "tvdb")?.value?.trim() ||
    null
  );
}

export function FixTitleMatchDialog({
  open,
  onOpenChange,
  title,
  onFixed,
}: Props) {
  const client = useClient();
  const t = useTranslate();
  const [query, setQuery] = React.useState("");
  const [results, setResults] = React.useState<MetadataTvdbSearchItem[]>([]);
  const [selectedTvdbId, setSelectedTvdbId] = React.useState<string | null>(null);
  const [searching, setSearching] = React.useState(false);
  const [applying, setApplying] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open || !title) {
      setQuery("");
      setResults([]);
      setSelectedTvdbId(null);
      setError(null);
      return;
    }

    setQuery(title.name);
    setResults([]);
    setSelectedTvdbId(null);
    setError(null);
  }, [open, title]);

  React.useEffect(() => {
    if (!open || !title) {
      return undefined;
    }

    const trimmed = query.trim();
    if (!trimmed) {
      setResults([]);
      setSelectedTvdbId(null);
      setSearching(false);
      return undefined;
    }

    const timeoutId = window.setTimeout(() => {
      setSearching(true);
      setError(null);
      client
        .query(searchMetadataQuery, {
          query: trimmed,
          type: metadataTypeForFacet(title.facet),
          limit: 8,
        })
        .toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) throw queryError;
          const items = (data?.searchMetadata ?? []) as MetadataTvdbSearchItem[];
          setResults(items);
          setSelectedTvdbId((current) =>
            current && items.some((item) => String(item.tvdbId) === current)
              ? current
              : items[0]
                ? String(items[0].tvdbId)
                : null,
          );
        })
        .catch((err: unknown) => {
          setResults([]);
          setSelectedTvdbId(null);
          setError(err instanceof Error ? err.message : t("title.fixMatchSearchFailed"));
        })
        .finally(() => setSearching(false));
    }, 220);

    return () => window.clearTimeout(timeoutId);
  }, [client, open, query, t, title]);

  const handleApply = React.useCallback(async () => {
    if (!title || !selectedTvdbId) return;
    setApplying(true);
    setError(null);
    try {
      const { data, error: mutationError } = await client
        .mutation(fixTitleMatchMutation, {
          input: {
            titleId: title.id,
            tvdbId: selectedTvdbId,
          },
        })
        .toPromise();
      if (mutationError) throw mutationError;
      const warnings = (data?.fixTitleMatch?.warnings ?? []) as string[];
      await onFixed?.(warnings);
      onOpenChange(false);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : t("title.fixMatchApplyFailed"));
    } finally {
      setApplying(false);
    }
  }, [client, onFixed, onOpenChange, selectedTvdbId, t, title]);

  const existingTvdbId = currentTvdbId(title);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("title.fixMatchDialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("title.fixMatchDialogDescription", {
              name: title?.name ?? t("title.fixMatchUnnamed"),
            })}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("title.fixMatchSearchPlaceholder")}
              disabled={applying}
            />
            <div className="text-xs text-muted-foreground sm:min-w-[180px]">
              {t("title.fixMatchCurrentTvdbId")}:{" "}
              <span className="font-mono">{existingTvdbId ?? t("title.fixMatchCurrentTvdbNone")}</span>
            </div>
          </div>

          {searching ? (
            <div className="flex items-center gap-2 rounded-md border border-border px-3 py-6 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("title.fixMatchSearching")}
            </div>
          ) : null}

          {error ? (
            <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-300">
              {error}
            </div>
          ) : null}

          {!searching && !error && query.trim() && results.length === 0 ? (
            <div className="rounded-md border border-border px-3 py-6 text-sm text-muted-foreground">
              {t("title.fixMatchNoResults")}
            </div>
          ) : null}

          <div className="max-h-[420px] space-y-3 overflow-y-auto pr-1">
            {results.map((result) => {
              const selected = String(result.tvdbId) === selectedTvdbId;
              return (
                <button
                  key={`${result.tvdbId}-${result.name}`}
                  type="button"
                  className={`flex w-full gap-3 rounded-lg border p-3 text-left transition-colors ${
                    selected
                      ? "border-primary bg-primary/5"
                      : "border-border bg-card/40 hover:bg-muted/35"
                  }`}
                  onClick={() => setSelectedTvdbId(String(result.tvdbId))}
                  disabled={applying}
                >
                  <div className="h-24 w-16 flex-none overflow-hidden rounded-md border border-border bg-muted">
                    {result.posterUrl ? (
                      <TitlePoster src={result.posterUrl} alt={result.name} />
                    ) : null}
                  </div>
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium text-foreground">{result.name}</span>
                      {result.year ? (
                        <span className="text-xs text-muted-foreground">{result.year}</span>
                      ) : null}
                      <span className="text-xs text-muted-foreground">
                        TVDB {result.tvdbId}
                      </span>
                    </div>
                    {result.status ? (
                      <div className="text-xs text-muted-foreground">{result.status}</div>
                    ) : null}
                    {result.overview ? (
                      <p className="line-clamp-3 text-sm text-muted-foreground">
                        {result.overview}
                      </p>
                    ) : null}
                  </div>
                  <div className="flex items-start">
                    <Button
                      type="button"
                      variant={selected ? "secondary" : "outline"}
                      size="sm"
                      className="gap-2"
                      disabled={applying}
                    >
                      <Search className="h-4 w-4" />
                      {selected ? t("title.fixMatchSelected") : t("title.fixMatchChoose")}
                    </Button>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={applying}>
            {t("label.cancel")}
          </Button>
          <Button type="button" onClick={() => void handleApply()} disabled={!selectedTvdbId || applying}>
            {applying ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                {t("title.fixMatchApplying")}
              </>
            ) : (
              t("title.fixMatchApply")
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
