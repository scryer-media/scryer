import * as React from "react";
import { ChevronDown, Loader2, Search } from "lucide-react";
import { Link } from "react-router";

import { TitlePoster } from "@/components/title-poster";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  PendingImportBindingEpisode,
  PendingImportBindingPreview,
  PendingImportItem,
} from "@/lib/types";

type MetadataSearchResult = {
  tvdbId: string;
  smgId?: number | null;
  tmdbId?: number | null;
  externalIds?: Array<{ source: string; value: string }>;
  name: string;
  imdbId: string | null;
  slug: string | null;
  type: string | null;
  year: number | null;
  status: string | null;
  overview: string | null;
  popularity: number | null;
  posterUrl: string | null;
  language: string | null;
  runtimeMinutes: number | null;
  sortTitle: string | null;
  existingTitleId?: string | null;
};

export type PendingImportCardProps = {
  item: PendingImportItem;
  isActive: boolean;
  isResolving: boolean;
  isBusy: boolean;
  libraryLabel: string;
  knownTitleHref: string | null;
  knownTitleLabel: string;
  summary: string;
  bindingLoading: boolean;
  bindingError: string | null;
  bindingPreview: PendingImportBindingPreview | null;
  bindingGroups: [string, PendingImportBindingEpisode[]][];
  expandedBindingSeasonKeys: string[];
  selectedEpisodeIds: string[];
  searchQuery: string;
  searchYear: number | null;
  searchResults: MetadataSearchResult[];
  searchError: string | null;
  searching: boolean;
  formatBindingEpisodeDisplay: (
    episode: PendingImportBindingEpisode,
  ) => { key: string | null; label: string; showSeparateLabel: boolean };
  onOpenSearch: (item: PendingImportItem) => void;
  onRequestIgnore: (item: PendingImportItem) => void;
  onBind: () => void;
  onResolve: (
    result: MetadataSearchResult,
    options?: { attachToExistingTitle?: boolean },
  ) => void;
  onToggleEpisodeSelection: (episodeId: string, checked: boolean) => void;
  onSetSelectedEpisodeIds: React.Dispatch<React.SetStateAction<string[]>>;
  onSetExpandedBindingSeasonKeys: React.Dispatch<React.SetStateAction<string[]>>;
  onSearchQueryChange: (value: string) => void;
  onSearchYearChange: (value: number | null) => void;
  onClearActiveItem: () => void;
};

export const PendingImportCard = React.memo(function PendingImportCard({
  item,
  isActive,
  isResolving,
  isBusy,
  libraryLabel,
  knownTitleHref,
  knownTitleLabel,
  summary,
  bindingLoading,
  bindingError,
  bindingPreview,
  bindingGroups,
  expandedBindingSeasonKeys,
  selectedEpisodeIds,
  searchQuery,
  searchYear,
  searchResults,
  searchError,
  searching,
  formatBindingEpisodeDisplay,
  onOpenSearch,
  onRequestIgnore,
  onBind,
  onResolve,
  onToggleEpisodeSelection,
  onSetSelectedEpisodeIds,
  onSetExpandedBindingSeasonKeys,
  onSearchQueryChange,
  onSearchYearChange,
  onClearActiveItem,
}: PendingImportCardProps) {
  const t = useTranslate();
  const isOwnershipConflict = item.reason === "title_already_owns_another_folder";
  const canSearchOrBind =
    !isOwnershipConflict && !(item.titleId && item.facet === "MOVIE");

  return (
    <Card className="border-border/80 bg-card/60">
      <CardHeader className="space-y-2">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="space-y-1">
            <CardTitle className="text-base">{item.displayName}</CardTitle>
            <p className="text-sm text-muted-foreground">{summary}</p>
            <p className="text-xs text-muted-foreground">{t("pendingImports.library")} {libraryLabel}</p>
          </div>
          <div className="flex flex-col gap-2 sm:flex-row">
            {canSearchOrBind ? (
              <Button
                type="button"
                size="sm"
                variant={isActive ? "secondary" : "default"}
                onClick={() => onOpenSearch(item)}
                disabled={isBusy}
              >
                {item.titleId ? null : <Search className="mr-2 h-4 w-4" />}
                {item.titleId ? t("pendingImports.bindEpisodes") : t("pendingImports.searchAction")}
              </Button>
            ) : null}
            {item.status === "PENDING" ? (
              <Button
                type="button"
                size="sm"
                variant="destructive"
                onClick={() => onRequestIgnore(item)}
                disabled={isBusy}
              >
                {t("pendingImports.ignore")}
              </Button>
            ) : null}
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-3 text-sm">
        <div>
          <span className="font-medium text-foreground">{t("pendingImports.path")}:</span>{" "}
          <span className="break-all font-[var(--font-code)] text-muted-foreground">{item.path}</span>
        </div>
        {item.titleId ? (
          <div>
            <span className="font-medium text-foreground">{t("pendingImports.knownTitle")}</span>{" "}
            {knownTitleHref ? (
              <Link
                to={knownTitleHref}
                className="break-all text-foreground underline-offset-4 hover:underline"
              >
                {knownTitleLabel}
              </Link>
            ) : (
              <span className="break-all text-muted-foreground">{knownTitleLabel}</span>
            )}
          </div>
        ) : null}
        {isOwnershipConflict ? (
          <p
            className="rounded-lg border border-border/80 bg-background/60 p-3 text-sm text-muted-foreground"
            data-ui="pending-import-ownership-conflict-help"
          >
            {t("pendingImports.ownershipConflictHelp")}
          </p>
        ) : null}
        {isActive && canSearchOrBind ? (
          <div className="space-y-3 rounded-lg border border-border/80 bg-background/60 p-3">
            {item.titleId ? (
              <>
                {bindingLoading ? (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    {t("pendingImports.loadingEpisodeBindings")}
                  </div>
                ) : null}
                {bindingError ? (
                  <div className="text-sm text-destructive">{bindingError}</div>
                ) : null}
                {bindingPreview ? (
                  <div className="space-y-4">
                    <div className="space-y-1">
                      <div className="text-sm font-medium text-foreground">
                        {bindingPreview.title.name}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        {bindingPreview.file.fileName}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        {t("pendingImports.parsedHints")}
                        {bindingPreview.file.parsedSeason != null
                          ? t("pendingImports.parsedHintSeason", {
                              season: bindingPreview.file.parsedSeason,
                            })
                          : ""}
                        {bindingPreview.file.parsedEpisodes.length > 0
                          ? t("pendingImports.parsedHintEpisodes", {
                              episodes: bindingPreview.file.parsedEpisodes.join(", "),
                            })
                          : ""}
                        {bindingPreview.file.parsedAbsoluteNumbers.length > 0
                          ? t("pendingImports.parsedHintAbsolute", {
                              absolute: bindingPreview.file.parsedAbsoluteNumbers.join(", "),
                            })
                          : ""}
                      </div>
                    </div>

                    <div className="flex flex-wrap gap-2">
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        disabled={isBusy}
                        onClick={() => {
                          const suggestedEpisodeIds = bindingPreview.file.suggestedEpisodeIds;
                          onSetSelectedEpisodeIds(suggestedEpisodeIds);
                          onSetExpandedBindingSeasonKeys(
                            bindingSeasonKeysForSelection(
                              bindingPreview.availableEpisodes,
                              suggestedEpisodeIds,
                            ),
                          );
                        }}
                      >
                        {t("pendingImports.useSuggested")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        disabled={isBusy}
                        onClick={() => {
                          onSetSelectedEpisodeIds([]);
                          onSetExpandedBindingSeasonKeys([]);
                        }}
                      >
                        {t("label.clear")}
                      </Button>
                    </div>

                    <div className="space-y-4">
                      {bindingGroups.map(([seasonKey, episodes]) => {
                        const seasonOpen = expandedBindingSeasonKeys.includes(seasonKey);
                        return (
                          <Collapsible
                            key={seasonKey}
                            open={seasonOpen}
                            onOpenChange={(open) =>
                              onSetExpandedBindingSeasonKeys((current) =>
                                open
                                  ? current.includes(seasonKey)
                                    ? current
                                    : [...current, seasonKey]
                                  : current.filter((key) => key !== seasonKey),
                              )
                            }
                            className="space-y-2"
                          >
                            <div className="flex items-center justify-between gap-3">
                              <CollapsibleTrigger asChild>
                                <button
                                  type="button"
                                  className="flex min-w-0 items-center gap-2 text-left text-sm font-medium text-foreground"
                                >
                                  <ChevronDown
                                    className={`h-4 w-4 shrink-0 transition-transform ${
                                      seasonOpen ? "rotate-0" : "-rotate-90"
                                    }`}
                                  />
                                  <span>
                                    {seasonKey === "specials"
                                      ? t("pendingImports.specials")
                                      : t("pendingImports.seasonNumber", { number: seasonKey })}
                                  </span>
                                </button>
                              </CollapsibleTrigger>
                              <Button
                                type="button"
                                size="sm"
                                variant="ghost"
                                disabled={isBusy}
                                onClick={() => {
                                  onSetSelectedEpisodeIds((current) => {
                                    const next = new Set(current);
                                    for (const episode of episodes) {
                                      next.add(episode.id);
                                    }
                                    return Array.from(next);
                                  });
                                  onSetExpandedBindingSeasonKeys((current) =>
                                    current.includes(seasonKey)
                                      ? current
                                      : [...current, seasonKey],
                                  );
                                }}
                              >
                                {t("pendingImports.selectSeason")}
                              </Button>
                            </div>
                            <CollapsibleContent>
                              <div className="space-y-2 rounded-md border border-border/70 p-3">
                                {episodes.map((episode) => {
                                  const episodeDisplay =
                                    formatBindingEpisodeDisplay(episode);
                                  return (
                                    <label
                                      key={episode.id}
                                      className="flex items-start gap-3 text-sm text-foreground"
                                    >
                                      <Checkbox
                                        checked={selectedEpisodeIds.includes(episode.id)}
                                        onCheckedChange={(checked) =>
                                          onToggleEpisodeSelection(
                                            episode.id,
                                            Boolean(checked),
                                          )
                                        }
                                        disabled={isBusy}
                                      />
                                      <span className="min-w-0">
                                        {episodeDisplay.showSeparateLabel ? (
                                          <>
                                            <span className="font-medium">
                                              {episodeDisplay.key}
                                            </span>
                                            <span className="ml-2 text-muted-foreground">
                                              {episodeDisplay.label}
                                            </span>
                                          </>
                                        ) : (
                                          <span>
                                            {episodeDisplay.key ?? episodeDisplay.label}
                                          </span>
                                        )}
                                      </span>
                                    </label>
                                  );
                                })}
                              </div>
                            </CollapsibleContent>
                          </Collapsible>
                        );
                      })}
                    </div>

                    <div className="flex justify-end">
                      <Button
                        type="button"
                        disabled={isBusy || selectedEpisodeIds.length === 0}
                        onClick={() => void onBind()}
                      >
                        {isResolving ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                        {t("pendingImports.bindSelectedEpisodes")}
                      </Button>
                    </div>
                  </div>
                ) : null}
              </>
            ) : (
              <>
                <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
                  <Input
                    className="min-w-0 flex-1"
                    value={searchQuery}
                    onChange={(event) => onSearchQueryChange(event.target.value)}
                    placeholder={t("pendingImports.searchPlaceholder")}
                    disabled={isBusy}
                  />
                  <div className="flex items-center gap-2">
                    <Input
                      type="number"
                      inputMode="numeric"
                      min={1888}
                      max={2100}
                      className="w-28"
                      value={searchYear == null ? "" : String(searchYear)}
                      onChange={(event) => {
                        const raw = event.target.value.trim();
                        if (!raw) {
                          onSearchYearChange(null);
                          return;
                        }
                        const parsed = Number.parseInt(raw, 10);
                        onSearchYearChange(Number.isNaN(parsed) ? null : parsed);
                      }}
                      placeholder={t("pendingImports.searchYearPlaceholder")}
                      aria-label={t("pendingImports.searchYearLabel")}
                      disabled={isBusy}
                    />
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      onClick={() => onSearchYearChange(null)}
                      disabled={isBusy || searchYear == null}
                    >
                      {t("pendingImports.searchYearClear")}
                    </Button>
                  </div>
                </div>

                <p className="text-xs text-muted-foreground">
                  {t("pendingImports.searchYearHint")}
                </p>

                {searching ? (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    {t("pendingImports.searching")}
                  </div>
                ) : null}

                {!searching && searchError ? (
                  <div className="text-sm text-destructive">
                    {t("pendingImports.searchRequestFailed", { error: searchError })}
                  </div>
                ) : null}

                {!searching && !searchError && searchQuery.trim() && searchResults.length === 0 ? (
                  <div className="text-sm text-muted-foreground">
                    {t("pendingImports.noSearchResults")}
                  </div>
                ) : null}

                <div className="space-y-3">
                  {searchResults.map((result) => {
                    const alreadyInLibrary = Boolean(result.existingTitleId);

                    return (
                      <div
                        key={`${item.id}-${result.smgId ?? result.tvdbId ?? result.name}`}
                        className="flex gap-3 rounded-lg border border-border bg-card/40 p-3"
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
                            {alreadyInLibrary ? (
                              <span className="rounded-full border border-border bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                                {t("pendingImports.alreadyInLibrary")}
                              </span>
                            ) : null}
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
                        <div className="flex flex-none items-start">
                          <Button
                            type="button"
                            size="sm"
                            variant={alreadyInLibrary ? "secondary" : "default"}
                            onClick={() =>
                              void onResolve(
                                result,
                                alreadyInLibrary ? { attachToExistingTitle: true } : undefined,
                              )
                            }
                            disabled={isBusy}
                          >
                            {alreadyInLibrary
                              ? t("pendingImports.attachToExistingTitle")
                              : t("pendingImports.match")}
                          </Button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </>
            )}

            <div className="flex justify-end">
              <Button
                type="button"
                variant="ghost"
                onClick={onClearActiveItem}
                disabled={isBusy}
              >
                {t("label.cancel")}
              </Button>
            </div>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
});

function bindingSeasonKeyForEpisode(episode: PendingImportBindingEpisode): string {
  return episode.seasonNumber?.trim() || "specials";
}

function bindingSeasonKeysForSelection(
  episodes: PendingImportBindingEpisode[],
  selectedEpisodeIds: string[],
): string[] {
  if (selectedEpisodeIds.length === 0) {
    return [];
  }

  const selectedIds = new Set(selectedEpisodeIds);
  const expandedKeys = new Set<string>();
  for (const episode of episodes) {
    if (selectedIds.has(episode.id)) {
      expandedKeys.add(bindingSeasonKeyForEpisode(episode));
    }
  }
  return Array.from(expandedKeys);
}
