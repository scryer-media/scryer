import * as React from "react";
import { useClient } from "urql";

import { LoadingMark } from "@/components/common/loading-mark";
import { TitlePoster } from "@/components/title-poster";
import { Input } from "@/components/ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "@/components/ui/popover";
import { useTranslate } from "@/lib/context/translate-context";
import { searchMetadataQuery } from "@/lib/graphql/queries";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { isAbortError, makeAbortableFetch } from "@/lib/graphql/urql-client";
import type { Facet } from "@/lib/types/titles";
import { cn } from "@/lib/utils";
import { exclusionFieldsFromMetadataResult } from "@/lib/utils/lists";

const MIN_SEARCH_LENGTH = 2;
const SEARCH_DEBOUNCE_MS = 250;
const MAX_RESULTS = 8;

type ExclusionTitleSearchProps = {
  id: string;
  value: string;
  /** The kind of title searched for. */
  kind: Facet;
  onChange: (value: string) => void;
  onPick: (result: MetadataTvdbSearchItem) => void;
};

/**
 * The exclusion form's title field. Typing searches the metadata service for
 * titles of the chosen kind; picking one hands it back so the form can take
 * its name, year and ids. A title typed without picking stays as typed.
 */
export function ExclusionTitleSearch({ id, value, kind, onChange, onPick }: ExclusionTitleSearchProps) {
  const client = useClient();
  const t = useTranslate();
  const listboxId = React.useId();
  // Only text the reader typed is searched, never a picked title's name.
  const [typed, setTyped] = React.useState(false);
  const [open, setOpen] = React.useState(false);
  const [results, setResults] = React.useState<MetadataTvdbSearchItem[]>([]);
  const [activeIndex, setActiveIndex] = React.useState(-1);
  const [loading, setLoading] = React.useState(false);
  const [failed, setFailed] = React.useState(false);
  const term = typed ? value.trim() : "";
  const searchable = term.length >= MIN_SEARCH_LENGTH;

  React.useEffect(() => {
    if (!searchable) {
      // The results stay as they are while the list closes.
      setOpen(false);
      setActiveIndex(-1);
      return undefined;
    }

    const abortController = new AbortController();
    let active = true;
    setResults([]);
    setActiveIndex(-1);
    setFailed(false);
    setLoading(true);
    setOpen(true);
    const handle = window.setTimeout(() => {
      client
        .query(
          searchMetadataQuery,
          { query: term, type: kind, limit: MAX_RESULTS },
          { fetch: makeAbortableFetch(abortController.signal) },
        )
        .toPromise()
        .then(({ data, error }) => {
          if (error) throw error;
          if (active) setResults((data?.searchMetadata ?? []) as MetadataTvdbSearchItem[]);
        })
        .catch((error: unknown) => {
          if (active && !isAbortError(error)) setFailed(true);
        })
        .finally(() => {
          if (active) setLoading(false);
        });
    }, SEARCH_DEBOUNCE_MS);

    return () => {
      active = false;
      window.clearTimeout(handle);
      abortController.abort();
    };
  }, [client, kind, searchable, term]);

  React.useEffect(() => {
    if (!open || activeIndex < 0) return;
    document.getElementById(`${listboxId}-option-${activeIndex}`)?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, listboxId, open]);

  const pick = (result: MetadataTvdbSearchItem) => {
    setTyped(false);
    setOpen(false);
    onPick(result);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverAnchor asChild>
        <Input
          id={id}
          value={value}
          placeholder={t("lists.exclusions.titlePlaceholder")}
          autoComplete="off"
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={open}
          aria-controls={open ? listboxId : undefined}
          aria-activedescendant={activeIndex >= 0 ? `${listboxId}-option-${activeIndex}` : undefined}
          onChange={(event) => {
            setTyped(true);
            onChange(event.target.value);
          }}
          onFocus={() => {
            if (searchable) setOpen(true);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setOpen(false);
              return;
            }
            if (event.key === "Enter") {
              const result = open ? results[activeIndex] : undefined;
              if (result) {
                event.preventDefault();
                pick(result);
              }
              return;
            }
            if ((event.key !== "ArrowDown" && event.key !== "ArrowUp") || results.length === 0) return;
            event.preventDefault();
            setOpen(true);
            setActiveIndex((current) =>
              event.key === "ArrowDown"
                ? (current + 1) % results.length
                : current <= 0
                  ? results.length - 1
                  : current - 1,
            );
          }}
        />
      </PopoverAnchor>
      <PopoverContent
        align="start"
        sideOffset={6}
        className="w-[var(--radix-popover-trigger-width)] p-0"
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <div id={listboxId} role="listbox" className="max-h-80 overflow-y-auto p-2">
          {loading ? (
            <div className="flex items-center gap-2 px-2 py-3 text-sm text-muted-foreground">
              <LoadingMark className="h-4 w-4" />
              {t("title.fixMatchSearching")}
            </div>
          ) : failed ? (
            <div className="px-2 py-3 text-sm text-[var(--scry-danger-text)]">{t("status.failedToLoad")}</div>
          ) : results.length === 0 ? (
            <div id="list-exclusion-title-no-results" className="px-2 py-3 text-sm text-muted-foreground">
              {t("title.fixMatchNoResults")}
            </div>
          ) : (
            results.map((result, index) => {
              const fields = exclusionFieldsFromMetadataResult(result, kind);
              return (
                <button
                  key={fields.ids || `${result.name}-${index}`}
                  id={`${listboxId}-option-${index}`}
                  type="button"
                  role="option"
                  data-ui="list-exclusion-title-result"
                  aria-selected={index === activeIndex}
                  onClick={() => pick(result)}
                  onMouseMove={() => setActiveIndex(index)}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-md px-2 py-2 text-left transition-colors hover:bg-accent",
                    index === activeIndex && "bg-accent",
                  )}
                >
                  <span className="h-12 w-8 flex-none overflow-hidden rounded-sm bg-muted">
                    <TitlePoster src={result.posterUrl} alt="" className="h-full w-full object-cover" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium text-foreground">
                      {result.name}
                      {result.year ? <span className="text-muted-foreground"> ({result.year})</span> : null}
                    </span>
                    <span className="block truncate font-[var(--font-code)] text-xs text-muted-foreground">
                      {fields.ids}
                    </span>
                  </span>
                </button>
              );
            })
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
