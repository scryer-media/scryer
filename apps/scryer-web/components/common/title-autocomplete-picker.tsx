import * as React from "react";
import { Loader2, Search, X } from "lucide-react";
import { useClient } from "urql";

import { TitlePosterSlot } from "@/components/title-poster-slot";
import { Input } from "@/components/ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "@/components/ui/popover";
import { ActionTooltip } from "@/components/ui/tooltip";
import { sectionLabelForFacet } from "@/lib/facets/helpers";
import { titleAutocompleteSelectionQuery, titlesQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleRecord } from "@/lib/types";
import { cn } from "@/lib/utils";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";

const MIN_SEARCH_LENGTH = 2;
const SEARCH_DEBOUNCE_MS = 250;
const MAX_RESULTS = 10;

type TitleAutocompletePickerProps = {
  selectedTitle: TitleRecord | null;
  selectedTitleId: string | null;
  onSelectedTitleChange: (title: TitleRecord | null) => void;
  placeholder?: string;
  ariaLabel?: string;
  className?: string;
  disabled?: boolean;
};

function formatTitleSecondaryLine(title: TitleRecord, facetLabel: string): string {
  return title.year ? `${facetLabel} • ${title.year}` : facetLabel;
}

function formatSelectedTitleLabel(title: TitleRecord): string {
  return title.year ? `${title.name} (${title.year})` : title.name;
}

export function TitleAutocompletePicker({
  selectedTitle,
  selectedTitleId,
  onSelectedTitleChange,
  placeholder,
  ariaLabel,
  className,
  disabled = false,
}: TitleAutocompletePickerProps) {
  const client = useClient();
  const t = useTranslate();
  const anchorRef = React.useRef<HTMLDivElement | null>(null);
  const searchRequestId = React.useRef(0);
  const hydrateRequestId = React.useRef(0);
  const listboxId = React.useId();

  const [query, setQuery] = React.useState("");
  const [open, setOpen] = React.useState(false);
  const [results, setResults] = React.useState<TitleRecord[]>([]);
  const [activeResultIndex, setActiveResultIndex] = React.useState(-1);
  const [loading, setLoading] = React.useState(false);
  const [searchError, setSearchError] = React.useState<string | null>(null);
  const [searchPerformed, setSearchPerformed] = React.useState(false);
  const [menuWidth, setMenuWidth] = React.useState<number | undefined>(undefined);
  const [hydratedTitle, setHydratedTitle] = React.useState<TitleRecord | null>(null);
  const [hydrationResolved, setHydrationResolved] = React.useState(false);

  const normalizedSelectedTitleId = selectedTitleId?.trim() || null;
  const resolvedSelectedTitle =
    selectedTitle?.id === normalizedSelectedTitleId
      ? selectedTitle
      : hydratedTitle?.id === normalizedSelectedTitleId
        ? hydratedTitle
        : null;
  const unresolvedSelection =
    normalizedSelectedTitleId !== null && !resolvedSelectedTitle && hydrationResolved;

  const refreshMenuWidth = React.useCallback(() => {
    if (anchorRef.current) {
      setMenuWidth(anchorRef.current.offsetWidth);
    }
  }, []);

  React.useEffect(() => {
    refreshMenuWidth();
  }, [refreshMenuWidth, resolvedSelectedTitle, query]);

  React.useEffect(() => {
    setActiveResultIndex((current) => (current >= results.length ? -1 : current));
  }, [results]);

  React.useEffect(() => {
    if (!open || activeResultIndex < 0) return;
    document
      .getElementById(`${listboxId}-option-${activeResultIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeResultIndex, listboxId, open]);

  React.useEffect(() => {
    if (!normalizedSelectedTitleId) {
      hydrateRequestId.current += 1;
      setHydratedTitle(null);
      setHydrationResolved(false);
      return;
    }

    if (selectedTitle?.id === normalizedSelectedTitleId) {
      hydrateRequestId.current += 1;
      setHydratedTitle(selectedTitle);
      setHydrationResolved(true);
      return;
    }

    const requestId = ++hydrateRequestId.current;
    setHydratedTitle(null);
    setHydrationResolved(false);

    void client
      .query<{ title?: TitleRecord | null }>(titleAutocompleteSelectionQuery, {
        id: normalizedSelectedTitleId,
      })
      .toPromise()
      .then(({ data, error }) => {
        if (requestId !== hydrateRequestId.current) {
          return;
        }
        if (error) {
          setHydratedTitle(null);
          return;
        }
        setHydratedTitle((data?.title as TitleRecord | null) ?? null);
      })
      .finally(() => {
        if (requestId === hydrateRequestId.current) {
          setHydrationResolved(true);
        }
      });
  }, [client, normalizedSelectedTitleId, selectedTitle]);

  React.useEffect(() => {
    if (resolvedSelectedTitle || disabled) {
      searchRequestId.current += 1;
      setOpen(false);
      setResults((current) => (current.length === 0 ? current : []));
      setLoading(false);
      setSearchError(null);
      setSearchPerformed(false);
      return;
    }

    const trimmed = query.trim();
    if (trimmed.length < MIN_SEARCH_LENGTH) {
      searchRequestId.current += 1;
      setOpen(false);
      setResults((current) => (current.length === 0 ? current : []));
      setLoading(false);
      setSearchError(null);
      setSearchPerformed(false);
      return;
    }

    const requestId = ++searchRequestId.current;
    const handle = window.setTimeout(() => {
      setLoading(true);
      setSearchError(null);
      setSearchPerformed(true);

      void client
        .query<{ titles?: { items?: TitleRecord[] } }>(titlesQuery, {
          facet: null,
          query: trimmed,
          limit: MAX_RESULTS,
        })
        .toPromise()
        .then(({ data, error }) => {
          if (requestId !== searchRequestId.current) {
            return;
          }
          if (error) {
            throw error;
          }
          setResults(((data?.titles?.items ?? []) as TitleRecord[]).slice(0, MAX_RESULTS));
          setOpen(true);
          refreshMenuWidth();
        })
        .catch((error: unknown) => {
          if (requestId !== searchRequestId.current) {
            return;
          }
          setResults([]);
          setSearchError(error instanceof Error ? error.message : t("status.failedToLoad"));
          setOpen(true);
          refreshMenuWidth();
        })
        .finally(() => {
          if (requestId === searchRequestId.current) {
            setLoading(false);
          }
        });
    }, SEARCH_DEBOUNCE_MS);

    return () => {
      window.clearTimeout(handle);
    };
  }, [client, disabled, query, refreshMenuWidth, resolvedSelectedTitle, t]);

  const clearSelection = React.useCallback(() => {
    hydrateRequestId.current += 1;
    searchRequestId.current += 1;
    setQuery("");
    setOpen(false);
    setResults([]);
    setActiveResultIndex(-1);
    setSearchError(null);
    setSearchPerformed(false);
    setHydratedTitle(null);
    setHydrationResolved(false);
    onSelectedTitleChange(null);
  }, [onSelectedTitleChange]);

  const handleSelect = React.useCallback(
    (title: TitleRecord) => {
      setQuery("");
      setOpen(false);
      setResults([]);
      setActiveResultIndex(-1);
      setSearchError(null);
      setSearchPerformed(false);
      setHydratedTitle(title);
      setHydrationResolved(true);
      onSelectedTitleChange(title);
    },
    [onSelectedTitleChange],
  );

  const shouldShowEmptyState =
    !loading && searchPerformed && !searchError && results.length === 0;

  return (
    <div className={cn("space-y-2", className)}>
      {resolvedSelectedTitle || unresolvedSelection ? (
        <div className="flex min-h-10 flex-wrap items-center gap-2 rounded-md border border-border bg-card px-2 py-1.5">
          <div className="inline-flex items-center overflow-hidden rounded-full bg-accent text-accent-foreground">
            <span className="px-3 py-1 text-sm">
              {resolvedSelectedTitle
                ? formatSelectedTitleLabel(resolvedSelectedTitle)
                : t("label.unknown")}
            </span>
            <ActionTooltip content={t("label.clear")}>
              <button
                type="button"
                onClick={clearSelection}
                disabled={disabled}
                className="inline-flex h-7 w-8 items-center justify-center border-l border-accent-foreground/15 text-accent-foreground/80 transition-colors hover:bg-accent-foreground/10 hover:text-accent-foreground disabled:cursor-not-allowed disabled:opacity-50"
                aria-label={t("label.clear")}
              >
                <X className="h-4 w-4" />
              </button>
            </ActionTooltip>
          </div>
        </div>
      ) : (
        <Popover
          open={open}
          onOpenChange={(nextOpen) => {
            setOpen(nextOpen);
            if (!nextOpen) setActiveResultIndex(-1);
          }}
        >
          <PopoverAnchor asChild>
            <div ref={anchorRef} className="w-full">
              <Input
                value={query}
                onChange={(event) => {
                  const nextValue = event.target.value;
                  setQuery(nextValue);
                  setResults([]);
                  setActiveResultIndex(-1);
                  if (nextValue.trim().length < MIN_SEARCH_LENGTH) {
                    setOpen(false);
                  }
                }}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    setOpen(false);
                    setActiveResultIndex(-1);
                    return;
                  }
                  if (event.key === "Enter") {
                    const activeResult = results[activeResultIndex];
                    if (open && activeResult) {
                      event.preventDefault();
                      handleSelect(activeResult);
                    }
                    return;
                  }
                  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                  if (results.length === 0) return;
                  event.preventDefault();
                  setOpen(true);
                  setActiveResultIndex((current) => {
                    if (event.key === "ArrowDown") {
                      return current < 0 ? 0 : (current + 1) % results.length;
                    }
                    return current <= 0 ? results.length - 1 : current - 1;
                  });
                }}
                onFocus={() => {
                  if (query.trim().length >= MIN_SEARCH_LENGTH) {
                    setOpen(true);
                    refreshMenuWidth();
                  }
                }}
                placeholder={placeholder ?? t("queue.assignTitlePlaceholder")}
                aria-label={ariaLabel}
                role="combobox"
                aria-autocomplete="list"
                aria-expanded={open}
                aria-controls={open ? listboxId : undefined}
                aria-activedescendant={
                  activeResultIndex >= 0
                    ? `${listboxId}-option-${activeResultIndex}`
                    : undefined
                }
                disabled={disabled}
              />
            </div>
          </PopoverAnchor>
          <PopoverContent
            align="start"
            className="p-0"
            sideOffset={6}
            style={menuWidth ? { width: menuWidth } : undefined}
            onOpenAutoFocus={(event) => event.preventDefault()}
          >
            <div id={listboxId} role="listbox" className="max-h-80 overflow-y-auto p-2">
              {loading ? (
                <div className="flex items-center gap-2 px-2 py-3 text-sm text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span>{t("label.loading")}</span>
                </div>
              ) : null}
              {searchError ? (
                <div className="px-2 py-3 text-sm text-[var(--scry-danger-text-soft)]">{searchError}</div>
              ) : null}
              {shouldShowEmptyState ? (
                <div className="px-2 py-3 text-sm text-muted-foreground">
                  {t("queue.assignTitleEmpty")}
                </div>
              ) : null}
              {!loading && !searchError && results.length > 0 ? (
                <div className="space-y-1">
                  {results.map((title, index) => {
                    const facetLabel = sectionLabelForFacet(t, title.facet);
                    const posterUrl = selectPosterVariantUrl(
                      title.posterUrl,
                      "w250",
                    );
                    return (
                      <button
                        key={title.id}
                        id={`${listboxId}-option-${index}`}
                        type="button"
                        role="option"
                        aria-selected={index === activeResultIndex}
                        onClick={() => handleSelect(title)}
                        onMouseMove={() => setActiveResultIndex(index)}
                        className={cn(
                          "flex w-full items-center gap-3 rounded-md px-2 py-2 text-left transition-colors hover:bg-accent",
                          index === activeResultIndex && "bg-accent",
                        )}
                      >
                        <TitlePosterSlot
                          src={posterUrl}
                          metadataFetchedAt={title.metadataFetchedAt}
                          createdAt={title.createdAt}
                          alt={title.name}
                          className="h-12 w-8 shrink-0 rounded-sm object-cover"
                          placeholderClassName="h-12 w-8 shrink-0 rounded-sm"
                          emptyLabel={title.name}
                          fallbackTitle={title.name}
                          fallbackTone={title.facet}
                          fallbackShowText={false}
                        />
                        <div className="min-w-0 flex-1">
                          <div className="truncate font-medium text-foreground">{title.name}</div>
                          <div className="truncate text-xs text-muted-foreground">
                            {formatTitleSecondaryLine(title, facetLabel)}
                          </div>
                        </div>
                        <Search className="h-4 w-4 shrink-0 text-muted-foreground/60" />
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </div>
          </PopoverContent>
        </Popover>
      )}
    </div>
  );
}
