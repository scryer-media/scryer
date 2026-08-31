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
import { isAbortError, makeAbortableFetch } from "@/lib/graphql/urql-client";
import {
  buildFixTitleMatchSearchVariables,
  fixTitleMatchDialogIdentity,
} from "@/lib/fix-title-match";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { selectorId } from "@/lib/utils/dom-ids";

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

function currentTvdbId(title: FixableTitle | null): string | null {
  return (
    title?.externalIds.find((entry) => entry.source.toLowerCase() === "tvdb")?.value?.trim() ||
    null
  );
}

function metadataResultKey(result: MetadataTvdbSearchItem): string {
  return result.smgId != null
    ? `smg:${result.smgId}`
    : result.tvdbId.trim()
      ? `tvdb:${result.tvdbId}`
      : `name:${result.name}`;
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
  const [selectedResultKey, setSelectedResultKey] = React.useState<string | null>(null);
  const [searching, setSearching] = React.useState(false);
  const [applying, setApplying] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const titleIdentity = fixTitleMatchDialogIdentity(title);
  const titleName = title?.name ?? "";

  React.useEffect(() => {
    if (!open || titleIdentity === null) {
      setQuery("");
      setResults([]);
      setSelectedResultKey(null);
      setError(null);
      return;
    }

    setQuery(titleName);
    setResults([]);
    setSelectedResultKey(null);
    setError(null);
  }, [open, titleIdentity, titleName]);

  React.useEffect(() => {
    if (!open || !title) {
      setSearching(false);
      return undefined;
    }

    const trimmed = query.trim();
    if (!trimmed) {
      setResults([]);
      setSelectedResultKey(null);
      setSearching(false);
      return undefined;
    }

    const abortController = new AbortController();
    const abortableFetch = makeAbortableFetch(abortController.signal);
    let active = true;
    const timeoutId = window.setTimeout(() => {
      setSearching(true);
      setError(null);
      client
        .query(
          searchMetadataQuery,
          buildFixTitleMatchSearchVariables(trimmed, title.facet),
          { fetch: abortableFetch },
        )
        .toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) throw queryError;
          if (!active) return;
          const items = (data?.searchMetadata ?? []) as MetadataTvdbSearchItem[];
          setResults(items);
          setSelectedResultKey((current) =>
            current && items.some((item) => metadataResultKey(item) === current)
              ? current
              : items[0]
                ? metadataResultKey(items[0])
                : null,
          );
        })
        .catch((err: unknown) => {
          if (!active || isAbortError(err)) {
            return;
          }
          setResults([]);
          setSelectedResultKey(null);
          setError(err instanceof Error ? err.message : t("title.fixMatchSearchFailed"));
        })
        .finally(() => {
          if (active) {
            setSearching(false);
          }
        });
    }, 220);

    return () => {
      active = false;
      window.clearTimeout(timeoutId);
      abortController.abort();
    };
  }, [client, open, query, t, title]);

  const handleApply = React.useCallback(async () => {
    const result = results.find((item) => metadataResultKey(item) === selectedResultKey);
    const tvdbId = result?.tvdbId.trim();
    const isMovie = title?.facet.toLowerCase() === "movie";
    if (!title || !result || (isMovie ? result.smgId == null && !tvdbId : !tvdbId)) return;
    setApplying(true);
    setError(null);
    try {
      const { data, error: mutationError } = await client
        .mutation(fixTitleMatchMutation, {
          input: {
            titleId: title.id,
            ...(isMovie
              ? { smgId: result.smgId ?? undefined, tvdbId: tvdbId || undefined }
              : { tvdbId }),
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
  }, [client, onFixed, onOpenChange, results, selectedResultKey, t, title]);

  const existingTvdbId = currentTvdbId(title);
  const selectedResult = results.find((item) => metadataResultKey(item) === selectedResultKey);
  const canApply = Boolean(
    selectedResult &&
      (title?.facet.toLowerCase() === "movie"
        ? selectedResult.smgId != null || selectedResult.tvdbId.trim()
        : selectedResult.tvdbId.trim()),
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent id="fix-title-match-dialog" className="sm:max-w-3xl">
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
              id="fix-title-match-search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("title.fixMatchSearchPlaceholder")}
              disabled={applying}
            />
            <div className="text-xs text-muted-foreground sm:min-w-[180px]">
              {t("title.fixMatchCurrentTvdbId")}:{" "}
              <span className="font-[var(--font-code)]">{existingTvdbId ?? t("title.fixMatchCurrentTvdbNone")}</span>
            </div>
          </div>

          {searching ? (
            <div className="flex items-center gap-2 rounded-md border border-border px-3 py-6 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("title.fixMatchSearching")}
            </div>
          ) : null}

          {error ? (
            <div
              id="fix-title-match-error"
              className="rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]"
            >
              {error}
            </div>
          ) : null}

          {!searching && !error && query.trim() && results.length === 0 ? (
            <div
              id="fix-title-match-no-results"
              className="rounded-md border border-border px-3 py-6 text-sm text-muted-foreground"
            >
              {t("title.fixMatchNoResults")}
            </div>
          ) : null}

          <div className="max-h-[420px] space-y-3 overflow-y-auto pr-1">
            {results.map((result) => {
              const resultKey = metadataResultKey(result);
              const selected = resultKey === selectedResultKey;
              return (
                <button
                  key={resultKey}
                  id={selectorId("fix-title-match-result", resultKey)}
                  type="button"
                  className={`flex w-full gap-3 rounded-lg border p-3 text-left transition-colors ${
                    selected
                      ? "border-primary bg-primary/5"
                      : "border-border bg-card/40 hover:bg-muted/35"
                  }`}
                  onClick={() => setSelectedResultKey(resultKey)}
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
                        {result.smgId != null ? `SMG ${result.smgId}` : `TVDB ${result.tvdbId}`}
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
                      id={selectorId("fix-title-match-choose", resultKey)}
                      type="button"
                      variant="primary"
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
          <Button
            id="fix-title-match-cancel"
            type="button"
            variant="ghost"
            onClick={() => onOpenChange(false)}
            disabled={applying}
          >
            {t("label.cancel")}
          </Button>
          <Button
            id="fix-title-match-apply"
            type="button"
            onClick={() => void handleApply()}
            disabled={!canApply || applying}
          >
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
