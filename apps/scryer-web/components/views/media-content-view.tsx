import * as React from "react";
import { useLocation } from "react-router";
import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  ChevronRight,
  ClipboardList,
  Columns3,
  Edit,
  Eye,
  EyeOff,
  FolderPen,
  LayoutGrid,
  LayoutList,
  Loader2,
  PanelLeftOpen,
  PanelRightOpen,
  Pencil,
  RefreshCw,
  Search,
  SlidersHorizontal,
  Sparkles,
  Table as TableIcon,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { deriveInteractiveSearchPresentation } from "@/lib/utils/interactive-search-presentation";
import {
  AnidbExternalLink,
  AnilistExternalLink,
  ImdbExternalLink,
  MalExternalLink,
  TmdbExternalLink,
  TvdbMovieExternalLink,
  TvdbSeriesExternalLink,
} from "@/components/common/external-media-links";
import { FixTitleMatchDialog } from "@/components/dialogs/fix-title-match-dialog";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { useActiveDownloadTitleIds } from "@/lib/hooks/use-active-download-title-ids";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import {
  MediaFilesOnDiskPanel,
  type MediaFileOnDisk,
} from "@/components/common/media-files-on-disk-panel";
import { TitleFilesOnDiskRail } from "@/components/common/title-files-on-disk-rail";
import {
  MediaRenamePlanPanel,
  type MediaRenamePlan,
} from "@/components/common/media-rename-plan-panel";
import { TitleHistoryModal } from "@/components/common/title-history-modal";
import { WatchInMediaServerMenu } from "@/components/common/watch-in-media-server-menu";
import {
  SearchResultBuckets,
  type ReleaseSearchSortDirection,
  type ReleaseSearchSortKey,
} from "@/components/common/release-search-results";
import { HorizontalRail } from "@/components/common/horizontal-scroll-fade";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { TitleCard } from "@/components/title-card";
import { TitleCastStrip } from "@/components/views/title-cast-strip";
import { TitleDubCastStrip } from "@/components/views/title-dub-cast-strip";
import { titleCastOriginalCredits } from "@/lib/utils/title-cast";
import { TitleRatingsStrip } from "@/components/views/title-ratings-strip";
import type {
  ContentSettingsSection,
  OverviewTitleTarget,
  Translate,
  ViewId,
} from "@/components/root/types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type {
  DownloadClientRecord,
  DownloadClientRoutingEntry,
  IndexerCategoryRoutingSettings,
  IndexerRecord,
  LibraryScanSummary,
  LibraryRecord,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  NzbgetCategoryRoutingSettings,
  Release,
  TitleReleaseBlocklistEntry,
  TitleRecord,
  CatalogDiscoveryGroup,
  CatalogDiscoveryItem,
  TitleCatalogFilterOptionsRecord,
} from "@/lib/types";
import type { TitleCatalogAdvancedFilters } from "@/lib/utils/title-catalog-query";
import type {
  InteractiveSearchIndexerProgress,
  InteractiveSearchProgress,
} from "@/lib/graphql/release-search";
import type { ImportMode } from "@/lib/types/settings";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { ViewCategoryId } from "./media-content/indexer-category-picker";
import { MediaLibrarySettingsPanel } from "./media-content/media-library-settings-panel";
import { IndexerRoutingPanel } from "./media-content/indexer-routing-panel";
import { DownloadClientRoutingPanel } from "./media-content/download-client-routing-panel";
import { GeneralSettingsPanel } from "./media-content/general-settings-panel";
import { QualitySettingsPanel } from "./media-content/quality-settings-panel";
import { RenameSettingsPanel } from "./media-content/rename-settings-panel";
import { FacetSettingsSection } from "./media-content/facet-settings-section";
import { AddTitleForm } from "./media-content/add-title-form";
import { PosterGrid } from "./media-content/poster-grid";
import { CatalogFiltersPanel } from "./media-content/catalog-filters-panel";
import { TitleTable } from "./media-content/title-table";
import { CompactTitleTable } from "./media-content/compact-title-table";
import {
  TitleBulkPosterStack,
  TitleWorkspaceActionButton,
  TitleWorkspaceActionGrid,
  TitleWorkspaceHero,
  TitleWorkspacePosterFrame,
  TitleWorkspaceSectionCard,
  TitleWorkspaceSectionHeader,
} from "./media-content/title-workspace-primitives";
import {
  TitleCollectionEmptyState,
  TitleCollectionErrorState,
  TitleTableActionButton,
  TitleCollectionLoadingState,
  formatTitleDate,
  isTitleTableColumnSupportedForView,
  isTitleTableRatingColumn,
  resolveDisplayedQualityLabel,
  titleTableRatingColumnLabel,
  titleTableSupportedRatingColumnsForView,
  type TitleTableColumnKey,
  type TitleTableSortDirection,
  type TitleTableSortKey,
  type TitleTableVisibleColumns,
} from "./media-content/title-table-shared";
import {
  titleOverviewSearchButtonId,
  titleOverviewViewModeId,
} from "@/lib/utils/dom-ids";
import {
  hasActiveTitleQuickFilters,
  TitleQuickFilterBar,
  type TitleQuickFilterCounts,
  type TitleQuickFilters,
} from "./media-content/title-quick-filters";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { RuleSetRecord } from "@/lib/types/rule-sets";
import type {
  FacetScoringPersonaSelectionRecord,
  ParsedQualityProfileEntry,
  ScoringPersonaId,
} from "@/lib/types/quality-profiles";
import { buildViewPath } from "@/lib/utils/routing";
import { selectedSeriesSidePanelTitleId } from "@/lib/utils/selected-overview-policy";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { discoveryItemFacet } from "@/lib/utils/discovery-actions";
import { discoveryItemDisplayTitle } from "@/lib/utils/discovery-display";
import { titleGenreLabels } from "@/lib/utils/title-genres";
import { cn } from "@/lib/utils";
import { persistOverviewWindowScroll } from "@/lib/hooks/use-overview-window-scroll-restoration";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import type { ContentViewMode } from "./media-content/content-view-mode";
import { MovieTitleSettingsPanel } from "./media-content/movie-title-settings-panel";
import { localizedTitleStatus } from "./overview-localization";
import { SeriesOverviewContainer } from "@/components/containers/series-overview-container";
import { handleFixTitleMatchComplete } from "@/lib/fix-title-match";
import type { TitleOptionUpdates } from "@/lib/types/title-options";

type Facet = "MOVIE" | "SERIES" | "ANIME";

function titleTableColumnLabel(
  key: TitleTableColumnKey,
  t: Translate,
  view?: ViewId,
): string {
  if (isTitleTableRatingColumn(key)) {
    return titleTableRatingColumnLabel(key);
  }
  switch (key) {
    case "library":
      return t("title.table.library");
    case "monitored":
      return t("title.table.monitored");
    case "quality":
      return t("title.table.qualityTier");
    case "episodes":
      return t("title.table.episodes");
    case "year":
      return t("title.table.year");
    case "runtime":
      return view === "movies"
        ? t("title.table.runtime")
        : t("title.table.avgRuntime");
    case "status":
      return t("title.table.status");
    case "root":
      return t("title.table.root");
    case "popularity":
      return t("title.table.popularity");
    case "resolution":
      return t("title.table.resolution");
    case "hdr":
      return t("title.table.hdr");
    case "audioCodec":
      return t("title.table.audioCodec");
    case "size":
      return t("title.table.size");
    case "added":
      return t("title.contextAdded");
    case "actions":
      return t("label.actions");
  }
  return key;
}

type ParsedQualityProfile = {
  id: string;
  name: string;
};

const TITLE_OVERVIEW_PANE_MIN_WIDTH = 700;
const TITLE_WORKSPACE_PANE_GAP = 16;
const TITLE_POSTER_GRID_MIN_COLUMN_WIDTH = 200;
const CATALOG_DISCOVERY_INLINE_MIN_WIDTH = 900;
const CATALOG_DISCOVERY_POSTER_INLINE_MIN_WIDTH = 900;
const SELECTED_POSTER_INLINE_MIN_WIDTH =
  TITLE_OVERVIEW_PANE_MIN_WIDTH +
  TITLE_WORKSPACE_PANE_GAP +
  TITLE_POSTER_GRID_MIN_COLUMN_WIDTH;

type QualityProfileOption = {
  value: string;
  label: string;
};

type TvdbSearchItem = MetadataTvdbSearchItem;

type ScopeRoutingRecord = Record<string, NzbgetCategoryRoutingSettings>;
type IndexerRoutingRecord = Record<string, IndexerCategoryRoutingSettings>;

function mediaTitleLabel(view: ViewId, t: Translate): string {
  if (view === "movies") {
    return t("nav.movies");
  }
  if (view === "anime") {
    return t("nav.anime");
  }
  return t("nav.series");
}

function managedStorageSummary(rawBytes: number): string {
  const gigabytes = rawBytes / (1024 * 1024 * 1024);
  if (gigabytes >= 2000) {
    return `${(gigabytes / 1024).toFixed(2)} TB`;
  }
  return `${gigabytes.toFixed(2)} GB`;
}


function useMinViewportWidth(query: string) {
  const [matches, setMatches] = React.useState(() =>
    typeof window === "undefined" ? false : window.matchMedia(query).matches,
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    const mediaQuery = window.matchMedia(query);
    const handleChange = () => setMatches(mediaQuery.matches);
    handleChange();
    mediaQuery.addEventListener("change", handleChange);
    return () => {
      mediaQuery.removeEventListener("change", handleChange);
    };
  }, [query]);

  return matches;
}

function useMeasuredElementWidth<TElement extends HTMLElement>() {
  const [element, setElement] = React.useState<TElement | null>(null);
  const [width, setWidth] = React.useState<number | null>(null);
  const ref = React.useCallback((node: TElement | null) => {
    setElement(node);
  }, []);

  React.useLayoutEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    if (!element) {
      setWidth(null);
      return;
    }

    const updateWidth = () => {
      const nextWidth = Math.round(element.getBoundingClientRect().width);
      setWidth((current) => (current === nextWidth ? current : nextWidth));
    };

    updateWidth();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateWidth);
      return () => window.removeEventListener("resize", updateWidth);
    }

    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);

  return [ref, width] as const;
}

function formatTitleYear(title: TitleRecord): string | null {
  if (typeof title.year === "number" && Number.isFinite(title.year)) {
    return String(title.year);
  }

  if (!title.firstAired) {
    return null;
  }
  const parsed = new Date(title.firstAired);
  return Number.isNaN(parsed.getTime()) ? null : String(parsed.getFullYear());
}

function formatRuntimeLabel(minutes: number | null | undefined): string | null {
  if (!minutes || minutes <= 0) {
    return null;
  }

  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  if (hours <= 0) {
    return `${remainingMinutes}m`;
  }
  if (remainingMinutes <= 0) {
    return `${hours}h`;
  }
  return `${hours}h ${remainingMinutes}m`;
}

function titleExternalIdValue(
  title: TitleRecord,
  source: string,
): string | null {
  const normalizedSource = source.toLowerCase();
  const idFromList = title.externalIds?.find(
    (entry) => entry.source.toLowerCase() === normalizedSource,
  )?.value;
  const value =
    normalizedSource === "imdb" ? title.imdbId || idFromList : idFromList;
  const trimmed = value?.trim();
  return trimmed || null;
}

type TitleContextRecommendation = {
  item: CatalogDiscoveryItem;
  reason: string;
};

type TitleContextRecommendationGroup = {
  id: string;
  label: string;
  recommendations: TitleContextRecommendation[];
};

function titleContextWeeklyLabel(view: ViewId, t: Translate) {
  if (view === "series") {
    return t("title.contextForYouTopSeriesThisWeek");
  }
  if (view === "anime") {
    return t("title.contextForYouTopAnimeThisWeek");
  }
  return t("title.contextForYouTopMoviesThisWeek");
}

function catalogDiscoveryGroupLabel(
  group: CatalogDiscoveryGroup,
  view: ViewId,
  t: Translate,
) {
  switch (group.kind) {
    case "PUBLIC_TOP":
      return group.labelValue ?? titleContextWeeklyLabel(view, t);
    case "PUBLIC_SECTION":
      return group.labelValue ?? titleContextWeeklyLabel(view, t);
    case "GENRE_AFFINITY":
      return t("title.contextForYouGenre", {
        genre: group.labelValue ?? "",
      });
    case "THEME_AFFINITY":
      return t("title.contextForYouTag", {
        tag: group.labelValue ?? "",
      });
    case "ACCLAIMED":
      return t("title.contextForYouAcclaimed");
    case "COMPLETE_COLLECTION":
      return t("title.contextForYouCompleteCollection");
    case "FALLBACK":
      return t("title.contextForYouTop");
  }
}

function catalogDiscoveryGroupReason(
  group: CatalogDiscoveryGroup,
  t: Translate,
) {
  switch (group.kind) {
    case "PUBLIC_TOP":
    case "PUBLIC_SECTION":
      return t("title.contextForYouReasonWeekly");
    case "GENRE_AFFINITY":
      return t("title.contextForYouReasonGenre", {
        genre: group.labelValue ?? "",
      });
    case "THEME_AFFINITY":
      return t("title.contextForYouReasonTag", {
        tag: group.labelValue ?? "",
      });
    case "ACCLAIMED":
      return t("title.contextForYouReasonAcclaimed");
    case "COMPLETE_COLLECTION":
      return t("title.contextForYouReasonCollection");
    case "FALLBACK":
      return t("title.contextForYouReasonTop");
  }
}

function titleContextRecommendationGroupsFromPayload(
  groups: CatalogDiscoveryGroup[],
  view: ViewId,
  t: Translate,
): TitleContextRecommendationGroup[] {
  return groups
    .filter((group) => group.items.length > 0)
    .map((group) => {
      const reason = catalogDiscoveryGroupReason(group, t);
      return {
        id: group.id,
        label: catalogDiscoveryGroupLabel(group, view, t),
        recommendations: group.items.map((item) => ({ item, reason })),
      };
    });
}

function TitleContextRecommendationButton({
  recommendation,
  view,
  t,
  canManageTitle,
  canRequestMedia,
  onAction,
}: {
  recommendation: TitleContextRecommendation;
  view: ViewId;
  t: Translate;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: CatalogDiscoveryItem) => void;
}) {
  const item = recommendation.item;
  const titleLabel = discoveryItemDisplayTitle(item);
  const posterUrl = selectPosterVariantUrl(item.posterUrl, "w250");
  const yearLabel =
    typeof item.year === "number" && Number.isFinite(item.year)
      ? String(item.year)
      : null;
  const owned = item.ownedInInput;
  const addable = !owned && canManageTitle;
  const requestable = !owned && !canManageTitle && canRequestMedia;
  const handleAction = React.useCallback(
    () => onAction(item),
    [onAction, item],
  );

  return (
    <TitleCard
      title={titleLabel}
      year={yearLabel ?? mediaTitleLabel(view, t)}
      posterUrl={posterUrl}
      addable={addable}
      requestable={requestable}
      compact
      onAdd={addable ? handleAction : undefined}
      onRequest={requestable ? handleAction : undefined}
    />
  );
}

function TitleContextMoreLikeThisStrip({
  items,
  loading,
  view,
  manageableFacets,
  requestableFacets,
  onAction,
}: {
  items: CatalogDiscoveryItem[];
  loading: boolean;
  view: ViewId;
  manageableFacets: ReadonlySet<Facet>;
  requestableFacets: ReadonlySet<Facet>;
  onAction: (item: CatalogDiscoveryItem) => void;
}) {
  const t = useTranslate();

  if (items.length === 0) {
    if (!loading) {
      return null;
    }

    return (
      <TitleWorkspaceSectionCard className="rounded-[14px] bg-[var(--scry-surf)]">
        <TitleWorkspaceSectionHeader
          icon={Sparkles}
          title={t("title.contextMoreLikeThis")}
        />
        <div
          aria-busy="true"
          aria-label={t("title.contextMoreLikeThis")}
          className="flex h-36 gap-[11px] overflow-hidden"
        >
          {[0, 1, 2, 3].map((index) => (
            <div
              key={index}
              className="h-36 w-24 shrink-0 animate-pulse rounded-[10px] bg-white/[0.06]"
            />
          ))}
        </div>
      </TitleWorkspaceSectionCard>
    );
  }

  return (
    <TitleWorkspaceSectionCard className="rounded-[14px] bg-[var(--scry-surf)]">
      <TitleWorkspaceSectionHeader
        icon={Sparkles}
        title={t("title.contextMoreLikeThis")}
      />
      <HorizontalRail className="flex gap-[11px] overflow-x-auto pb-1">
        {items.map((item) => {
          const posterUrl = selectPosterVariantUrl(item.posterUrl, "w250");
          const yearLabel =
            typeof item.year === "number" && Number.isFinite(item.year)
              ? String(item.year)
              : null;
          const titleLabel = discoveryItemDisplayTitle(item);
          const owned = item.ownedInInput;
          const facet = discoveryItemFacet(item);
          const addable =
            !owned && facet !== null && manageableFacets.has(facet);
          const requestable =
            !owned &&
            facet !== null &&
            !manageableFacets.has(facet) &&
            requestableFacets.has(facet);

          return (
            <div key={item.id} className="w-24 shrink-0">
              <TitleCard
                title={titleLabel}
                year={yearLabel ?? mediaTitleLabel(view, t)}
                posterUrl={posterUrl}
                addable={addable}
                requestable={requestable}
                compact
                onAdd={addable ? () => onAction(item) : undefined}
                onRequest={requestable ? () => onAction(item) : undefined}
              />
            </div>
          );
        })}
      </HorizontalRail>
    </TitleWorkspaceSectionCard>
  );
}

function TitleContextForYouPanel({
  discoveryGroups,
  view,
  canManageTitle,
  canRequestMedia,
  onDiscoveryAction,
}: {
  discoveryGroups: CatalogDiscoveryGroup[];
  view: ViewId;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onDiscoveryAction: (item: CatalogDiscoveryItem) => void;
}) {
  const t = useTranslate();
  const recommendationGroups = React.useMemo(
    () =>
      titleContextRecommendationGroupsFromPayload(discoveryGroups, view, t),
    [discoveryGroups, t, view],
  );

  return (
    <div className="relative flex min-h-0 flex-1 flex-col overflow-y-auto bg-[var(--scry-surf)] p-[18px] shadow-[inset_0_1px_0_rgba(255,255,255,0.035),inset_0_0_0_1px_rgba(var(--scry-accent-rgb),0.035)]">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 top-0 h-px bg-[linear-gradient(90deg,transparent,rgba(var(--scry-accent-rgb),0.34),transparent)]"
      />
      <div className="flex items-center gap-3">
        <div className="flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.32),rgba(155,91,255,0.2))] text-[var(--scry-accent-text)]">
          <Sparkles className="h-[18px] w-[18px]" />
        </div>
        <div className="min-w-0">
          <p className="text-[16px] font-semibold text-[var(--scry-ink2)]">
            {t("title.contextForYouTitle")}
          </p>
        </div>
      </div>

      {recommendationGroups.length === 0 ? (
        <div className="flex min-h-[42rem] flex-1 flex-col items-center justify-center gap-3 px-4 text-center">
          <div className="flex size-12 items-center justify-center rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[var(--scry-muted2)]">
            <LayoutList className="h-5 w-5" />
          </div>
          <div>
            <p className="text-sm font-semibold text-[var(--scry-ink2)]">
              {t("title.contextForYouEmptyTitle")}
            </p>
            <p className="mt-1 text-[12px] leading-5 text-[var(--scry-muted3)]">
              {t("title.contextForYouEmptyBody")}
            </p>
          </div>
        </div>
      ) : (
        <div className="mt-5 min-h-[42rem] space-y-5">
          {recommendationGroups.map((group) => (
            <section
              key={group.id}
              className="rounded-[15px] border border-[var(--scry-border)] bg-[var(--scry-card2)] px-4 py-4 shadow-[inset_0_1px_0_rgba(255,255,255,0.025)]"
            >
              <h3 className="mx-0.5 mb-3.5 text-[11px] font-bold uppercase tracking-[0.06em] text-[var(--scry-muted3)]">
                {group.label}
              </h3>
              <HorizontalRail
                className="flex gap-3.5 overflow-x-auto pb-1.5 pr-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
                fadeClassName="to-[var(--scry-card2)]"
              >
                {group.recommendations.map((recommendation) => (
                  <div
                    key={`${group.id}-${recommendation.item.id}`}
                    className="w-24 shrink-0"
                  >
                    <TitleContextRecommendationButton
                      recommendation={recommendation}
                      view={view}
                      t={t}
                      canManageTitle={canManageTitle}
                      canRequestMedia={canRequestMedia}
                      onAction={onDiscoveryAction}
                    />
                  </div>
                ))}
              </HorizontalRail>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

function TitleContextReleaseSearchPanel({
  title,
  onInteractiveSearch,
  onQueueFromInteractive,
  onQueueAdditionalFromInteractive,
  disabled = false,
  runRequestId = 0,
  onLoadingChange,
}: {
  title: TitleRecord;
  onInteractiveSearch: (
    title: TitleRecord,
    onUpdate?: (snapshot: InteractiveSearchProgress) => void,
  ) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  onQueueAdditionalFromInteractive: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  disabled?: boolean;
  runRequestId?: number;
  onLoadingChange?: (loading: boolean) => void;
}) {
  const t = useTranslate();
  const requestIdRef = React.useRef(0);
  const lastRunRequestIdRef = React.useRef(0);
  const [results, setResults] = React.useState<Release[] | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [searchFailed, setSearchFailed] = React.useState(false);
  const [indexerProgress, setIndexerProgress] = React.useState<
    InteractiveSearchIndexerProgress[] | null
  >(null);
  const [sortKey, setSortKey] =
    React.useState<ReleaseSearchSortKey>("score");
  const [sortDirection, setSortDirection] =
    React.useState<ReleaseSearchSortDirection>("desc");
  const searchPresentation = React.useMemo(
    () =>
      deriveInteractiveSearchPresentation({
        hasSnapshot: results !== null,
        loading,
        resultCount: results?.length ?? 0,
        indexers: indexerProgress ?? [],
      }),
    [indexerProgress, loading, results],
  );
  const releaseSearchDescription = React.useMemo(() => {
    if (results === null) {
      return t("help.interactiveSearchTooltip");
    }

    if (loading && indexerProgress !== null) {
      return t("title.contextReleaseSearchProgress", {
        releaseCount: results.length,
        done: searchPresentation.completedIndexerCount,
        total: searchPresentation.totalIndexerCount,
      });
    }

    // Report what the run did, not which sources happened to return
    // results: an indexer that answered with nothing still searched, and one
    // that failed or was skipped is called out on its own line below.
    if (searchPresentation.totalIndexerCount > 0) {
      return t("title.contextReleaseSearchSummaryDetailed", {
        releaseCount: results.length,
        searched: searchPresentation.searchedIndexerCount,
        total: searchPresentation.totalIndexerCount,
      });
    }
    const sourceCount = new Set(
      results
        .map((release) => release.source?.trim())
        .filter((source): source is string => Boolean(source)),
    ).size;
    return t("title.contextReleaseSearchSummary", {
      releaseCount: results.length,
      indexerCount: sourceCount,
    });
  }, [indexerProgress, loading, results, searchPresentation, t]);

  const handleSortChange = React.useCallback(
    (
      nextKey: ReleaseSearchSortKey,
      nextDirection: ReleaseSearchSortDirection,
    ) => {
      setSortKey(nextKey);
      setSortDirection(nextDirection);
    },
    [],
  );

  const toggleSort = React.useCallback(
    (nextKey: ReleaseSearchSortKey) => {
      const nextDirection: ReleaseSearchSortDirection =
        sortKey === nextKey && sortDirection === "desc" ? "asc" : "desc";
      handleSortChange(nextKey, nextDirection);
    },
    [handleSortChange, sortDirection, sortKey],
  );

  const renderSortIcon = React.useCallback(
    (key: ReleaseSearchSortKey) => {
      if (sortKey !== key) {
        return <ChevronDown className="h-3 w-3 opacity-45" />;
      }
      return sortDirection === "desc" ? (
        <ArrowDown className="h-3 w-3" />
      ) : (
        <ArrowUp className="h-3 w-3" />
      );
    },
    [sortDirection, sortKey],
  );

  React.useEffect(() => {
    requestIdRef.current += 1;
    setResults(null);
    setLoading(false);
    setSearchFailed(false);
    setIndexerProgress(null);
  }, [title.id]);

  React.useEffect(() => {
    onLoadingChange?.(loading);
  }, [loading, onLoadingChange]);

  const runSearch = React.useCallback(() => {
    if (disabled || loading) {
      return;
    }

    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setLoading(true);
    setSearchFailed(false);
    setIndexerProgress(null);

    const onUpdate = (snapshot: InteractiveSearchProgress) => {
      if (requestIdRef.current !== requestId) {
        return;
      }
      setResults(snapshot.releases);
      setIndexerProgress(snapshot.indexers);
    };

    void Promise.resolve(onInteractiveSearch(title, onUpdate))
      .then((nextResults) => {
        if (requestIdRef.current !== requestId) {
          return;
        }
        setResults(nextResults);
      })
      .catch(() => {
        if (requestIdRef.current !== requestId) {
          return;
        }
        setResults([]);
        setSearchFailed(true);
      })
      .finally(() => {
        if (requestIdRef.current === requestId) {
          setLoading(false);
        }
      });
  }, [disabled, loading, onInteractiveSearch, title]);

  React.useEffect(() => {
    if (runRequestId <= 0 || lastRunRequestIdRef.current === runRequestId) {
      return;
    }
    lastRunRequestIdRef.current = runRequestId;
    runSearch();
  }, [runRequestId, runSearch]);

  const showRetrySearchControl =
    results === null || searchFailed || results.length === 0;

  return (
    <section className="overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-card2)]">
      <div className="flex flex-wrap items-center gap-3 border-b border-[var(--scry-line3)] px-4 py-3.5">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-[9px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.15)] text-[var(--scry-accent-text)]">
          <Search className="h-4 w-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-[14.5px] font-bold text-[var(--scry-ink2)]">
            {t("label.interactiveSearch")}
          </h3>
          <p
            className="mt-0.5 flex items-center gap-1.5 truncate text-[11.5px] text-[var(--scry-faint)]"
            data-ui="title-release-search-summary"
            data-search-state={loading ? "searching" : "done"}
          >
            {loading && indexerProgress !== null ? (
              <Loader2
                className="h-3 w-3 shrink-0 animate-spin"
                aria-label={t("label.searching")}
              />
            ) : null}
            <span className="truncate">{releaseSearchDescription}</span>
          </p>
          {!loading && searchPresentation.failedIndexerNames.length > 0 ? (
            <p className="mt-0.5 truncate text-[11.5px] text-[var(--scry-danger-text)]">
              {t("title.contextReleaseSearchIndexerFailures", {
                count: searchPresentation.failedIndexerNames.length,
                names: searchPresentation.failedIndexerNames.join(", "),
              })}
            </p>
          ) : null}
          {!loading && searchPresentation.skippedIndexers.length > 0 ? (
            <p
              className="mt-0.5 truncate text-[11.5px] text-[var(--scry-faint)]"
              title={searchPresentation.skippedIndexers
                .map((indexer) =>
                  indexer.reason ? `${indexer.name}: ${indexer.reason}` : indexer.name,
                )
                .join("\n")}
            >
              {t("title.contextReleaseSearchIndexerSkipped", {
                count: searchPresentation.skippedIndexers.length,
                names: searchPresentation.skippedIndexers
                  .map((indexer) => indexer.name)
                  .join(", "),
              })}
            </p>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center justify-end gap-1.5">
          {results && results.length > 1 ? (
            <>
              <Button
                type="button"
                size="sm"
                variant={sortKey === "score" ? "secondary" : "outline"}
                className="h-[30px] shrink-0 rounded-[8px] border-[var(--scry-border2)] px-2.5 text-[11px] font-semibold"
                onClick={() => toggleSort("score")}
              >
                <span>Score</span>
                {renderSortIcon("score")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant={sortKey === "size" ? "secondary" : "outline"}
                className="h-[30px] shrink-0 rounded-[8px] border-[var(--scry-border2)] px-2.5 text-[11px] font-semibold"
                onClick={() => toggleSort("size")}
              >
                <span>Size</span>
                {renderSortIcon("size")}
              </Button>
            </>
          ) : null}
          {showRetrySearchControl ? (
            <Button
              type="button"
              size="sm"
              variant={results === null ? "secondary" : "outline"}
              className="h-[30px] shrink-0 rounded-[8px] px-2.5 text-[11px] font-semibold"
              onClick={runSearch}
              disabled={loading || disabled}
            >
              {loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Search className="h-3.5 w-3.5" />
              )}
              <span>{loading ? t("label.searching") : t("label.search")}</span>
            </Button>
          ) : null}
        </div>
      </div>
      <div
        className={cn(
          results !== null && results.length > 0 && !searchFailed ? "" : "p-4",
        )}
      >
        {loading && (results === null || results.length === 0) ? (
          <div className="flex items-center gap-2 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 py-2 text-[12px] text-[var(--scry-muted2)]">
            <Loader2 className="h-4 w-4 animate-spin text-[var(--scry-accent)]" />
            {t("title.searchingReleases")}
          </div>
        ) : searchFailed ? (
          <p className="rounded-[10px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-[12px] text-[var(--scry-danger-text)]">
            {t("nzb.searchFailed")}
          </p>
        ) : results === null ? (
          <p className="rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 py-2 text-[12px] leading-5 text-[var(--scry-muted3)]">
            {t("nzb.noResultsYet")}
          </p>
        ) : results.length === 0 ? (
          <p className="rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 py-2 text-[12px] leading-5 text-[var(--scry-muted3)]">
            {t("title.noReleasesFound", { name: title.name })}
          </p>
        ) : (
          <SearchResultBuckets
            results={results}
            onQueue={(release) => onQueueFromInteractive(title, release)}
            onQueueAdditional={(release) =>
              onQueueAdditionalFromInteractive(title, release)
            }
            canQueueAdditional={(release) =>
              releaseSupportsAdditionalFileQueue(release, title.facet)
            }
            disabled={disabled}
            requireCandidateToken
            sortKey={sortKey}
            sortDirection={sortDirection}
            onSortChange={handleSortChange}
            hideInlineSortControls
            showBlockedInline
            presentation="selected-title"
          />
        )}
      </div>
    </section>
  );
}

function TitleContextPanel({
  id,
  title,
  discoveryGroups,
  libraries,
  librariesLoading,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  advancedFilters,
  filterOptions,
  filterOptionsError,
  onRetryFilterOptions,
  onAdvancedFiltersChange,
  onClearAdvancedFilters,
  view,
  blocklistEntries,
  externalSubtitles,
  isTogglingMonitored,
  isDeleting,
  onUpdateTitleOptions,
  onTitleOptionsChanged,
  onToggleMonitored,
  onAutoQueue,
  onRefreshTitles,
  onRefreshSubtitles,
  onDeleteMediaFile,
  deletingMediaFileIds,
  onMakePrimaryMediaFile,
  primaryMediaFileUpdatingId,
  onPreviewRename,
  onApplyRename,
  refreshLoading,
  onInteractiveSearch,
  onQueueFromInteractive,
  onQueueAdditionalFromInteractive,
  bulkActionBusy,
  onDelete,
  onClearSelection,
  canManageTitle,
  canManageTitlesInLibrary,
  canRequestMedia,
  manageableDiscoveryFacets,
  requestableDiscoveryFacets,
  onDiscoveryAction,
  titleFilterValue,
  onTitleFilterValueChange,
  quickFilters,
  quickFilterCounts,
  quickFilterView,
  onToggleQuickMonitoring,
  onToggleQuickStatus,
  onClearQuickFilters,
  titleListDisclosure,
  onCollapseCatalogRail,
  className,
}: {
  id?: string;
  title: TitleRecord | null;
  discoveryGroups: CatalogDiscoveryGroup[];
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  advancedFilters: TitleCatalogAdvancedFilters;
  filterOptions: TitleCatalogFilterOptionsRecord;
  filterOptionsError: boolean;
  onRetryFilterOptions: () => void;
  onAdvancedFiltersChange: (
    updates: Partial<TitleCatalogAdvancedFilters>,
  ) => void;
  onClearAdvancedFilters: () => void;
  view: ViewId;
  blocklistEntries: TitleReleaseBlocklistEntry[];
  externalSubtitles: ExternalSubtitleRecord[];
  isTogglingMonitored: boolean;
  isDeleting: boolean;
  onUpdateTitleOptions: (
    title: TitleRecord,
    options: TitleOptionUpdates,
  ) => Promise<void> | void;
  onTitleOptionsChanged: (title: TitleRecord) => Promise<void> | void;
  onToggleMonitored?: (
    title: TitleRecord,
    monitored: boolean,
  ) => Promise<void> | void;
  onAutoQueue: (title: TitleRecord) => Promise<void> | void;
  onRefreshTitles: () => Promise<void> | void;
  onRefreshSubtitles: () => Promise<void> | void;
  onDeleteMediaFile: (title: TitleRecord, fileId: string) => void;
  deletingMediaFileIds: ReadonlySet<string>;
  onMakePrimaryMediaFile: (
    title: TitleRecord,
    fileId: string,
  ) => Promise<void> | void;
  primaryMediaFileUpdatingId: string | null;
  onPreviewRename: (
    title: TitleRecord,
  ) => Promise<MediaRenamePlan | null> | MediaRenamePlan | null;
  onApplyRename: (
    title: TitleRecord,
    plan: MediaRenamePlan,
  ) => Promise<boolean | void> | boolean | void;
  refreshLoading: boolean;
  onInteractiveSearch: (
    title: TitleRecord,
    onUpdate?: (snapshot: InteractiveSearchProgress) => void,
  ) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  onQueueAdditionalFromInteractive: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  bulkActionBusy: boolean;
  onDelete: (title: TitleRecord) => void;
  onClearSelection: () => void;
  canManageTitle: boolean;
  canManageTitlesInLibrary: (libraryId: string | null | undefined) => boolean;
  canRequestMedia: boolean;
  manageableDiscoveryFacets: ReadonlySet<Facet>;
  requestableDiscoveryFacets: ReadonlySet<Facet>;
  onDiscoveryAction: (item: CatalogDiscoveryItem) => void;
  titleFilterValue: string;
  onTitleFilterValueChange: (value: string) => void;
  quickFilters: TitleQuickFilters;
  quickFilterCounts: TitleQuickFilterCounts;
  quickFilterView: "movies" | "series" | "anime";
  onToggleQuickMonitoring: (filter: "monitored" | "unmonitored") => void;
  onToggleQuickStatus: (filter: "continuing" | "ended") => void;
  onClearQuickFilters: () => void;
  titleListDisclosure?: React.ReactNode;
  onCollapseCatalogRail?: () => void;
  className?: string;
}) {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const dateTimeFormat = useUiDateTimeFormat();
  const [autoQueueLoadingTitleId, setAutoQueueLoadingTitleId] = React.useState<
    string | null
  >(null);
  const [releaseSearchRequestId, setReleaseSearchRequestId] = React.useState(0);
  const [releaseSearchLoading, setReleaseSearchLoading] = React.useState(false);
  const [releaseSearchTitleId, setReleaseSearchTitleId] = React.useState<
    string | null
  >(null);
  const [renamePlan, setRenamePlan] = React.useState<MediaRenamePlan | null>(
    null,
  );
  const [renamePreviewing, setRenamePreviewing] = React.useState(false);
  const [renameApplying, setRenameApplying] = React.useState(false);
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const [blockedReleasesOpen, setBlockedReleasesOpen] =
    React.useState(false);
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [fixMatchOpen, setFixMatchOpen] = React.useState(false);
  const releaseSearchOpen = title !== null && releaseSearchTitleId === title.id;
  const releaseSearchActionLoading = releaseSearchOpen && releaseSearchLoading;
  // The action bar only carries mutations, so a viewer without manage rights
  // on this title's library gets no bar rather than a row of failing buttons.
  const canManageThisTitle =
    title !== null && canManageTitlesInLibrary(title.libraryId);
  const panelClassName = cn(
    "min-h-0 w-full min-w-0 flex-col overflow-visible min-[981px]:overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-surfD)]",
    className,
  );
  const moreLikeThisItems = React.useMemo(
    () =>
      (title?.moreLikeThis ?? []).filter((item) => {
        const facet = discoveryItemFacet(item);
        return (
          facet !== null &&
          (manageableDiscoveryFacets.has(facet) ||
            requestableDiscoveryFacets.has(facet))
        );
      }),
    [
      manageableDiscoveryFacets,
      requestableDiscoveryFacets,
      title?.moreLikeThis,
    ],
  );
  const titleMediaFiles = React.useMemo<MediaFileOnDisk[]>(
    () =>
      (title?.mediaFiles ?? []).flatMap((file) => {
        const filePath = file.filePath?.trim();
        if (!filePath) {
          return [];
        }
        return [
          {
            ...file,
            filePath,
            sizeBytes: file.sizeBytes ?? null,
            scanStatus: file.scanStatus ?? "unknown",
            videoCodec: file.videoCodec ?? file.videoCodecParsed ?? null,
            videoWidth: file.videoWidth ?? null,
            videoHeight: file.videoHeight ?? null,
            videoBitrateKbps: file.videoBitrateKbps ?? null,
            videoBitDepth: file.videoBitDepth ?? null,
            videoHdrFormat: file.videoHdrFormat ?? null,
            videoFrameRate: file.videoFrameRate ?? null,
            videoProfile: file.videoProfile ?? null,
            audioCodec: file.audioCodec ?? file.audioCodecParsed ?? null,
            audioChannels: file.audioChannels ?? null,
            audioBitrateKbps: file.audioBitrateKbps ?? null,
            audioLanguages: file.audioLanguages ?? [],
            audioStreams: (file.audioStreams ?? []).map((stream) => ({
              codec: stream.codec ?? null,
              channels: stream.channels ?? null,
              language: stream.language ?? null,
              bitrateKbps: stream.bitrateKbps ?? null,
            })),
            subtitleLanguages: file.subtitleLanguages ?? [],
            subtitleCodecs: file.subtitleCodecs ?? [],
            subtitleStreams: (file.subtitleStreams ?? []).map((stream) => ({
              codec: stream.codec ?? null,
              language: stream.language ?? null,
              name: stream.name ?? null,
              forced: stream.forced ?? false,
              default: stream.default ?? false,
            })),
            hasMultiaudio: file.hasMultiaudio ?? false,
            durationSeconds: file.durationSeconds ?? null,
            numChapters: file.numChapters ?? null,
            containerFormat: file.containerFormat ?? null,
          },
        ];
      }),
    [title?.mediaFiles],
  );

  React.useEffect(() => {
    setReleaseSearchLoading(false);
    setReleaseSearchTitleId(null);
  }, [title?.id]);
  React.useEffect(() => {
    setRenamePlan(null);
    setRenamePreviewing(false);
    setRenameApplying(false);
    setHistoryOpen(false);
    setBlockedReleasesOpen(false);
    setSettingsOpen(false);
    setFixMatchOpen(false);
  }, [title?.facet, title?.id]);

  const handleFixMatchComplete = React.useCallback(
    async (warnings: string[]) => {
      if (!title) {
        return;
      }
      await handleFixTitleMatchComplete({
        warnings,
        refreshTitleDetail: async () => {
          await onTitleOptionsChanged(title);
        },
        setGlobalStatus,
        t,
        titleName: title.name,
      });
    },
    [onTitleOptionsChanged, setGlobalStatus, t, title],
  );

  const handlePreviewRename = React.useCallback(async () => {
    if (!title) {
      return;
    }

    setRenamePreviewing(true);
    try {
      setRenamePlan(await onPreviewRename(title));
    } finally {
      setRenamePreviewing(false);
    }
  }, [onPreviewRename, title]);

  const handleApplyRename = React.useCallback(async () => {
    if (!title || !renamePlan) {
      return;
    }

    setRenameApplying(true);
    try {
      const applied = await onApplyRename(title, renamePlan);
      if (applied !== false) {
        setRenamePlan(null);
      }
    } finally {
      setRenameApplying(false);
    }
  }, [onApplyRename, renamePlan, title]);

  if (!title) {
    return (
      <aside
        id={id}
        aria-label={t("title.contextPanelTitle")}
        className={cn("flex", panelClassName)}
      >
        <CatalogFiltersPanel
          libraries={libraries}
          librariesLoading={librariesLoading}
          selectedLibraryIds={selectedLibraryIds}
          onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
          filters={advancedFilters}
          options={filterOptions}
          optionsError={filterOptionsError}
          onRetryOptions={onRetryFilterOptions}
          onFiltersChange={onAdvancedFiltersChange}
          searchValue={titleFilterValue}
          onSearchValueChange={onTitleFilterValueChange}
          onClear={onClearAdvancedFilters}
          quickFilters={quickFilters}
          quickFilterCounts={quickFilterCounts}
          quickFilterView={quickFilterView}
          onToggleQuickMonitoring={onToggleQuickMonitoring}
          onToggleQuickStatus={onToggleQuickStatus}
          onClearQuickFilters={onClearQuickFilters}
          onCollapse={onCollapseCatalogRail}
          className="shrink-0 border-b border-[var(--scry-border2)]"
        />
        <div className="flex min-h-0 flex-1 overflow-hidden">
          <TitleContextForYouPanel
            discoveryGroups={discoveryGroups}
            view={view}
            canManageTitle={canManageTitle}
            canRequestMedia={canRequestMedia}
            onDiscoveryAction={onDiscoveryAction}
          />
        </div>
      </aside>
    );
  }

  const posterUrl = selectPosterVariantUrl(title.posterUrl, "w250");
  const backgroundUrl = title.backgroundUrl ?? title.backgroundSourceUrl ?? null;
  const yearLabel = formatTitleYear(title);
  const statusLabel = localizedTitleStatus(t, title.contentStatus);
  const addedAtLabel =
    formatTitleDate(title.createdAt, dateTimeFormat) ?? t("label.unknown");
  const unknownLabel = t("label.unknown");
  const qualityLabel = resolveDisplayedQualityLabel(title, unknownLabel);
  const overviewText =
    title.overview?.trim() || t("title.descriptionUnavailable");
  const runtimeLabel = formatRuntimeLabel(title.runtimeMinutes);
  const loadingLabel = t("label.loading");
  const studioOrNetworkLabel =
    view === "movies"
      ? title.studio?.trim()
      : title.network?.trim() || title.studio?.trim();
  const imdbId = titleExternalIdValue(title, "imdb");
  const tmdbId = titleExternalIdValue(title, "tmdb");
  const tvdbId = titleExternalIdValue(title, "tvdb");
  const malId = titleExternalIdValue(title, "mal");
  const anilistId = titleExternalIdValue(title, "anilist");
  const anidbId = titleExternalIdValue(title, "anidb");
  const hasExternalLinks = Boolean(
    imdbId || tmdbId || tvdbId || malId || anilistId || anidbId,
  );
  const heroAccentPills = [
    statusLabel,
    qualityLabel === unknownLabel || qualityLabel === loadingLabel
      ? null
      : qualityLabel,
  ].filter((value): value is string => Boolean(value));
  const heroMutedMetadata = [
    runtimeLabel,
    studioOrNetworkLabel,
  ].filter((value): value is string => Boolean(value));
  const heroGenreLabels = titleGenreLabels(title).slice(0, 4);
  const autoQueueLoading = autoQueueLoadingTitleId === title.id;
  const releaseSearchPanelId = `title-context-release-search-${title.id}`;
  const handleAutoQueue = async () => {
    setAutoQueueLoadingTitleId(title.id);
    try {
      await onAutoQueue(title);
    } finally {
      setAutoQueueLoadingTitleId((current) =>
        current === title.id ? null : current,
      );
    }
  };
  const handleInteractiveSearchAction = () => {
    if (releaseSearchOpen) {
      setReleaseSearchTitleId(null);
      setReleaseSearchLoading(false);
      return;
    }
    setReleaseSearchTitleId(title.id);
    setReleaseSearchRequestId((current) => current + 1);
  };

  return (
    <aside
      id={id}
      aria-label={t("title.contextPanelTitle")}
      className={panelClassName}
    >
      <div
        data-slot="title-context-scroll"
        className="relative min-h-0 flex-1 overflow-visible p-4 pb-[max(5rem,calc(1rem+env(safe-area-inset-bottom)))] min-[981px]:overflow-y-auto sm:p-5 sm:pb-5"
      >
        {titleListDisclosure ? (
          <div className="mb-3 flex items-center">{titleListDisclosure}</div>
        ) : null}
        <div className="-mx-4 -mt-4 sm:-mx-5 sm:-mt-5">
          <TitleWorkspaceHero
            backgroundUrl={backgroundUrl}
            closeLabel={t("label.clear")}
            onClose={onClearSelection}
            headerActions={
              (title.playbackLinks?.length ?? 0) > 0 ? (
                <WatchInMediaServerMenu
                  links={title.playbackLinks}
                  showLabel
                  className="justify-end"
                />
              ) : null
            }
          >
            <TitleWorkspacePosterFrame>
              <TitlePosterSlot
                src={posterUrl}
                metadataFetchedAt={title.metadataFetchedAt}
                createdAt={title.createdAt}
                alt={t("media.posterAlt", { name: title.name })}
                className="h-full w-full object-cover"
                placeholderClassName="flex h-full w-full items-center justify-center px-2 text-center text-[11px] text-[var(--scry-muted3)]"
                emptyLabel={t("label.noArt")}
                loading="lazy"
                decoding="async"
              />
            </TitleWorkspacePosterFrame>
            <div className="flex w-full min-w-0 flex-1 flex-col sm:w-auto">
              <h2 className="text-[21px] font-bold leading-[1.1] tracking-normal text-white">
                {title.name}
                {yearLabel ? (
                  <span className="font-medium text-[var(--scry-muted3)]">
                    {" "}
                    ({yearLabel})
                  </span>
                ) : null}
              </h2>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span
                  className={cn(
                    "inline-flex h-6 items-center gap-1 rounded-[7px] px-2.5 text-[11px] font-semibold",
                    title.monitored
                      ? "bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]"
                      : "bg-white/[0.06] text-[var(--scry-muted2)]",
                  )}
                >
                  {title.monitored ? (
                    <Eye className="h-3.5 w-3.5" />
                  ) : (
                    <EyeOff className="h-3.5 w-3.5" />
                  )}
                  {title.monitored
                    ? t("title.monitored")
                    : t("title.unmonitored")}
                </span>
                {heroAccentPills.map((pill, index) => (
                  <span
                    key={`${index}-${pill}`}
                    className="inline-flex h-6 max-w-[12rem] items-center truncate rounded-[7px] bg-[rgba(var(--scry-accent-rgb),0.15)] px-2.5 text-[11px] font-semibold text-[var(--scry-accent-text)]"
                  >
                    {pill}
                  </span>
                ))}
                {heroMutedMetadata.map((pill, index) => (
                  <span
                    key={`${index}-${pill}`}
                    className="inline-flex h-6 max-w-[12rem] items-center truncate text-[12px] font-medium text-[var(--scry-muted2)]"
                  >
                    {pill}
                  </span>
                ))}
              </div>
              {heroGenreLabels.length > 0 ? (
                <div className="mt-3 flex flex-wrap gap-1.5">
                  {heroGenreLabels.map((genre) => (
                    <span
                      key={genre}
                      className="inline-flex h-6 max-w-[9.5rem] items-center rounded-[7px] border border-white/10 bg-white/[0.06] px-2.5 text-[11px] font-semibold text-[#cfd7ee]"
                    >
                      <span className="min-w-0 truncate">{genre}</span>
                    </span>
                  ))}
                </div>
              ) : null}
              <div className="mt-3 hidden min-h-10 sm:block">
                <TitleRatingsStrip ratings={title.ratings} variant="hero" />
              </div>
              <p className="mt-3 min-h-[6.25rem] line-clamp-5 text-[12.5px] leading-5 text-[#b7c0dd]">
                {overviewText}
              </p>
              <div className="mt-auto flex min-h-11 flex-wrap items-center gap-2 pt-3 text-[11px] text-[var(--scry-faint2)]">
                {hasExternalLinks ? (
                  <div className="hidden flex-wrap items-center gap-2 sm:flex [&_a]:h-8 [&_a]:rounded-[8px] [&_a]:border-white/10 [&_a]:bg-white/[0.07] [&_a]:px-2.5 [&_a]:py-1 [&_a]:text-[11px] [&_a]:text-[#dbe4fb] [&_a:hover]:bg-white/[0.12] [&_img]:h-4 [&_img]:w-4 [&_span]:text-[#dbe4fb]">
                    <ImdbExternalLink imdbId={imdbId} />
                    {view === "movies" ? (
                      <TvdbMovieExternalLink tvdbId={tvdbId} slug={title.slug} />
                    ) : (
                      <TvdbSeriesExternalLink tvdbId={tvdbId} slug={title.slug} />
                    )}
                    <TmdbExternalLink
                      mediaType={view === "movies" ? "movie" : "tv"}
                      tmdbId={tmdbId}
                    />
                    <MalExternalLink malId={malId} />
                    <AnilistExternalLink anilistId={anilistId} />
                    <AnidbExternalLink anidbId={anidbId} />
                  </div>
                ) : null}
                <span className="ml-auto shrink-0">
                  {t("title.contextAdded")}: {addedAtLabel}
                </span>
              </div>
            </div>
          </TitleWorkspaceHero>
        </div>

        {canManageThisTitle ? (
          <TitleWorkspaceActionGrid>
            <TitleWorkspaceActionButton
              id="title-overview-toggle-monitoring"
              icon={title.monitored ? EyeOff : Eye}
              label={
                title.monitored
                  ? t("title.unmonitorAction")
                  : t("title.monitorAction")
              }
              active={title.monitored}
              pressed={title.monitored}
              loading={isTogglingMonitored}
              disabled={bulkActionBusy || !onToggleMonitored}
              onClick={() => void onToggleMonitored?.(title, !title.monitored)}
            />
            <TitleWorkspaceActionButton
              id={titleOverviewSearchButtonId(title.id)}
              icon={Zap}
              label={t("label.search")}
              loading={autoQueueLoading}
              disabled={bulkActionBusy}
              onClick={() => void handleAutoQueue()}
            />
            <TitleWorkspaceActionButton
              icon={Search}
              label={t("label.interactive")}
              active={releaseSearchOpen}
              loading={releaseSearchActionLoading}
              disabled={bulkActionBusy && !releaseSearchOpen}
              expanded={releaseSearchOpen}
              controlsId={releaseSearchPanelId}
              onClick={handleInteractiveSearchAction}
            />
            <TitleWorkspaceActionButton
              icon={RefreshCw}
              label={t("label.refresh")}
              loading={refreshLoading}
              disabled={bulkActionBusy || refreshLoading}
              onClick={() => void onRefreshTitles()}
            />
            <TitleWorkspaceActionButton
              icon={ClipboardList}
              label={t("activity.history")}
              disabled={bulkActionBusy}
              onClick={() => setHistoryOpen(true)}
            />
            <TitleWorkspaceActionButton
              id="title-overview-edit-settings"
              icon={Edit}
              label={t("label.edit")}
              active={settingsOpen}
              disabled={bulkActionBusy}
              expanded={settingsOpen}
              controlsId="title-overview-settings-panel"
              onClick={() => setSettingsOpen((current) => !current)}
            />
            <TitleWorkspaceActionButton
              icon={Trash2}
              label={t("label.delete")}
              destructive
              loading={isDeleting}
              disabled={bulkActionBusy}
              onClick={() => onDelete(title)}
            />
          </TitleWorkspaceActionGrid>
        ) : null}

        {settingsOpen ? (
          <div
            id="title-overview-settings-panel"
            role="region"
            aria-label={t("label.edit")}
            className="mb-3 overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)]"
          >
            <MovieTitleSettingsPanel
              title={title}
              libraries={libraries}
              onUpdateTitleOptions={(options) =>
                Promise.resolve(onUpdateTitleOptions(title, options))
              }
              onTitleChanged={() =>
                Promise.resolve(onTitleOptionsChanged(title))
              }
              onOpenFixMatch={() => setFixMatchOpen(true)}
            />
          </div>
        ) : null}

        {releaseSearchOpen ? (
          <div
            id={releaseSearchPanelId}
            role="region"
            aria-label={t("label.interactiveSearch")}
            aria-live="polite"
            className="mt-3"
          >
            <TitleContextReleaseSearchPanel
              title={title}
              onInteractiveSearch={onInteractiveSearch}
              onQueueFromInteractive={onQueueFromInteractive}
              onQueueAdditionalFromInteractive={onQueueAdditionalFromInteractive}
              disabled={bulkActionBusy}
              runRequestId={releaseSearchRequestId}
              onLoadingChange={setReleaseSearchLoading}
            />
          </div>
        ) : null}

        <div className="mt-3 space-y-3">
          <TitleFilesOnDiskRail
            action={
              <Button
                id={`title-context-rename-preview-${title.id}`}
                data-ui="title-context-rename-preview"
                data-title-id={title.id}
                type="button"
                variant="primary"
                size="sm"
                className="h-[34px] shrink-0 justify-center gap-2 rounded-md border border-transparent !bg-primary px-3 text-xs font-semibold !text-primary-foreground shadow-sm hover:!bg-primary/90 focus-visible:ring-[var(--scry-accent-ring)]"
                onClick={() => {
                  void handlePreviewRename();
                }}
                disabled={renamePreviewing || renameApplying}
              >
                {renamePreviewing ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Eye className="h-4 w-4" />
                )}
                <span>
                  {renamePreviewing
                    ? t("rename.previewing")
                    : t("rename.previewButton")}
                </span>
              </Button>
            }
            footer={
              renamePlan ? (
                <MediaRenamePlanPanel
                  plan={renamePlan}
                  applying={renameApplying}
                  applyDisabled={renameApplying || renamePlan.renamable === 0}
                  applyButtonId={`title-context-rename-apply-${title.id}`}
                  onApply={() => {
                    void handleApplyRename();
                  }}
                />
              ) : null
            }
          >
            <MediaFilesOnDiskPanel
              emptyMessage={t("title.noFilesTracked")}
              emptyHint={t("title.noFilesTrackedHint")}
              mediaFiles={titleMediaFiles}
              subtitleDownloads={externalSubtitles}
              onRefreshSubtitles={onRefreshSubtitles}
              onDeleteFile={(fileId) => onDeleteMediaFile(title, fileId)}
              deletingFileIds={deletingMediaFileIds}
              onMakePrimaryFile={
                title.facet === "MOVIE"
                  ? (fileId) => onMakePrimaryMediaFile(title, fileId)
                  : undefined
              }
              primaryFileUpdatingId={primaryMediaFileUpdatingId}
              showPrimaryRoleBadge
              fileRowIdPrefix={`title-context-file-row-${title.id}`}
              filePathIdPrefix={`title-context-file-path-${title.id}`}
              roleIdPrefix={`title-context-file-role-${title.id}`}
              subtitleSearchIdPrefix={`title-context-file-search-subtitles-${title.id}`}
              deleteFileIdPrefix={`title-context-file-delete-${title.id}`}
              makePrimaryFileIdPrefix={`title-context-file-make-primary-${title.id}`}
              presentation="selected-title"
            />
          </TitleFilesOnDiskRail>

          <TitleContextMoreLikeThisStrip
            items={moreLikeThisItems}
            loading={title?.moreLikeThis === undefined}
            view={view}
            manageableFacets={manageableDiscoveryFacets}
            requestableFacets={requestableDiscoveryFacets}
            onAction={onDiscoveryAction}
          />

          <TitleCastStrip
            credits={titleCastOriginalCredits(title.credits)}
            variant="workspace"
          />

          <TitleDubCastStrip credits={title.credits} variant="workspace" />

          {blocklistEntries.length === 0 ? (
            <section className="flex min-h-[3.25rem] items-center gap-2.5 rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] px-4">
              <ChevronRight className="h-4 w-4 shrink-0 text-[var(--scry-faint)]" />
              <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold text-[var(--scry-text2)]">
                {t("title.contextBlockedReleases")}
              </span>
              <span className="shrink-0 rounded-[7px] bg-white/[0.06] px-2 py-0.5 text-[11px] font-semibold text-[var(--scry-muted)]">
                {blocklistEntries.length}
              </span>
            </section>
          ) : (
            <Collapsible
              open={blockedReleasesOpen}
              onOpenChange={setBlockedReleasesOpen}
            >
              <section className="overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)]">
                <CollapsibleTrigger asChild>
                  <button
                    type="button"
                    className="flex min-h-[3.25rem] w-full min-w-0 items-center gap-2.5 px-4 text-left transition hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--scry-focus)]"
                  >
                    <ChevronRight
                      className={cn(
                        "h-4 w-4 shrink-0 text-[var(--scry-faint)] transition-transform",
                        blockedReleasesOpen && "rotate-90",
                      )}
                    />
                    <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold text-[var(--scry-text2)]">
                      {t("title.contextBlockedReleases")}
                    </span>
                    <span className="shrink-0 rounded-[7px] bg-white/[0.06] px-2 py-0.5 text-[11px] font-semibold text-[var(--scry-muted)]">
                      {blocklistEntries.length}
                    </span>
                  </button>
                </CollapsibleTrigger>
                <CollapsibleContent className="border-t border-[var(--scry-line3)] p-4">
                  <div className="space-y-2">
                    {blocklistEntries.map((entry) => {
                      const attemptedAtLabel = formatTitleDate(
                        entry.attemptedAt,
                        dateTimeFormat,
                      );
                      const releaseLabel =
                        entry.releaseName.trim() ||
                        t("episode.untitledRelease");

                      return (
                        <div
                          key={entry.id}
                          className="rounded-[11px] border border-[var(--scry-line3)] bg-[var(--scry-inset)] p-3"
                        >
                          <div className="flex min-w-0 items-start justify-between gap-3">
                            <div className="min-w-0">
                              <p className="line-clamp-2 break-words text-[12px] font-semibold text-[var(--scry-ink2)]">
                                {releaseLabel}
                              </p>
                              <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-[var(--scry-muted3)]">
                                {attemptedAtLabel ? (
                                  <span>{attemptedAtLabel}</span>
                                ) : null}
                              </div>
                            </div>
                          </div>
                          {entry.errorMessage ? (
                            <p className="mt-2 line-clamp-3 rounded-[8px] bg-[var(--scry-danger-bg)] px-2.5 py-1.5 text-[11px] leading-4 text-[var(--scry-danger-text)]">
                              {entry.errorMessage}
                            </p>
                          ) : null}
                        </div>
                      );
                    })}
                  </div>
                </CollapsibleContent>
              </section>
            </Collapsible>
          )}
        </div>
      </div>
      <FixTitleMatchDialog
        open={fixMatchOpen}
        onOpenChange={setFixMatchOpen}
        title={{
          id: title.id,
          name: title.name,
          facet: title.facet,
          externalIds: title.externalIds ?? [],
        }}
        onFixed={handleFixMatchComplete}
      />
      <TitleHistoryModal
        open={historyOpen}
        onOpenChange={setHistoryOpen}
        titleId={title.id}
        titleName={title.name}
      />
    </aside>
  );
}

function isMediaSettingsSection(section: ContentSettingsSection): boolean {
  return (
    section === "library" ||
    section === "general" ||
    section === "quality" ||
    section === "renaming" ||
    section === "routing"
  );
}

function canAccessMediaSettingsSection(
  section: ContentSettingsSection,
  canManageConfig: boolean,
  canManageLibrarySettings: boolean,
): boolean {
  if (!isMediaSettingsSection(section)) {
    return true;
  }

  if (section === "library") {
    return canManageConfig || canManageLibrarySettings;
  }

  return canManageConfig;
}

export function MediaContentView({
  state,
}: {
  state: {
    view: ViewId;
    contentSettingsSection: ContentSettingsSection;
    canManageConfig: boolean;
    canManageSystemSettings: boolean;
    canManageCatalogSettings: boolean;
    canManageLibrarySettings: boolean;
    contentSettingsLabel: string;
    moviesPath: string;
    setMoviesPath: (value: string) => void;
    seriesPath: string;
    setSeriesPath: (value: string) => void;
    localPathStyle: LocalPathStyle | undefined;
    mediaSettingsLoading: boolean;
    librarySettingsSaving: boolean;
    qualityProfiles: ParsedQualityProfile[];
    qualityProfileEntries: ParsedQualityProfileEntry[];
    qualityProfileParseError: string;
    globalScoringPersona: ScoringPersonaId;
    categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
    categoryRequiredAudioLanguages: Record<ViewCategoryId, string[]>;
    saveCategoryRequiredAudioLanguages: (
      languages: string[],
    ) => Promise<void> | void;
    categoryPersonaSelections: Record<
      ViewCategoryId,
      FacetScoringPersonaSelectionRecord
    >;
    activeQualityScopeId: ViewCategoryId;
    categoryFolderTemplates: Record<ViewCategoryId, string>;
    setCategoryFolderTemplates: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categorySeasonFolderTemplates: Record<ViewCategoryId, string>;
    setCategorySeasonFolderTemplates: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryUseSeasonFolders: Record<ViewCategoryId, boolean>;
    setCategoryUseSeasonFolders: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, boolean>>
    >;
    categorySpecialsFolderTemplates: Record<ViewCategoryId, string>;
    setCategorySpecialsFolderTemplates: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameTemplates: Record<ViewCategoryId, string>;
    setCategoryRenameTemplates: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameEnabled: Record<ViewCategoryId, string>;
    setCategoryRenameEnabled: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameCollisionPolicies: Record<ViewCategoryId, string>;
    setCategoryRenameCollisionPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameMissingMetadataPolicies: Record<ViewCategoryId, string>;
    setCategoryRenameMissingMetadataPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryFillerPolicies: Record<ViewCategoryId, string>;
    setCategoryFillerPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRecapPolicies: Record<ViewCategoryId, string>;
    setCategoryRecapPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryMonitorSpecials: Record<ViewCategoryId, string>;
    setCategoryMonitorSpecials: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryInterSeasonMovies: Record<ViewCategoryId, string>;
    setCategoryInterSeasonMovies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryMonitorFillerMovies: Record<ViewCategoryId, string>;
    setCategoryMonitorFillerMovies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    nfoWriteOnImport: Record<ViewCategoryId, string>;
    setNfoWriteOnImport: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    plexmatchWriteOnImport: Record<ViewCategoryId, string>;
    setPlexmatchWriteOnImport: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    importMode: Record<ViewCategoryId, ImportMode>;
    setImportMode: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, ImportMode>>
    >;
    setPermissionsLinux: Record<ViewCategoryId, string>;
    setSetPermissionsLinux: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    fileChmod: Record<ViewCategoryId, string>;
    setFileChmod: React.Dispatch<React.SetStateAction<Record<ViewCategoryId, string>>>;
    folderChmod: Record<ViewCategoryId, string>;
    setFolderChmod: React.Dispatch<React.SetStateAction<Record<ViewCategoryId, string>>>;
    chownGroup: Record<ViewCategoryId, string>;
    setChownGroup: React.Dispatch<React.SetStateAction<Record<ViewCategoryId, string>>>;
    qualityProfileInheritValue: string;
    toProfileOptions: (
      profiles: ParsedQualityProfile[],
    ) => QualityProfileOption[];
    handleFacetPersonaSave: (
      persona: ScoringPersonaId | null,
    ) => Promise<void> | void;
    saveSetting: (
      scope: string,
      scopeId: string | undefined,
      keyName: string,
      value: string,
    ) => void;
    saveCategoryQualityProfileOverride: (value: string) => Promise<void> | void;
    updateCategoryMediaProfileSettings: (
      event: React.FormEvent<HTMLFormElement>,
    ) => Promise<void> | void;
    mediaSettingsSaving: boolean;
    titleNameForQueue: string;
    setTitleNameForQueue: (value: string) => void;
    queueFacet: Facet;
    setQueueFacet: (value: Facet) => void;
    monitoredForQueue: boolean;
    setMonitoredForQueue: (value: boolean) => void;
    seasonFoldersForQueue: boolean;
    setSeasonFoldersForQueue: (value: boolean) => void;
    minAvailabilityForQueue: string;
    setMinAvailabilityForQueue: (value: string) => void;
    tvdbCandidates: TvdbSearchItem[];
    onAddSubmit: (
      event: React.FormEvent<HTMLFormElement>,
    ) => Promise<void> | void;
    addTvdbCandidateToCatalog: (
      candidate: TvdbSearchItem,
    ) => Promise<void> | void;
    titleFilter: string;
    setTitleFilter: (value: string) => void;
    refreshTitles: (query?: string) => Promise<void> | void;
    titleLoading: boolean;
    catalogTotalTitleCount: number;
    catalogManagedBytes: number;
    catalogHasMoreTitles: boolean;
    catalogLoadingMoreTitles: boolean;
    loadMoreCatalogTitles: () => Promise<void> | void;
    titleCatalogSortKey: TitleTableSortKey;
    titleCatalogSortDirection: TitleTableSortDirection;
    updateTitleCatalogSort: (key: TitleTableSortKey) => void;
    visibleTitleTableColumns: TitleTableVisibleColumns;
    setTitleTableColumnVisible: (
      key: TitleTableColumnKey,
      checked: boolean,
    ) => void;
    catalogBootstrapLoading: boolean;
    catalogInitialLoadComplete: boolean;
    catalogSurfacePhase:
      | "resolving"
      | "content"
      | "empty"
      | "rootsMissing"
      | "rootsInvalid"
      | "error";
    catalogSurfaceError: string | null;
    retryCatalogBootstrap: () => void;
    monitoredTitles: TitleRecord[];
    titleContextTitles: TitleRecord[];
    catalogDiscoveryGroups: CatalogDiscoveryGroup[];
    canViewCatalog: boolean;
    canManageTitle: boolean;
    canManageTitlesInLibrary: (libraryId: string | null | undefined) => boolean;
    canRequestMedia: boolean;
    canManageCatalogDiscovery: boolean;
    canRequestCatalogDiscovery: boolean;
    manageableDiscoveryFacets: Facet[];
    requestableDiscoveryFacets: Facet[];
    onCatalogDiscoveryAction: (item: CatalogDiscoveryItem) => void;
    titleQuickFilters: TitleQuickFilters;
    titleQuickFilterCounts: TitleQuickFilterCounts;
    advancedTitleFilters: TitleCatalogAdvancedFilters;
    titleCatalogFilterOptions: TitleCatalogFilterOptionsRecord;
    titleCatalogFilterOptionsError: boolean;
    retryTitleCatalogFilterOptions: () => void;
    updateAdvancedTitleFilters: (
      updates: Partial<TitleCatalogAdvancedFilters>,
    ) => void;
    clearAdvancedTitleFilters: () => void;
    toggleTitleQuickMonitoringFilter: (
      filter: "monitored" | "unmonitored",
    ) => void;
    toggleTitleQuickStatusFilter: (filter: "continuing" | "ended") => void;
    clearTitleQuickFilters: () => void;
    queueExisting: (title: TitleRecord) => Promise<void> | void;
    toggleTitleMonitored: (
      title: TitleRecord,
      monitored: boolean,
    ) => Promise<void> | void;
    runInteractiveSearchForTitle: (
      title: TitleRecord,
      onUpdate?: (snapshot: InteractiveSearchProgress) => void,
    ) => Promise<Release[]> | Release[];
    queueExistingFromRelease: (
      title: TitleRecord,
      release: Release,
    ) => Promise<void> | void;
    queueAdditionalFromRelease: (
      title: TitleRecord,
      release: Release,
    ) => Promise<void> | void;
    isTogglingTitleMonitoredById: Record<string, boolean>;
    downloadClients: DownloadClientRecord[];
    activeScopeRouting: ScopeRoutingRecord;
    activeScopeRoutingOrder: string[];
    downloadClientRoutingLoading: boolean;
    downloadClientRoutingSaving: boolean;
    updateDownloadClientRoutingForScope: (
      clientId: string,
      nextValue: Partial<NzbgetCategoryRoutingSettings>,
      options?: { save?: boolean },
    ) => Promise<void> | void;
    moveDownloadClientInScope: (
      clientId: string,
      direction: "up" | "down",
    ) => void;
    indexers: IndexerRecord[];
    activeScopeIndexerRouting: IndexerRoutingRecord;
    activeScopeIndexerRoutingOrder: string[];
    indexerRoutingLoading: boolean;
    indexerRoutingSaving: boolean;
    setIndexerEnabledForScope: (
      indexerId: string,
      enabled: boolean,
    ) => Promise<void> | void;
    updateIndexerRoutingForScope: (
      indexerId: string,
      nextValue: Partial<IndexerCategoryRoutingSettings>,
    ) => Promise<void> | void;
    moveIndexerInScope: (indexerId: string, direction: "up" | "down") => void;
    ruleSets: RuleSetRecord[];
    rulesLoading: boolean;
    rulesSaving: boolean;
    onToggleRuleFacet: (ruleSetId: string, enabled: boolean) => void;
    libraryScanLoading: boolean;
    libraryScanDisabled: boolean;
    libraryScanNotice: string | null;
    libraryScanSummary: LibraryScanSummary | null;
    libraries: LibraryRecord[];
    librariesLoading: boolean;
    rootValidationLibraries: LibraryRecord[];
    rootValidationLibrariesLoading: boolean;
    rootValidationUnavailable: boolean;
    invalidRootPathsByLibraryId: Record<string, string[]>;
    selectedLibraryIds: string[];
    allLibrariesValue: string;
    setSelectedLibraryIds: (value: string[]) => void;
    libraryDownloadClients: DownloadClientRecord[];
    libraryDownloadClientsLoading: boolean;
    loadLibrarySettings: (
      libraryId: string,
    ) => Promise<LibrarySettingsRecord | null>;
    loadFacetDownloadClientRouting: (
      scopeId: Facet,
    ) => Promise<DownloadClientRoutingEntry[]>;
    createLibrary: (input: {
      name: string;
      roots: import("@/lib/types/titles").RootFolderOption[];
      settings?: LibrarySettingsDraft;
    }) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
    updateLibrary: (
      libraryId: string,
      input: {
        name: string;
        roots: import("@/lib/types/titles").RootFolderOption[];
        settings?: LibrarySettingsDraft;
      },
    ) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
    deleteLibrary: (
      libraryId: string,
    ) => Promise<boolean | void> | boolean | void;
    scanLibrary: (libraryId?: string) => Promise<void> | void;
    onOpenOverview: (
      targetView: ViewId,
      overviewTarget: OverviewTitleTarget,
    ) => void;
    selectedOverviewTitleId: string | null;
    selectedOverviewTitle: TitleRecord | null;
    selectedOverviewDetailLoading: boolean;
    routeOverviewPending: boolean;
    routeOverviewEpisodeId: string | null;
    selectedOverviewBlocklistEntries: TitleReleaseBlocklistEntry[];
    selectedOverviewExternalSubtitles: ExternalSubtitleRecord[];
    refreshSelectedOverviewExternalSubtitles: () => Promise<void> | void;
    deleteSelectedOverviewMediaFile: (
      title: TitleRecord,
      fileId: string,
    ) => void;
    pendingMediaFileDeletionIds: ReadonlySet<string>;
    makeSelectedOverviewMovieFilePrimary: (
      title: TitleRecord,
      fileId: string,
    ) => Promise<void> | void;
    selectedOverviewPrimaryMovieFileUpdatingId: string | null;
    previewTitleRename: (
      title: TitleRecord,
    ) => Promise<MediaRenamePlan | null> | MediaRenamePlan | null;
    applyTitleRename: (
      title: TitleRecord,
      plan: MediaRenamePlan,
    ) => Promise<boolean | void> | boolean | void;
    setSelectedOverviewTitleId: (titleId: string | null) => void;
    clearSelectedOverviewTitle: () => void;
    onCloseOverview: () => void;
    updateMovieTitleOptions: (
      title: TitleRecord,
      options: TitleOptionUpdates,
    ) => Promise<void> | void;
    refreshMovieTitleOptions: (title: TitleRecord) => Promise<void> | void;
    deleteCatalogTitle: (title: TitleRecord) => void;
    isDeletingCatalogTitleById: Record<string, boolean>;
    isMobile: boolean;
    viewMode: ContentViewMode;
    setViewMode: (value: ContentViewMode) => void;
    selectedTitleIds: ReadonlySet<string>;
    toggleTitleSelection: (titleId: string) => void;
    toggleAllVisibleTitles: (checked: boolean) => void;
    clearSelectedTitles: () => void;
    bulkActionBusy: boolean;
    bulkMonitorTitles: (monitored: boolean) => Promise<void> | void;
    openBulkTitleEdit: () => void;
    openBulkTitleDelete: () => void;
    openBulkTitleRename: () => void;
    canRenameSelectedTitles: boolean;
  };
}) {
  const t = useTranslate();
  const location = useLocation();
  const contextPanelViewportMatches = useMinViewportWidth(
    "(min-width: 760px)",
  );
  const posterContextPanelViewportMatches = useMinViewportWidth(
    "(min-width: 720px)",
  );
  const selectedTitleListInlineViewportMatches = useMinViewportWidth(
    "(min-width: 1180px)",
  );
  const selectedPosterInlineViewportMatches = useMinViewportWidth(
    `(min-width: ${SELECTED_POSTER_INLINE_MIN_WIDTH}px)`,
  );
  const {
    view,
    contentSettingsSection,
    canManageConfig,
    canManageSystemSettings,
    canManageCatalogSettings,
    canManageLibrarySettings,
    contentSettingsLabel,
    localPathStyle,
    mediaSettingsLoading,
    librarySettingsSaving,
    qualityProfiles,
    qualityProfileParseError,
    globalScoringPersona,
    categoryQualityProfileOverrides,
    categoryRequiredAudioLanguages,
    saveCategoryRequiredAudioLanguages,
    categoryPersonaSelections,
    activeQualityScopeId,
    categoryFolderTemplates,
    setCategoryFolderTemplates,
    categorySeasonFolderTemplates,
    setCategorySeasonFolderTemplates,
    categoryUseSeasonFolders,
    setCategoryUseSeasonFolders,
    categorySpecialsFolderTemplates,
    setCategorySpecialsFolderTemplates,
    categoryRenameTemplates,
    setCategoryRenameTemplates,
    categoryRenameEnabled,
    setCategoryRenameEnabled,
    categoryRenameCollisionPolicies,
    setCategoryRenameCollisionPolicies,
    categoryRenameMissingMetadataPolicies,
    setCategoryRenameMissingMetadataPolicies,
    categoryFillerPolicies,
    setCategoryFillerPolicies,
    categoryRecapPolicies,
    setCategoryRecapPolicies,
    categoryMonitorSpecials,
    setCategoryMonitorSpecials,
    categoryInterSeasonMovies,
    setCategoryInterSeasonMovies,
    categoryMonitorFillerMovies,
    setCategoryMonitorFillerMovies,
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
    importMode,
    setImportMode,
    setPermissionsLinux,
    setSetPermissionsLinux,
    fileChmod,
    setFileChmod,
    folderChmod,
    setFolderChmod,
    chownGroup,
    setChownGroup,
    qualityProfileInheritValue,
    toProfileOptions,
    handleFacetPersonaSave,
    saveSetting,
    saveCategoryQualityProfileOverride,
    updateCategoryMediaProfileSettings,
    mediaSettingsSaving,
    titleNameForQueue,
    setTitleNameForQueue,
    queueFacet,
    setQueueFacet,
    monitoredForQueue,
    setMonitoredForQueue,
    seasonFoldersForQueue,
    setSeasonFoldersForQueue,
    minAvailabilityForQueue,
    setMinAvailabilityForQueue,
    tvdbCandidates,
    addTvdbCandidateToCatalog,
    onAddSubmit,
    titleFilter,
    setTitleFilter,
    refreshTitles,
    titleLoading,
    catalogTotalTitleCount,
    catalogManagedBytes,
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    loadMoreCatalogTitles,
    titleCatalogSortKey,
    titleCatalogSortDirection,
    updateTitleCatalogSort,
    visibleTitleTableColumns,
    setTitleTableColumnVisible,
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    catalogSurfacePhase,
    catalogSurfaceError,
    retryCatalogBootstrap,
    monitoredTitles,
    titleContextTitles,
    catalogDiscoveryGroups,
    canViewCatalog,
    canManageTitle,
    canManageTitlesInLibrary,
    canManageCatalogDiscovery,
    canRequestCatalogDiscovery,
    manageableDiscoveryFacets,
    requestableDiscoveryFacets,
    onCatalogDiscoveryAction,
    titleQuickFilters,
    titleQuickFilterCounts,
    advancedTitleFilters,
    titleCatalogFilterOptions,
    titleCatalogFilterOptionsError,
    retryTitleCatalogFilterOptions,
    updateAdvancedTitleFilters,
    clearAdvancedTitleFilters,
    toggleTitleQuickMonitoringFilter,
    toggleTitleQuickStatusFilter,
    clearTitleQuickFilters,
    queueExisting,
    toggleTitleMonitored,
    runInteractiveSearchForTitle,
    queueExistingFromRelease,
    queueAdditionalFromRelease,
    isTogglingTitleMonitoredById,
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
    indexers,
    activeScopeIndexerRouting,
    activeScopeIndexerRoutingOrder,
    indexerRoutingLoading,
    indexerRoutingSaving,
    setIndexerEnabledForScope,
    updateIndexerRoutingForScope,
    moveIndexerInScope,
    libraryScanLoading,
    libraryScanDisabled,
    libraryScanNotice,
    libraryScanSummary,
    libraries,
    librariesLoading,
    libraryDownloadClients,
    libraryDownloadClientsLoading,
    rootValidationLibraries,
    rootValidationLibrariesLoading,
    rootValidationUnavailable,
    invalidRootPathsByLibraryId,
    selectedLibraryIds,
    allLibrariesValue,
    setSelectedLibraryIds,
    scanLibrary,
    onOpenOverview,
    selectedOverviewTitleId,
    selectedOverviewTitle: selectedOverviewTitleOverride,
    selectedOverviewDetailLoading,
    routeOverviewPending,
    routeOverviewEpisodeId,
    selectedOverviewBlocklistEntries,
    selectedOverviewExternalSubtitles,
    refreshSelectedOverviewExternalSubtitles,
    deleteSelectedOverviewMediaFile,
    pendingMediaFileDeletionIds,
    makeSelectedOverviewMovieFilePrimary,
    selectedOverviewPrimaryMovieFileUpdatingId,
    previewTitleRename,
    applyTitleRename,
    setSelectedOverviewTitleId,
    onCloseOverview,
    updateMovieTitleOptions,
    refreshMovieTitleOptions,
    deleteCatalogTitle,
    isDeletingCatalogTitleById,
    viewMode,
    setViewMode,
    selectedTitleIds,
    toggleTitleSelection,
    toggleAllVisibleTitles,
    clearSelectedTitles,
    bulkActionBusy,
    bulkMonitorTitles,
    openBulkTitleEdit,
    openBulkTitleDelete,
    openBulkTitleRename,
    canRenameSelectedTitles,
  } = state;
  const [titleFilterInputValue, setTitleFilterInputValue] =
    React.useState(titleFilter);
  const [titleLayoutRef, titleLayoutWidth] =
    useMeasuredElementWidth<HTMLDivElement>();
  const deferredTitleContextTitles =
    React.useDeferredValue(titleContextTitles);
  const deferredCatalogDiscoveryGroups = React.useDeferredValue(
    catalogDiscoveryGroups,
  );
  const manageableDiscoveryFacetSet = React.useMemo(
    () => new Set(manageableDiscoveryFacets),
    [manageableDiscoveryFacets],
  );
  const requestableDiscoveryFacetSet = React.useMemo(
    () => new Set(requestableDiscoveryFacets),
    [requestableDiscoveryFacets],
  );
  const routeOverviewSlug = React.useMemo(() => {
    if (!routeOverviewPending) {
      return null;
    }
    const segments = location.pathname.split("/").filter(Boolean);
    const candidate = segments[segments.length - 1]?.trim();
    if (!candidate || candidate === "overview" || candidate === "settings") {
      return null;
    }
    try {
      return decodeURIComponent(candidate);
    } catch {
      return candidate;
    }
  }, [location.pathname, routeOverviewPending]);
  const titleTableSupportedRatingColumns = React.useMemo(
    () => titleTableSupportedRatingColumnsForView(view),
    [view],
  );
  const isTitleTableColumnSupported = React.useCallback(
    (key: TitleTableColumnKey) =>
      isTitleTableColumnSupportedForView(key, view),
    [view],
  );
  const titleTableColumnGroups = React.useMemo(
    () =>
      [
        {
          id: "core",
          label: t("title.table.groupCore"),
          columns: [
            "library",
            "monitored",
            "quality",
            "episodes",
            "year",
            "runtime",
            "status",
          ] satisfies TitleTableColumnKey[],
        },
        {
          id: "ratings",
          label: t("title.table.groupRatings"),
          columns: [...titleTableSupportedRatingColumns],
        },
        {
          id: "media",
          label: t("title.table.groupMedia"),
          columns: [
            "size",
            "resolution",
            "hdr",
            "audioCodec",
            "popularity",
          ] satisfies TitleTableColumnKey[],
        },
        {
          id: "operational",
          label: t("title.table.groupOperational"),
          columns: ["root", "added", "actions"] satisfies TitleTableColumnKey[],
        },
      ]
        .map((group) => ({
          ...group,
          columns: group.columns.filter(isTitleTableColumnSupported),
        }))
        .filter((group) => group.columns.length > 0),
    [isTitleTableColumnSupported, t, titleTableSupportedRatingColumns],
  );
  const toggleTitleTableColumn = React.useCallback(
    (key: TitleTableColumnKey, checked: boolean) => {
      setTitleTableColumnVisible(key, checked);
    },
    [setTitleTableColumnVisible],
  );

  React.useEffect(() => {
    if (titleCatalogSortKey === "name") {
      return;
    }
    const columnKey = titleCatalogSortKey as TitleTableColumnKey;
    if (
      !isTitleTableColumnSupported(columnKey) ||
      visibleTitleTableColumns[columnKey] !== true
    ) {
      updateTitleCatalogSort("name");
    }
  }, [
    isTitleTableColumnSupported,
    titleCatalogSortKey,
    updateTitleCatalogSort,
    visibleTitleTableColumns,
  ]);

  React.useEffect(() => {
    setTitleFilterInputValue((current) =>
      current === titleFilter ? current : titleFilter,
    );
  }, [titleFilter]);
  const compactSelectedVisibleCount = React.useMemo(
    () =>
      monitoredTitles.filter((title) => selectedTitleIds.has(title.id))
        .length,
    [monitoredTitles, selectedTitleIds],
  );
  const bulkPosterStackTitles = React.useMemo(
    () =>
      monitoredTitles
        .filter((title) => selectedTitleIds.has(title.id))
        .slice(0, 3),
    [monitoredTitles, selectedTitleIds],
  );
  const selectedOverviewTitle = React.useMemo(
    () => {
      if (selectedOverviewTitleId) {
        if (selectedOverviewTitleOverride?.id === selectedOverviewTitleId) {
          return selectedOverviewTitleOverride;
        }
        return (
          deferredTitleContextTitles.find(
            (title) => title.id === selectedOverviewTitleId,
          ) ??
          monitoredTitles.find((title) => title.id === selectedOverviewTitleId) ??
          null
        );
      }
      if (routeOverviewSlug) {
        return (
          deferredTitleContextTitles.find(
            (title) => title.slug === routeOverviewSlug,
          ) ?? null
        );
      }
      return null;
    },
    [
      deferredTitleContextTitles,
      monitoredTitles,
      routeOverviewSlug,
      selectedOverviewTitleId,
      selectedOverviewTitleOverride,
    ],
  );
  const activeOverviewTitle = selectedOverviewTitle;
  const seriesSidePanelTitleId = selectedSeriesSidePanelTitleId(
    view,
    selectedOverviewTitleId,
  );
  const activeOverviewTitleId = activeOverviewTitle?.id ?? seriesSidePanelTitleId;
  React.useEffect(() => {
    if (
      !selectedOverviewTitleId ||
      selectedOverviewTitle ||
      seriesSidePanelTitleId
    ) {
      return;
    }
    if (
      titleLoading ||
      catalogBootstrapLoading ||
      !catalogInitialLoadComplete ||
      selectedOverviewDetailLoading
    ) {
      return;
    }
    onCloseOverview();
  }, [
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    onCloseOverview,
    seriesSidePanelTitleId,
    selectedOverviewDetailLoading,
    selectedOverviewTitle,
    selectedOverviewTitleId,
    titleLoading,
  ]);
  const handleSelectOverviewTitle = React.useCallback(
    (title: TitleRecord) => {
      // Selecting a title is a navigation: the URL (slug deep link) is the
      // source of truth, and the container mirrors it into the inline pane.
      setSelectedOverviewTitleId(title.id);
      onOpenOverview(view, {
        id: title.id,
        slug: title.slug ?? null,
        libraryId: title.libraryId,
        librarySlug: title.librarySlug ?? null,
      });
    },
    [onOpenOverview, setSelectedOverviewTitleId, view],
  );
  const effectiveContentSettingsSection = canAccessMediaSettingsSection(
    contentSettingsSection,
    canManageConfig,
    canManageLibrarySettings,
  )
    ? contentSettingsSection
    : canManageLibrarySettings &&
        !canManageConfig &&
        isMediaSettingsSection(contentSettingsSection)
      ? "library"
      : "overview";

  const scopeLabel =
    activeQualityScopeId === "MOVIE"
      ? t("search.facetMovie")
      : activeQualityScopeId === "SERIES"
        ? t("search.facetSeries")
        : t("search.facetAnime");
  const effectiveViewMode: ContentViewMode = viewMode;
  const contextPanelMinimumWidth =
    effectiveViewMode === "poster" ? 720 : 760;
  const catalogDiscoveryInlineMinimumWidth =
    effectiveViewMode === "poster"
      ? CATALOG_DISCOVERY_POSTER_INLINE_MIN_WIDTH
      : CATALOG_DISCOVERY_INLINE_MIN_WIDTH;
  const contextPanelWidthMatches =
    titleLayoutWidth == null
      ? effectiveViewMode === "poster"
        ? posterContextPanelViewportMatches
        : contextPanelViewportMatches
      : titleLayoutWidth >= contextPanelMinimumWidth;
  const catalogDiscoveryInlineWidthMatches =
    titleLayoutWidth != null &&
    titleLayoutWidth >= catalogDiscoveryInlineMinimumWidth;
  const selectedOverviewTitleAvailable =
    selectedOverviewTitleId !== null &&
    (selectedOverviewTitle !== null || seriesSidePanelTitleId !== null);
  const titleContextPanelAvailable =
    contextPanelWidthMatches || selectedOverviewTitleAvailable;
  const selectedTitleLayoutActive =
    titleContextPanelAvailable && activeOverviewTitleId !== null;
  const [catalogContextRailCollapsed, setCatalogContextRailCollapsed] =
    React.useState(false);
  const catalogDiscoveryInlineAvailable =
    activeOverviewTitleId === null &&
    catalogDiscoveryInlineWidthMatches &&
    !catalogContextRailCollapsed;
  const catalogDiscoveryFlyoutAvailable =
    activeOverviewTitleId === null && !catalogDiscoveryInlineWidthMatches;
  const contextPanelAvailable =
    selectedTitleLayoutActive || catalogDiscoveryInlineAvailable;
  const selectedTitlePosterInlineActive =
    selectedTitleLayoutActive &&
    effectiveViewMode === "poster" &&
    (titleLayoutWidth == null
      ? selectedPosterInlineViewportMatches
      : titleLayoutWidth >= SELECTED_POSTER_INLINE_MIN_WIDTH);
  const selectedTitleCompactLayoutActive =
    selectedTitleLayoutActive && !selectedTitlePosterInlineActive;
  // Keep the title list inline only when the overview has enough room to avoid
  // stealing space from the table.
  const selectedTitleListInlineActive =
    selectedTitleCompactLayoutActive &&
    (titleLayoutWidth == null
      ? selectedTitleListInlineViewportMatches
      : titleLayoutWidth >= contextPanelMinimumWidth);
  const [selectedTitleListDrawerOpen, setSelectedTitleListDrawerOpen] =
    React.useState(false);
  const selectedTitleListDrawerRef = React.useRef<HTMLDivElement | null>(null);
  const selectedTitleListPreviousFocusRef = React.useRef<HTMLElement | null>(
    null,
  );
  const selectedTitleListDrawerModeActive =
    selectedTitleListDrawerOpen &&
    selectedTitleCompactLayoutActive &&
    !selectedTitleListInlineActive;

  React.useEffect(() => {
    setSelectedTitleListDrawerOpen(false);
  }, [activeOverviewTitleId, selectedTitleCompactLayoutActive]);

  React.useEffect(() => {
    if (!selectedTitleListDrawerModeActive) {
      return;
    }

    selectedTitleListPreviousFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;

    const getDrawerFocusableElements = () => {
      const drawer = selectedTitleListDrawerRef.current;
      if (!drawer) {
        return [];
      }

      return Array.from(
        drawer.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((element) => element.offsetParent !== null);
    };

    const focusFrame = window.requestAnimationFrame(() => {
      const drawer = selectedTitleListDrawerRef.current;
      if (!drawer) {
        return;
      }

      (getDrawerFocusableElements()[0] ?? drawer).focus();
    });

    const handleSelectedTitleDrawerKeyDown = (event: KeyboardEvent) => {
      const drawer = selectedTitleListDrawerRef.current;
      const eventTarget = event.target;
      if (!(eventTarget instanceof Node) || !drawer?.contains(eventTarget)) {
        return;
      }

      if (event.key === "Escape") {
        event.preventDefault();
        setSelectedTitleListDrawerOpen(false);
        return;
      }

      if (event.key !== "Tab") {
        return;
      }

      const focusableElements = getDrawerFocusableElements();
      if (focusableElements.length === 0) {
        event.preventDefault();
        selectedTitleListDrawerRef.current?.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];
      if (event.shiftKey && document.activeElement === firstElement) {
        event.preventDefault();
        lastElement.focus();
      } else if (!event.shiftKey && document.activeElement === lastElement) {
        event.preventDefault();
        firstElement.focus();
      }
    };

    window.addEventListener("keydown", handleSelectedTitleDrawerKeyDown);
    return () => {
      window.cancelAnimationFrame(focusFrame);
      window.removeEventListener("keydown", handleSelectedTitleDrawerKeyDown);
      const previousFocus = selectedTitleListPreviousFocusRef.current;
      selectedTitleListPreviousFocusRef.current = null;
      if (previousFocus && document.contains(previousFocus)) {
        previousFocus.focus();
      }
    };
  }, [selectedTitleListDrawerModeActive]);

  const isTitleCatalogView =
    view === "movies" || view === "series" || view === "anime";
  // One catalog-wide subscription to the live download queue (import activity
  // included) feeds the pulsing "Downloading" pill in every display mode. It
  // deliberately sits outside the catalog's own paging/sorting/title queries so
  // it can't perturb them.
  const activeDownloadTitleIds = useActiveDownloadTitleIds({
    enabled: isTitleCatalogView && canViewCatalog,
  });
  const selectedTitleCompactDrawerActive =
    selectedTitleCompactLayoutActive && !selectedTitleListInlineActive;
  const selectedTitleTableInlineActive =
    selectedTitleCompactLayoutActive && selectedTitleListInlineActive;
  const selectedTitleFullTableInlineActive =
    selectedTitleTableInlineActive && effectiveViewMode === "poster-table";
  const collectionViewMode: ContentViewMode = effectiveViewMode;
  const selectedTitlePosterLayoutActive =
    selectedTitleLayoutActive && collectionViewMode === "poster";
  // The design never sheds table columns — the table always keeps the chosen
  // columns and scrolls horizontally when the overview pane squeezes it. The
  // narrow "compact drawer" (title/mon/size) is a separate width-gated layout,
  // not column shedding, so we keep the full column set in every table state.
  const effectiveVisibleTitleTableColumns = visibleTitleTableColumns;
  const showTitleTableColumnControls =
    effectiveViewMode !== "poster" && !selectedTitleCompactDrawerActive;
  const multiSelectActive = selectedTitleIds.size > 0;
  // In multi-select mode the bulk actions live in the side panel; the inline
  // bar is only a fallback for widths too narrow to show the panel.
  const showTitleBulkSelectionBar =
    multiSelectActive &&
    !contextPanelAvailable &&
    (collectionViewMode === "compact" ||
      collectionViewMode === "poster-table");

  React.useEffect(() => {
    if (
      !isTitleCatalogView ||
      collectionViewMode !== "poster" ||
      !catalogHasMoreTitles ||
      catalogLoadingMoreTitles
    ) {
      return;
    }

    const maybeLoadNextPage = () => {
      if (selectedTitlePosterLayoutActive) {
        const element = selectedTitleListDrawerRef.current;
        if (!element || element.clientHeight <= 0) {
          return;
        }
        const remaining =
          element.scrollHeight - (element.scrollTop + element.clientHeight);
        if (remaining <= 1200) {
          void loadMoreCatalogTitles();
        }
        return;
      }

      const scrollElement = document.documentElement;
      const remaining =
        scrollElement.scrollHeight - (window.scrollY + window.innerHeight);
      if (remaining <= 1200) {
        void loadMoreCatalogTitles();
      }
    };

    const scrollElement = selectedTitlePosterLayoutActive
      ? selectedTitleListDrawerRef.current
      : window;
    scrollElement?.addEventListener("scroll", maybeLoadNextPage, {
      passive: true,
    });
    window.addEventListener("resize", maybeLoadNextPage);
    return () => {
      scrollElement?.removeEventListener("scroll", maybeLoadNextPage);
      window.removeEventListener("resize", maybeLoadNextPage);
    };
  }, [
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    collectionViewMode,
    monitoredTitles.length,
    isTitleCatalogView,
    loadMoreCatalogTitles,
    selectedTitlePosterLayoutActive,
  ]);

  const selectedTitleListDrawerId =
    activeOverviewTitleId !== null
      ? `title-context-list-drawer-${activeOverviewTitleId}`
      : "title-context-list-drawer";
  const selectedTitleContextPanelId =
    activeOverviewTitleId !== null
      ? `title-context-panel-${activeOverviewTitleId}`
      : "title-context-panel";
  const contextPanelSelectedTitleId = contextPanelAvailable
    ? activeOverviewTitleId
    : null;
  const onSelectTitleForContextPanel = handleSelectOverviewTitle;
  const handleOpenOverviewFromContext = React.useCallback(
    (targetView: ViewId, overviewTarget: OverviewTitleTarget) => {
      if (effectiveViewMode === "poster") {
        persistOverviewWindowScroll(location.pathname);
      }
      setSelectedOverviewTitleId(overviewTarget.id);
      onOpenOverview(targetView, overviewTarget);
    },
    [
      effectiveViewMode,
      location.pathname,
      onOpenOverview,
      setSelectedOverviewTitleId,
    ],
  );
  const showInitialScanAction =
    canManageLibrarySettings &&
    catalogSurfacePhase === "empty";
  const showConfigureRootFoldersAction =
    canManageLibrarySettings &&
    (catalogSurfacePhase === "rootsMissing" ||
      catalogSurfacePhase === "rootsInvalid");
  const configureRootFoldersReason =
    catalogSurfacePhase === "rootsInvalid" ? "invalid" : "missing";
  const configureRootFoldersHref =
    view === "movies" || view === "series" || view === "anime"
      ? buildViewPath(view, undefined, "library")
      : undefined;

  const mediaLibrarySettingsTitle =
    view === "series"
      ? t("settings.seriesLibrarySettings")
      : view === "anime"
        ? t("settings.animeSettings")
        : t("settings.moviesLibrarySettings");

  const handleRenameTemplateChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategoryRenameTemplates((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameTemplates],
  );

  const handleFolderTemplateChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategoryFolderTemplates((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategoryFolderTemplates],
  );

  const handleSeasonFolderTemplateChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategorySeasonFolderTemplates((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategorySeasonFolderTemplates],
  );

  const handleSpecialsFolderTemplateChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategorySpecialsFolderTemplates((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategorySpecialsFolderTemplates],
  );

  const handleRenameCollisionPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRenameCollisionPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameCollisionPolicies],
  );

  const handleRenameMissingMetadataPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRenameMissingMetadataPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameMissingMetadataPolicies],
  );

  const handleFillerPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryFillerPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "anime.filler_policy", value);
    },
    [activeQualityScopeId, setCategoryFillerPolicies, saveSetting],
  );

  const handleRecapPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRecapPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "anime.recap_policy", value);
    },
    [activeQualityScopeId, setCategoryRecapPolicies, saveSetting],
  );

  const handleMonitorSpecialsChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryMonitorSpecials((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting(
        "system",
        activeQualityScopeId,
        "anime.monitor_specials",
        value,
      );
    },
    [activeQualityScopeId, setCategoryMonitorSpecials, saveSetting],
  );

  const handleInterSeasonMoviesChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryInterSeasonMovies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting(
        "system",
        activeQualityScopeId,
        "anime.inter_season_movies",
        value,
      );
    },
    [activeQualityScopeId, setCategoryInterSeasonMovies, saveSetting],
  );

  const handleMonitorFillerMoviesChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryMonitorFillerMovies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting(
        "system",
        activeQualityScopeId,
        "anime.monitor_filler_movies",
        value,
      );
    },
    [activeQualityScopeId, setCategoryMonitorFillerMovies, saveSetting],
  );

  const handleNfoWriteChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      const key =
        activeQualityScopeId === "MOVIE"
          ? "nfo.write_on_import.movie"
          : activeQualityScopeId === "ANIME"
            ? "nfo.write_on_import.anime"
            : "nfo.write_on_import.series";
      setNfoWriteOnImport((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", undefined, key, value);
    },
    [activeQualityScopeId, setNfoWriteOnImport, saveSetting],
  );

  const handlePlexmatchWriteChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      const key =
        activeQualityScopeId === "ANIME"
          ? "plexmatch.write_on_import.anime"
          : "plexmatch.write_on_import.series";
      setPlexmatchWriteOnImport((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", undefined, key, value);
    },
    [activeQualityScopeId, setPlexmatchWriteOnImport, saveSetting],
  );

  const handleImportModeChange = React.useCallback(
    (value: ImportMode) => {
      setImportMode((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "import.mode", value);
    },
    [activeQualityScopeId, saveSetting, setImportMode],
  );

  const handleSetPermissionsLinuxChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setSetPermissionsLinux((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "permissions.set_linux", value);
    },
    [activeQualityScopeId, saveSetting, setSetPermissionsLinux],
  );

  const handleFileChmodChange = React.useCallback(
    (value: string) => {
      setFileChmod((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "permissions.file_chmod", value);
    },
    [activeQualityScopeId, saveSetting, setFileChmod],
  );

  const handleFolderChmodChange = React.useCallback(
    (value: string) => {
      setFolderChmod((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "permissions.folder_chmod", value);
    },
    [activeQualityScopeId, saveSetting, setFolderChmod],
  );

  const handleChownGroupChange = React.useCallback(
    (value: string) => {
      setChownGroup((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "permissions.chown_group", value);
    },
    [activeQualityScopeId, saveSetting, setChownGroup],
  );

  const handleIndexerCategoriesChange = React.useCallback(
    (indexerId: string, categories: string[]) => {
      void updateIndexerRoutingForScope(indexerId, {
        categories,
      });
    },
    [updateIndexerRoutingForScope],
  );

  const handleIndexerEnabledChange = React.useCallback(
    (indexerId: string, checked: boolean) => {
      void setIndexerEnabledForScope(indexerId, checked);
    },
    [setIndexerEnabledForScope],
  );

  const moveIndexerUp = React.useCallback(
    (indexerId: string) => {
      moveIndexerInScope(indexerId, "up");
    },
    [moveIndexerInScope],
  );

  const moveIndexerDown = React.useCallback(
    (indexerId: string) => {
      moveIndexerInScope(indexerId, "down");
    },
    [moveIndexerInScope],
  );

  const handleTitleFilterValueChange = React.useCallback(
    (nextValue: string) => {
      setTitleFilterInputValue(nextValue);
      setTitleFilter(nextValue);
    },
    [setTitleFilter],
  );
  const handleTitleFilterChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      handleTitleFilterValueChange(event.target.value);
    },
    [handleTitleFilterValueChange],
  );

  const handleRefreshTitles = React.useCallback(() => {
    const nextQuery = titleFilterInputValue;
    if (titleFilter !== nextQuery) {
      React.startTransition(() => {
        setTitleFilter(nextQuery);
      });
    }
    void refreshTitles(nextQuery);
  }, [refreshTitles, setTitleFilter, titleFilter, titleFilterInputValue]);

  const handleSelectedOverviewBackToList = React.useCallback(() => {
    onCloseOverview();
    void refreshTitles(titleFilterInputValue);
  }, [onCloseOverview, refreshTitles, titleFilterInputValue]);

  const handleLibraryScan = React.useCallback(
    (libraryId?: string) => {
      void scanLibrary(libraryId);
    },
    [scanLibrary],
  );

  const quickFilterView =
    view === "movies" ? "movies" : view === "series" ? "series" : "anime";
  const hasActiveTitleDisplayFilters =
    titleFilter.trim().length > 0 ||
    hasActiveTitleQuickFilters(titleQuickFilters, quickFilterView);
  const showEmptyStateActions = !hasActiveTitleDisplayFilters;
  const knownCatalogTitleCount = Math.max(
    titleQuickFilterCounts.all,
    catalogTotalTitleCount,
  );

  const handleDeleteCatalogTitle = React.useCallback(
    (title: TitleRecord) => {
      deleteCatalogTitle(title);
    },
    [deleteCatalogTitle],
  );
  const mediaTitle = mediaTitleLabel(view, t);
  const titleSummaryNoun = (() => {
    if (view === "movies") {
      return knownCatalogTitleCount === 1 ? "title" : "titles";
    }
    return view === "series" ? "series" : "anime";
  })();
  const mediaSummary = [
    `${knownCatalogTitleCount.toLocaleString()} ${titleSummaryNoun}`,
    managedStorageSummary(Math.max(0, catalogManagedBytes)),
  ].join(" · ");

  const [libraryRoutingWide, setLibraryRoutingWide] = React.useState(false);
  const [libraryCrumb, setLibraryCrumb] = React.useState<string | null>(null);
  const facetSettingsSection =
    effectiveContentSettingsSection === "library" ||
    effectiveContentSettingsSection === "general" ||
    effectiveContentSettingsSection === "quality" ||
    effectiveContentSettingsSection === "renaming" ||
    effectiveContentSettingsSection === "routing"
      ? effectiveContentSettingsSection
      : null;

  const titleTableViewControls = (
    <div className="flex w-auto min-w-0 items-center justify-end gap-1.5">
      {showTitleTableColumnControls ? (
        <Popover>
          <PopoverTrigger asChild>
            <Button
              type="button"
              variant="outline"
              className="h-9 w-9 rounded-[10px] border !border-[rgba(var(--scry-accent-rgb),0.55)] bg-[var(--scry-inset)] px-0 text-[var(--scry-muted2)] shadow-none transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
              aria-label={t("title.columns")}
              title={t("title.columns")}
            >
              <Columns3 className="!size-4" />
            </Button>
          </PopoverTrigger>
          <PopoverContent
            align="end"
            className="w-[236px] rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] p-[7px] shadow-[0_18px_44px_rgba(0,0,0,0.55)]"
          >
            <div className="px-2 pb-2 pt-1 text-[10.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
              {t("title.toggleColumns")}
            </div>
            <div className="max-h-[min(32rem,70vh)] overflow-y-auto pr-1">
              {titleTableColumnGroups.map((group) => (
                <div key={group.id} className="pb-1.5 last:pb-0">
                  <div className="px-2 pb-1 pt-1.5 text-[10px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint3)]">
                    {group.label}
                  </div>
                  {group.columns.map((columnKey) => {
                    const label = titleTableColumnLabel(columnKey, t, view);
                    return (
                      <label
                        key={columnKey}
                        className="flex cursor-pointer items-center gap-2.5 rounded-[8px] px-2 py-[7px] text-[13px] text-[var(--scry-text2)] transition hover:bg-[var(--scry-hover)]"
                      >
                        <Checkbox
                          checked={visibleTitleTableColumns[columnKey]}
                          onCheckedChange={(checked) =>
                            toggleTitleTableColumn(
                              columnKey,
                              checked === true,
                            )
                          }
                          aria-label={label}
                          size="compact"
                        />
                        <span className="min-w-0 truncate">{label}</span>
                      </label>
                    );
                  })}
                </div>
              ))}
            </div>
          </PopoverContent>
        </Popover>
      ) : (
        <div
          aria-hidden="true"
          className="hidden h-[2.8125rem] shrink-0 sm:block sm:w-[2.8125rem]"
        />
      )}
      <ToggleGroup
        type="single"
        variant="outline"
        value={effectiveViewMode}
        onValueChange={(v) => {
          if (v === "compact" || v === "poster-table" || v === "poster") {
            setViewMode(v);
          }
        }}
        size="sm"
        aria-label={t("title.viewModeToggle")}
        className="h-[2.8125rem] w-auto shrink-0 justify-center gap-[0.1875rem] rounded-[11px] border !border-[rgba(var(--scry-accent-rgb),0.55)] bg-[var(--scry-inset)] p-[0.28125rem] shadow-none"
      >
        <ToggleGroupItem
          id={titleOverviewViewModeId(view, "compact")}
          value="compact"
          size="sm"
          aria-label={t("title.viewModeCompact")}
          title={t("title.viewModeCompact")}
          className="h-9 w-9 rounded-[9px] px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent)] data-[state=on]:!text-primary-foreground data-[state=on]:!shadow-none"
        >
          <TableIcon className="h-[1.125rem] w-[1.125rem]" />
        </ToggleGroupItem>
        <ToggleGroupItem
          id={titleOverviewViewModeId(view, "poster-table")}
          value="poster-table"
          size="sm"
          aria-label={t("title.viewModePosterTable")}
          title={t("title.viewModePosterTable")}
          className="h-9 w-9 rounded-[9px] px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent)] data-[state=on]:!text-primary-foreground data-[state=on]:!shadow-none"
        >
          <LayoutList className="h-[1.125rem] w-[1.125rem]" />
        </ToggleGroupItem>
        <ToggleGroupItem
          id={titleOverviewViewModeId(view, "poster")}
          value="poster"
          size="sm"
          aria-label={t("title.viewModePoster")}
          title={t("title.viewModePoster")}
          className="h-9 w-9 rounded-[9px] px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent)] data-[state=on]:!text-primary-foreground data-[state=on]:!shadow-none"
        >
          <LayoutGrid className="h-[1.125rem] w-[1.125rem]" />
        </ToggleGroupItem>
      </ToggleGroup>
      {catalogContextRailCollapsed &&
      activeOverviewTitleId === null &&
      catalogDiscoveryInlineWidthMatches ? (
        <Button
          type="button"
          variant="outline"
          className="h-[2.8125rem] w-full rounded-[11px] border !border-[rgba(var(--scry-accent-rgb),0.55)] bg-[var(--scry-inset)] px-4 text-[15px] text-[var(--scry-accent-text)] shadow-none transition hover:bg-[var(--scry-hover)] sm:w-[2.8125rem] sm:px-0"
          aria-label={t("discovery.filters")}
          title={t("discovery.filters")}
          onClick={() => setCatalogContextRailCollapsed(false)}
        >
          <PanelRightOpen className="!size-[1.125rem]" />
          <span className="sm:hidden">{t("discovery.filters")}</span>
        </Button>
      ) : null}
    </div>
  );

  const keepCatalogHeaderOutsideWorkspace =
    selectedTitleCompactLayoutActive && !selectedTitleListInlineActive;
  const catalogHeader = (
    <div className="relative min-h-[3.25rem] shrink-0 px-4 pb-0 pt-2 sm:min-h-[4.5rem] sm:px-5 lg:px-6">
      <div className="flex min-w-0 items-center justify-between gap-3">
        <div className="min-w-0">
          <h1 className="text-[22px] font-bold leading-tight tracking-normal text-[var(--scry-ink2)]">
            {mediaTitle}
          </h1>
          <p className="mt-1 hidden text-[12.5px] text-[var(--scry-muted3)] sm:block">
            {mediaSummary}
          </p>
        </div>
        <div
          className={cn(
            "shrink-0 sm:absolute sm:right-4 sm:z-10 lg:right-5",
            showTitleBulkSelectionBar
              ? "sm:top-3"
              : "sm:top-1/2 sm:-translate-y-1/2",
          )}
        >
          {titleTableViewControls}
        </div>
      </div>
      {showTitleBulkSelectionBar ? (
        <div className="mt-4">
          <TitleQuickFilterBar
            view={view as "movies" | "series" | "anime"}
            filters={titleQuickFilters}
            counts={titleQuickFilterCounts}
            onToggleMonitoring={toggleTitleQuickMonitoringFilter}
            onToggleStatus={toggleTitleQuickStatusFilter}
            onClear={clearTitleQuickFilters}
            hideFilters
            trailingContent={
              <div className="flex w-full flex-col gap-2.5 sm:w-auto sm:flex-row sm:flex-wrap sm:items-center sm:justify-end">
                <div className="flex h-12 w-full items-center justify-end gap-2 rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-2 sm:w-[20rem]">
                  <span className="mr-1 whitespace-nowrap text-sm text-[var(--scry-muted3)]">
                    {t("title.bulkSelectionCount", {
                      count: compactSelectedVisibleCount,
                    })}
                  </span>
                  <TitleTableActionButton
                    tone="enabled"
                    label={t("title.monitorAction")}
                    onClick={() => void bulkMonitorTitles(true)}
                    disabled={bulkActionBusy}
                    className="rounded-md"
                  >
                    <Eye className="h-4 w-4" />
                  </TitleTableActionButton>
                  <TitleTableActionButton
                    tone="disabled"
                    label={t("title.unmonitorAction")}
                    onClick={() => void bulkMonitorTitles(false)}
                    disabled={bulkActionBusy}
                    className="rounded-md"
                  >
                    <EyeOff className="h-4 w-4" />
                  </TitleTableActionButton>
                  <TitleTableActionButton
                    tone="edit"
                    label={t("label.edit")}
                    onClick={openBulkTitleEdit}
                    disabled={bulkActionBusy}
                    className="rounded-md"
                  >
                    <Pencil className="h-4 w-4" />
                  </TitleTableActionButton>
                  {canRenameSelectedTitles ? (
                    <TitleTableActionButton
                      tone="accent"
                      label={t("title.renameAction")}
                      onClick={openBulkTitleRename}
                      disabled={bulkActionBusy}
                      className="rounded-md"
                    >
                      <FolderPen className="h-4 w-4" />
                    </TitleTableActionButton>
                  ) : null}
                  <TitleTableActionButton
                    tone="delete"
                    label={t("label.delete")}
                    onClick={openBulkTitleDelete}
                    disabled={bulkActionBusy}
                    className="rounded-md"
                  >
                    <Trash2 className="h-4 w-4" />
                  </TitleTableActionButton>
                  <TitleTableActionButton
                    tone="neutral"
                    label={t("label.clear")}
                    onClick={clearSelectedTitles}
                    disabled={bulkActionBusy}
                    className="rounded-md"
                  >
                    <X className="h-4 w-4" />
                  </TitleTableActionButton>
                </div>
              </div>
            }
          />
        </div>
      ) : null}
    </div>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      {facetSettingsSection ? (
        <FacetSettingsSection
          view={view}
          section={facetSettingsSection}
          facetLabel={mediaTitle}
          canManageConfig={canManageConfig}
          canManageLibrarySettings={canManageLibrarySettings}
          showSecondaryNav={facetSettingsSection === "library"}
          contentWidth={
            facetSettingsSection === "renaming"
              ? "reference"
              : facetSettingsSection === "routing"
                ? "full"
              : facetSettingsSection === "library" && libraryRoutingWide
              ? "wide"
              : "default"
          }
          trailingCrumb={
            facetSettingsSection === "library"
              ? (libraryCrumb ?? undefined)
              : undefined
          }
        >
          {effectiveContentSettingsSection === "quality" ? (
        <QualitySettingsPanel
          contentSettingsLabel={contentSettingsLabel}
          mediaSettingsLoading={mediaSettingsLoading}
          mediaSettingsSaving={mediaSettingsSaving}
          qualityProfiles={qualityProfiles}
          qualityProfileParseError={qualityProfileParseError}
          categoryQualityProfileOverrides={categoryQualityProfileOverrides}
          categoryRequiredAudioLanguages={categoryRequiredAudioLanguages}
          saveCategoryRequiredAudioLanguages={
            saveCategoryRequiredAudioLanguages
          }
          activeQualityScopeId={activeQualityScopeId}
          globalScoringPersona={globalScoringPersona}
          categoryPersonaSelections={categoryPersonaSelections}
          qualityProfileInheritValue={qualityProfileInheritValue}
          toProfileOptions={toProfileOptions}
          saveCategoryQualityProfileOverride={
            saveCategoryQualityProfileOverride
          }
          onFacetPersonaSave={handleFacetPersonaSave}
        />
      ) : effectiveContentSettingsSection === "renaming" ? (
        <RenameSettingsPanel
          activeQualityScopeId={activeQualityScopeId}
          mediaSettingsLoading={mediaSettingsLoading}
          mediaSettingsSaving={mediaSettingsSaving}
          categoryFolderTemplates={categoryFolderTemplates}
          handleFolderTemplateChange={handleFolderTemplateChange}
          categorySeasonFolderTemplates={categorySeasonFolderTemplates}
          handleSeasonFolderTemplateChange={handleSeasonFolderTemplateChange}
          categoryUseSeasonFolders={categoryUseSeasonFolders}
          handleUseSeasonFoldersChange={(checked) =>
            setCategoryUseSeasonFolders((previous) => ({
              ...previous,
              [activeQualityScopeId]: checked,
            }))
          }
          categorySpecialsFolderTemplates={categorySpecialsFolderTemplates}
          handleSpecialsFolderTemplateChange={handleSpecialsFolderTemplateChange}
          categoryRenameTemplates={categoryRenameTemplates}
          handleRenameTemplateChange={handleRenameTemplateChange}
          categoryRenameEnabled={categoryRenameEnabled}
          handleRenameEnabledChange={(checked) =>
            setCategoryRenameEnabled((previous) => ({
              ...previous,
              [activeQualityScopeId]: checked ? "true" : "false",
            }))
          }
          categoryRenameCollisionPolicies={categoryRenameCollisionPolicies}
          handleRenameCollisionPolicyChange={handleRenameCollisionPolicyChange}
          categoryRenameMissingMetadataPolicies={
            categoryRenameMissingMetadataPolicies
          }
          handleRenameMissingMetadataPolicyChange={
            handleRenameMissingMetadataPolicyChange
          }
          updateCategoryMediaProfileSettings={
            updateCategoryMediaProfileSettings
          }
        />
      ) : effectiveContentSettingsSection === "routing" ? (
        <div className="space-y-[18px]">
          <IndexerRoutingPanel
            scopeLabel={scopeLabel}
            activeQualityScopeId={activeQualityScopeId}
            indexers={indexers}
            activeScopeIndexerRouting={activeScopeIndexerRouting}
            activeScopeIndexerRoutingOrder={activeScopeIndexerRoutingOrder}
            indexerRoutingLoading={indexerRoutingLoading}
            indexerRoutingSaving={indexerRoutingSaving}
            onEnabledChange={handleIndexerEnabledChange}
            onCategoriesChange={handleIndexerCategoriesChange}
            onMoveUp={moveIndexerUp}
            onMoveDown={moveIndexerDown}
          />
          <DownloadClientRoutingPanel
            scopeLabel={scopeLabel}
            downloadClients={downloadClients}
            activeScopeRouting={activeScopeRouting}
            activeScopeRoutingOrder={activeScopeRoutingOrder}
            downloadClientRoutingLoading={downloadClientRoutingLoading}
            downloadClientRoutingSaving={downloadClientRoutingSaving}
            updateDownloadClientRoutingForScope={
              updateDownloadClientRoutingForScope
            }
            moveDownloadClientInScope={moveDownloadClientInScope}
          />
        </div>
      ) : effectiveContentSettingsSection === "library" ? (
        view === "movies" || view === "series" || view === "anime" ? (
          <MediaLibrarySettingsPanel
            facet={
              view === "movies"
                ? "MOVIE"
                : view === "series"
                  ? "SERIES"
                  : "ANIME"
            }
            settingsTitle={mediaLibrarySettingsTitle}
            libraries={libraries}
            librariesLoading={librariesLoading}
            onWideLayoutChange={setLibraryRoutingWide}
            onActiveLibraryNameChange={setLibraryCrumb}
            rootValidationLibraries={rootValidationLibraries}
            rootValidationLibrariesLoading={rootValidationLibrariesLoading}
            rootValidationUnavailable={rootValidationUnavailable}
            invalidRootPathsByLibraryId={invalidRootPathsByLibraryId}
            preferredLibraryId={
              selectedLibraryIds.length === 1
                ? selectedLibraryIds[0]
                : allLibrariesValue
            }
            allLibrariesValue={allLibrariesValue}
            loading={mediaSettingsLoading}
            saving={librarySettingsSaving}
            scanLoading={libraryScanLoading}
            scanNotice={libraryScanNotice}
            scanSummary={libraryScanSummary}
            localPathStyle={localPathStyle}
            qualityProfiles={qualityProfiles}
            downloadClients={libraryDownloadClients}
            downloadClientsLoading={libraryDownloadClientsLoading}
            canCreateLibrary={canManageCatalogSettings}
            canManageDownloadClientRouting={
              canManageSystemSettings || canManageCatalogSettings
            }
            loadLibrarySettings={state.loadLibrarySettings}
            loadFacetDownloadClientRouting={
              state.loadFacetDownloadClientRouting
            }
            onCreateLibrary={state.createLibrary}
            onUpdateLibrary={state.updateLibrary}
            onDeleteLibrary={state.deleteLibrary}
            onScan={handleLibraryScan}
          />
        ) : null
      ) : effectiveContentSettingsSection === "general" ? (
        <GeneralSettingsPanel
          activeQualityScopeId={activeQualityScopeId}
          mediaSettingsLoading={mediaSettingsLoading}
          categoryFillerPolicies={categoryFillerPolicies}
          handleFillerPolicyChange={handleFillerPolicyChange}
          categoryRecapPolicies={categoryRecapPolicies}
          handleRecapPolicyChange={handleRecapPolicyChange}
          categoryMonitorSpecials={categoryMonitorSpecials}
          handleMonitorSpecialsChange={handleMonitorSpecialsChange}
          categoryInterSeasonMovies={categoryInterSeasonMovies}
          handleInterSeasonMoviesChange={handleInterSeasonMoviesChange}
          categoryMonitorFillerMovies={categoryMonitorFillerMovies}
          handleMonitorFillerMoviesChange={handleMonitorFillerMoviesChange}
          nfoWriteOnImport={nfoWriteOnImport}
          handleNfoWriteChange={handleNfoWriteChange}
          plexmatchWriteOnImport={plexmatchWriteOnImport}
          handlePlexmatchWriteChange={handlePlexmatchWriteChange}
          importMode={importMode}
          handleImportModeChange={handleImportModeChange}
          localPathStyle={localPathStyle}
          setPermissionsLinux={setPermissionsLinux}
          handleSetPermissionsLinuxChange={handleSetPermissionsLinuxChange}
          fileChmod={fileChmod}
          handleFileChmodChange={handleFileChmodChange}
          folderChmod={folderChmod}
          handleFolderChmodChange={handleFolderChmodChange}
          chownGroup={chownGroup}
          handleChownGroupChange={handleChownGroupChange}
        />
          ) : null}
        </FacetSettingsSection>
      ) : view === "movies" || view === "series" || view === "anime" ? (
        <Card
          id={`media-overview-${view}`}
          className="flex min-h-0 flex-1 flex-col overflow-visible rounded-none border-0 bg-transparent p-0 shadow-none sm:overflow-hidden"
        >
          <CardContent className="flex min-h-0 flex-1 flex-col space-y-0 p-0">
            {keepCatalogHeaderOutsideWorkspace ? catalogHeader : null}
            <div
              className={cn(
                "flex min-h-0 flex-1 flex-col bg-transparent p-3 sm:p-4 lg:p-5",
                selectedTitleLayoutActive
                  ? "overflow-hidden"
                  : collectionViewMode === "poster"
                    ? "overflow-visible sm:overflow-hidden"
                    : undefined,
              )}
            >
              {(() => {
                const isMovieView = view === "movies";
                const overviewTargetView = isMovieView
                  ? ("movies" as const)
                  : view === "anime"
                    ? ("anime" as const)
                    : ("series" as const);
                const titleCatalogControlBar = (
                  <>
                    {catalogDiscoveryFlyoutAvailable ? (
                      <div className="my-3 flex shrink-0 justify-end">
                        <div className="flex min-w-0 flex-[999_1_28rem] overflow-hidden rounded-[11px] border !border-[rgba(var(--scry-accent-rgb),0.55)] bg-[var(--scry-inset)] shadow-none">
                      {catalogDiscoveryFlyoutAvailable ? (
                        <Sheet>
                          <SheetTrigger asChild>
                            <Button
                              type="button"
                              variant="outline"
                              className="h-10 min-w-0 flex-1 shrink-0 gap-2 rounded-none border-0 bg-transparent px-3 text-[13px] text-[var(--scry-body)] shadow-none transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
                            >
                              <SlidersHorizontal className="h-4 w-4 text-[var(--scry-accent-text)]" />
                              <span>{t("discovery.filters")}</span>
                            </Button>
                          </SheetTrigger>
                          <SheetContent
                            side="right"
                            className="w-[min(92vw,26rem)] max-w-none gap-0 border-l border-[var(--scry-border2)] bg-[var(--scry-surfD)] p-0 sm:max-w-none"
                          >
                            <SheetHeader className="sr-only">
                              <SheetTitle>{t("discovery.filters")}</SheetTitle>
                            </SheetHeader>
                            <CatalogFiltersPanel
                              libraries={libraries}
                              librariesLoading={librariesLoading}
                              selectedLibraryIds={selectedLibraryIds}
                              onSelectedLibraryIdsChange={
                                setSelectedLibraryIds
                              }
                              filters={advancedTitleFilters}
                              options={titleCatalogFilterOptions}
                              optionsError={titleCatalogFilterOptionsError}
                              onRetryOptions={retryTitleCatalogFilterOptions}
                              onFiltersChange={updateAdvancedTitleFilters}
                              searchValue={titleFilterInputValue}
                              onSearchValueChange={handleTitleFilterValueChange}
                              onClear={clearAdvancedTitleFilters}
                              quickFilters={titleQuickFilters}
                              quickFilterCounts={titleQuickFilterCounts}
                              quickFilterView={view}
                              onToggleQuickMonitoring={
                                toggleTitleQuickMonitoringFilter
                              }
                              onToggleQuickStatus={toggleTitleQuickStatusFilter}
                              onClearQuickFilters={clearTitleQuickFilters}
                              className="h-full"
                            />
                          </SheetContent>
                        </Sheet>
                      ) : null}
                      {catalogDiscoveryFlyoutAvailable ? (
                        <Sheet>
                          <SheetTrigger asChild>
                            <Button
                              type="button"
                              variant="outline"
                              className="h-10 min-w-0 flex-1 shrink-0 gap-2 rounded-none border-0 border-l border-[var(--scry-border2)] bg-transparent px-3 text-[13px] text-[var(--scry-body)] shadow-none transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
                            >
                              <Sparkles className="h-4 w-4 text-[var(--scry-accent-text)]" />
                              <span>{t("title.contextForYouTitle")}</span>
                            </Button>
                          </SheetTrigger>
                          <SheetContent
                            side="right"
                            className="w-[min(92vw,34rem)] max-w-none gap-0 border-l border-[var(--scry-border2)] bg-[var(--scry-surfD)] p-0 sm:max-w-none"
                          >
                            <SheetHeader className="sr-only">
                              <SheetTitle>
                                {t("title.contextForYouTitle")}
                              </SheetTitle>
                            </SheetHeader>
                            <TitleContextForYouPanel
                              discoveryGroups={deferredCatalogDiscoveryGroups}
                              view={view}
                              canManageTitle={canManageCatalogDiscovery}
                              canRequestMedia={canRequestCatalogDiscovery}
                              onDiscoveryAction={onCatalogDiscoveryAction}
                            />
                          </SheetContent>
                        </Sheet>
                      ) : null}
                        </div>
                      </div>
                    ) : null}
                  </>
                );
                let titleCollectionView: React.ReactNode;
                const overviewTitleResolutionPending =
                  (routeOverviewPending &&
                    selectedOverviewTitleId === null &&
                    activeOverviewTitle === null) ||
                  (selectedOverviewTitleId !== null &&
                    activeOverviewTitle === null &&
                    selectedOverviewDetailLoading);
                const titleListLoading = false;
                const titleListInitialLoadComplete =
                  catalogSurfacePhase === "content" ||
                  catalogSurfacePhase === "empty";

                if (
                  catalogSurfacePhase === "resolving" ||
                  overviewTitleResolutionPending
                ) {
                  titleCollectionView = (
                    <div
                      data-slot="title-list-bootstrap-loading"
                      className={cn(
                        "flex h-full w-full items-start justify-center px-4 pt-12",
                        selectedTitleCompactLayoutActive &&
                          !selectedTitleListInlineActive
                          ? "min-h-[18rem]"
                          : "min-h-[22rem]",
                      )}
                    >
                      <TitleCollectionLoadingState />
                    </div>
                  );
                } else if (
                  catalogSurfacePhase === "rootsMissing" ||
                  catalogSurfacePhase === "rootsInvalid"
                ) {
                  titleCollectionView = (
                    <div className="flex h-full w-full items-start justify-center px-4 pt-12">
                      <TitleCollectionEmptyState
                        t={t}
                        showConfigureRootsAction={
                          showConfigureRootFoldersAction
                        }
                        configureRootsReason={configureRootFoldersReason}
                        configureRootsHref={configureRootFoldersHref}
                      />
                    </div>
                  );
                } else if (catalogSurfacePhase === "error") {
                  titleCollectionView = (
                    <div className="flex h-full w-full items-start justify-center px-4 pt-12">
                      <TitleCollectionErrorState
                        t={t}
                        error={catalogSurfaceError}
                        onRetry={retryCatalogBootstrap}
                      />
                    </div>
                  );
                } else if (collectionViewMode === "poster") {
                  titleCollectionView = (
                    <PosterGrid
                      key={`${view}-poster-grid`}
                      titles={monitoredTitles}
                      catalogInitialLoadComplete={titleListInitialLoadComplete}
                      catalogHasMoreTitles={catalogHasMoreTitles}
                      catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                      onCatalogEndReached={loadMoreCatalogTitles}
                      onOpenOverview={handleOpenOverviewFromContext}
                      selectedTitleId={contextPanelSelectedTitleId}
                      contextPanelId={selectedTitleContextPanelId}
                      onSelectTitle={onSelectTitleForContextPanel}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      isDeletingById={isDeletingCatalogTitleById}
                      overviewTargetView={overviewTargetView}
                      showScanLibraryAction={
                        showEmptyStateActions && showInitialScanAction
                      }
                      showConfigureRootsAction={
                        showEmptyStateActions && showConfigureRootFoldersAction
                      }
                      configureRootsReason={configureRootFoldersReason}
                      configureRootsHref={configureRootFoldersHref}
                      onScanLibrary={scanLibrary}
                      scanLibraryLoading={libraryScanLoading}
                      scanLibraryDisabled={libraryScanDisabled}
                      scanLibraryNotice={libraryScanNotice}
                      scrollContainer={!selectedTitleLayoutActive}
                      activeDownloadTitleIds={activeDownloadTitleIds}
                    />
                  );
                } else if (collectionViewMode === "compact") {
                  titleCollectionView = (
                    <CompactTitleTable
                      key={`${view}-compact-title-table`}
                      view={view}
                      titles={monitoredTitles}
                      titleLoading={titleListLoading}
                      catalogHasMoreTitles={catalogHasMoreTitles}
                      catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                      onCatalogEndReached={loadMoreCatalogTitles}
                      sortKey={titleCatalogSortKey}
                      sortDirection={titleCatalogSortDirection}
                      onSortChange={updateTitleCatalogSort}
                      visibleColumns={effectiveVisibleTitleTableColumns}
                      onOpenOverview={onOpenOverview}
                      selectedTitleId={contextPanelSelectedTitleId}
                      contextPanelId={selectedTitleContextPanelId}
                      onSelectTitle={onSelectTitleForContextPanel}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      onToggleMonitored={toggleTitleMonitored}
                      onInteractiveSearch={runInteractiveSearchForTitle}
                      onQueueFromInteractive={queueExistingFromRelease}
                      onQueueAdditionalFromInteractive={
                        queueAdditionalFromRelease
                      }
                      isDeletingById={isDeletingCatalogTitleById}
                      isTogglingMonitoredById={isTogglingTitleMonitoredById}
                      selectedTitleIds={selectedTitleIds}
                      onToggleSelected={toggleTitleSelection}
                      onToggleSelectAll={toggleAllVisibleTitles}
                      selectionMode={multiSelectActive}
                      bulkActionBusy={bulkActionBusy}
                      showScanLibraryAction={
                        showEmptyStateActions && showInitialScanAction
                      }
                      showConfigureRootsAction={
                        showEmptyStateActions && showConfigureRootFoldersAction
                      }
                      configureRootsReason={configureRootFoldersReason}
                      configureRootsHref={configureRootFoldersHref}
                      onScanLibrary={scanLibrary}
                      scanLibraryLoading={libraryScanLoading}
                      scanLibraryDisabled={libraryScanDisabled}
                      scanLibraryNotice={libraryScanNotice}
                      activeDownloadTitleIds={activeDownloadTitleIds}
                    />
                  );
                } else {
                  titleCollectionView = (
                    <TitleTable
                      key={`${view}-poster-title-table`}
                      view={view}
                      titles={monitoredTitles}
                      titleLoading={titleListLoading}
                      catalogHasMoreTitles={catalogHasMoreTitles}
                      catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                      onCatalogEndReached={loadMoreCatalogTitles}
                      sortKey={titleCatalogSortKey}
                      sortDirection={titleCatalogSortDirection}
                      onSortChange={updateTitleCatalogSort}
                      visibleColumns={effectiveVisibleTitleTableColumns}
                      onOpenOverview={onOpenOverview}
                      selectedTitleId={contextPanelSelectedTitleId}
                      selectedPaneMode={selectedTitleFullTableInlineActive}
                      contextPanelId={selectedTitleContextPanelId}
                      onSelectTitle={onSelectTitleForContextPanel}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      onToggleMonitored={toggleTitleMonitored}
                      onInteractiveSearch={runInteractiveSearchForTitle}
                      onQueueFromInteractive={queueExistingFromRelease}
                      onQueueAdditionalFromInteractive={
                        queueAdditionalFromRelease
                      }
                      isDeletingById={isDeletingCatalogTitleById}
                      isTogglingMonitoredById={isTogglingTitleMonitoredById}
                      selectedTitleIds={selectedTitleIds}
                      onToggleSelected={toggleTitleSelection}
                      onToggleSelectAll={toggleAllVisibleTitles}
                      selectionMode={multiSelectActive}
                      bulkActionBusy={bulkActionBusy}
                      showScanLibraryAction={
                        showEmptyStateActions && showInitialScanAction
                      }
                      showConfigureRootsAction={
                        showEmptyStateActions && showConfigureRootFoldersAction
                      }
                      configureRootsReason={configureRootFoldersReason}
                      configureRootsHref={configureRootFoldersHref}
                      onScanLibrary={scanLibrary}
                      scanLibraryLoading={libraryScanLoading}
                      scanLibraryDisabled={libraryScanDisabled}
                      scanLibraryNotice={libraryScanNotice}
                      activeDownloadTitleIds={activeDownloadTitleIds}
                    />
                  );
                }
                const contextPanelGridTemplateColumns =
                  catalogDiscoveryInlineAvailable
                    ? "minmax(0,1fr) minmax(23rem,30rem)"
                    : undefined;
                const selectedTitleGridTemplateColumns =
                  selectedTitleListInlineActive || selectedTitlePosterInlineActive
                    ? "minmax(0,1fr) clamp(700px,50%,1030px)"
                    : undefined;
                const titleListDisclosure =
                  selectedTitleCompactLayoutActive &&
                  !selectedTitleListInlineActive ? (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="h-8 gap-2 rounded-[8px] border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 text-[12px] shadow-none"
                      aria-expanded={selectedTitleListDrawerOpen}
                      aria-controls={selectedTitleListDrawerId}
                      aria-label={
                        selectedTitleListDrawerOpen
                          ? t("title.hideTitleList")
                          : t("title.showTitleList")
                      }
                      title={
                        selectedTitleListDrawerOpen
                          ? t("title.hideTitleList")
                          : t("title.showTitleList")
                      }
                      onClick={() =>
                        setSelectedTitleListDrawerOpen((open) => !open)
                      }
                    >
                      <PanelLeftOpen className="h-4 w-4" />
                      <span>{t("title.contextListDisclosure")}</span>
                    </Button>
                  ) : undefined;
                const titleOverviewPaneClassName =
                  selectedTitleLayoutActive
                    ? selectedTitleListInlineActive ||
                      selectedTitlePosterInlineActive
                      ? "flex h-full min-h-0"
                      : "flex min-h-0 flex-1"
                    : contextPanelAvailable
                      ? collectionViewMode === "poster"
                        ? "flex h-full min-h-0"
                        : "flex h-full"
                      : "hidden";
                const titleOverviewPane = multiSelectActive ? (
                  <section
                    id={selectedTitleContextPanelId}
                    aria-label={t("title.bulkSelectionPanelTitle")}
                    className={cn(
                      "min-h-0 w-full min-w-0 flex-col overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-surfD)]",
                      titleOverviewPaneClassName,
                    )}
                  >
                    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-6 p-6 text-center">
                      <TitleBulkPosterStack titles={bulkPosterStackTitles} />
                      <div className="space-y-1">
                        <div className="text-[17px] font-semibold text-[var(--scry-ink2)]">
                          {t("title.bulkSelectionCount", {
                            count: selectedTitleIds.size,
                          })}
                        </div>
                        <p className="max-w-[18rem] text-[13px] text-[var(--scry-muted3)]">
                          {t("title.bulkSelectionPanelHint")}
                        </p>
                      </div>
                      <div className="flex w-full max-w-[18rem] flex-col gap-2">
                        <Button
                          onClick={() => void bulkMonitorTitles(true)}
                          disabled={bulkActionBusy}
                          className="justify-center gap-2"
                        >
                          <Eye className="h-4 w-4" />
                          {t("title.monitorAction")}
                        </Button>
                        <Button
                          variant="outline"
                          onClick={() => void bulkMonitorTitles(false)}
                          disabled={bulkActionBusy}
                          className="justify-center gap-2"
                        >
                          <EyeOff className="h-4 w-4" />
                          {t("title.unmonitorAction")}
                        </Button>
                        <Button
                          variant="outline"
                          onClick={openBulkTitleEdit}
                          disabled={bulkActionBusy}
                          className="justify-center gap-2"
                        >
                          <Pencil className="h-4 w-4" />
                          {t("label.edit")}
                        </Button>
                        {canRenameSelectedTitles ? (
                          <Button
                            variant="outline"
                            onClick={openBulkTitleRename}
                            disabled={bulkActionBusy}
                            className="justify-center gap-2"
                          >
                            <FolderPen className="h-4 w-4" />
                            {t("title.renameAction")}
                          </Button>
                        ) : null}
                        <Button
                          variant="destructive"
                          onClick={openBulkTitleDelete}
                          disabled={bulkActionBusy}
                          className="justify-center gap-2"
                        >
                          <Trash2 className="h-4 w-4" />
                          {t("label.delete")}
                        </Button>
                      </div>
                      <Button
                        variant="ghost"
                        onClick={clearSelectedTitles}
                        disabled={bulkActionBusy}
                        className="gap-2 text-[var(--scry-muted2)]"
                      >
                        <X className="h-4 w-4" />
                        {t("label.clear")}
                      </Button>
                    </div>
                  </section>
                ) : seriesSidePanelTitleId !== null ? (
                    <section
                      id={selectedTitleContextPanelId}
                      aria-label={t("title.contextPanelTitle")}
                      className={cn(
                        "min-h-0 w-full min-w-0 flex-col overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-surfD)]",
                        titleOverviewPaneClassName,
                      )}
                    >
                      <div
                        data-slot="title-context-scroll"
                        className="relative min-h-0 flex-1 overflow-y-auto p-4 pb-[max(5rem,calc(1rem+env(safe-area-inset-bottom)))] sm:p-5 sm:pb-5"
                      >
                        {titleListDisclosure ? (
                          <div className="mb-3 flex items-center">
                            {titleListDisclosure}
                          </div>
                        ) : null}
                        <SeriesOverviewContainer
                          titleId={seriesSidePanelTitleId}
                          fullBleedHero
                          initialEpisodeId={routeOverviewEpisodeId}
                          onTitleNotFound={handleSelectedOverviewBackToList}
                          onBackToList={handleSelectedOverviewBackToList}
                          onTitleResolved={(resolvedTitle) => {
                            if (resolvedTitle.id !== seriesSidePanelTitleId) {
                              onOpenOverview(view, resolvedTitle);
                            }
                          }}
                        />
                      </div>
                    </section>
                  ) : (
                    <TitleContextPanel
                      id={selectedTitleContextPanelId}
                      title={activeOverviewTitle}
                      discoveryGroups={deferredCatalogDiscoveryGroups}
                      libraries={libraries}
                      librariesLoading={librariesLoading}
                      selectedLibraryIds={selectedLibraryIds}
                      onSelectedLibraryIdsChange={setSelectedLibraryIds}
                      advancedFilters={advancedTitleFilters}
                      filterOptions={titleCatalogFilterOptions}
                      filterOptionsError={titleCatalogFilterOptionsError}
                      onRetryFilterOptions={retryTitleCatalogFilterOptions}
                      onAdvancedFiltersChange={updateAdvancedTitleFilters}
                      onClearAdvancedFilters={clearAdvancedTitleFilters}
                      view={view}
                      blocklistEntries={selectedOverviewBlocklistEntries}
                      externalSubtitles={selectedOverviewExternalSubtitles}
                      isTogglingMonitored={
                        activeOverviewTitle
                          ? isTogglingTitleMonitoredById[
                              activeOverviewTitle.id
                            ] === true
                          : false
                      }
                      isDeleting={
                        activeOverviewTitle
                          ? isDeletingCatalogTitleById[
                              activeOverviewTitle.id
                            ] === true
                          : false
                      }
                      onUpdateTitleOptions={updateMovieTitleOptions}
                      onTitleOptionsChanged={refreshMovieTitleOptions}
                      onToggleMonitored={toggleTitleMonitored}
                      onAutoQueue={queueExisting}
                      onRefreshTitles={handleRefreshTitles}
                      onRefreshSubtitles={refreshSelectedOverviewExternalSubtitles}
                      onDeleteMediaFile={deleteSelectedOverviewMediaFile}
                      deletingMediaFileIds={pendingMediaFileDeletionIds}
                      onMakePrimaryMediaFile={
                        makeSelectedOverviewMovieFilePrimary
                      }
                      primaryMediaFileUpdatingId={
                        selectedOverviewPrimaryMovieFileUpdatingId
                      }
                      onPreviewRename={previewTitleRename}
                      onApplyRename={applyTitleRename}
                      refreshLoading={titleLoading || catalogBootstrapLoading}
                      onInteractiveSearch={runInteractiveSearchForTitle}
                      onQueueFromInteractive={queueExistingFromRelease}
                      onQueueAdditionalFromInteractive={
                        queueAdditionalFromRelease
                      }
                      bulkActionBusy={bulkActionBusy}
                      onDelete={handleDeleteCatalogTitle}
                      onClearSelection={onCloseOverview}
                      canManageTitle={canManageTitle}
                      canManageTitlesInLibrary={canManageTitlesInLibrary}
                      canRequestMedia={canRequestCatalogDiscovery}
                      manageableDiscoveryFacets={manageableDiscoveryFacetSet}
                      requestableDiscoveryFacets={requestableDiscoveryFacetSet}
                      onDiscoveryAction={onCatalogDiscoveryAction}
                      titleFilterValue={titleFilterInputValue}
                      onTitleFilterValueChange={handleTitleFilterValueChange}
                      quickFilters={titleQuickFilters}
                      quickFilterCounts={titleQuickFilterCounts}
                      quickFilterView={view}
                      onToggleQuickMonitoring={toggleTitleQuickMonitoringFilter}
                      onToggleQuickStatus={toggleTitleQuickStatusFilter}
                      onClearQuickFilters={clearTitleQuickFilters}
                      titleListDisclosure={titleListDisclosure}
                      onCollapseCatalogRail={() =>
                        setCatalogContextRailCollapsed(true)
                      }
                      className={titleOverviewPaneClassName}
                    />
                  );

                return (
                  <div
                    ref={titleLayoutRef}
                    className={cn(
                      selectedTitleLayoutActive
                        ? cn(
                            "relative min-h-0 gap-4",
                            selectedTitleListInlineActive ||
                              selectedTitlePosterInlineActive
                              ? "grid h-full items-stretch"
                              : "flex min-h-0 flex-1 flex-col overflow-visible min-[981px]:overflow-hidden",
                          )
                        : "grid min-h-0 gap-4",
                      !selectedTitleLayoutActive &&
                        (collectionViewMode === "poster"
                          ? "items-stretch sm:h-full"
                          : "h-full"),
                    )}
                    style={
                      selectedTitleGridTemplateColumns ||
                      contextPanelGridTemplateColumns
                        ? {
                            gridTemplateColumns:
                              selectedTitleGridTemplateColumns ??
                              contextPanelGridTemplateColumns,
                          }
                        : undefined
                    }
                  >
                    {selectedTitleCompactLayoutActive &&
                    selectedTitleListDrawerOpen &&
                    !selectedTitleListInlineActive ? (
                      <button
                        type="button"
                        className="absolute inset-0 z-10 bg-black/45 backdrop-blur-[2px]"
                        aria-label={t("label.close")}
                        onClick={() => setSelectedTitleListDrawerOpen(false)}
                      />
                    ) : null}
                    <div
                      id={selectedTitleListDrawerId}
                      ref={selectedTitleListDrawerRef}
                      role={
                        selectedTitleListDrawerModeActive ? "dialog" : "region"
                      }
                      aria-modal={
                        selectedTitleListDrawerModeActive ? true : undefined
                      }
                      aria-label={t("title.contextTitleList")}
                      tabIndex={selectedTitleListDrawerModeActive ? -1 : undefined}
                      className={cn(
                        "min-w-0",
                        collectionViewMode === "poster"
                          ? selectedTitlePosterLayoutActive
                            ? "h-full min-h-0 overflow-y-auto pr-1"
                            : selectedTitleLayoutActive
                              ? ""
                              : "flex min-h-0 flex-col sm:h-full"
                          : "flex min-h-0 flex-col",
                        selectedTitleCompactLayoutActive &&
                          (selectedTitleListInlineActive
                            ? "flex min-h-0 flex-col"
                            : selectedTitleListDrawerOpen
                              ? "absolute bottom-3 left-3 top-3 z-30 flex w-[min(360px,82%)] min-w-0 flex-col overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] p-2 shadow-[0_24px_70px_rgba(0,0,0,0.62)] motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-left-3"
                              : "hidden"),
                      )}
                    >
                      {!keepCatalogHeaderOutsideWorkspace ? catalogHeader : null}
                      {titleCatalogControlBar}
                      <div
                        className={cn(
                          "min-w-0",
                          collectionViewMode === "poster"
                            ? selectedTitlePosterLayoutActive
                              ? ""
                              : selectedTitleLayoutActive
                                ? ""
                                : "min-h-0 flex-1"
                            : "min-h-0 flex-1",
                        )}
                      >
                        {titleCollectionView}
                      </div>
                    </div>
                    {titleOverviewPane}
                  </div>
                );
              })()}
            </div>
          </CardContent>
        </Card>
      ) : (
        <AddTitleForm
          titleNameForQueue={titleNameForQueue}
          setTitleNameForQueue={setTitleNameForQueue}
          queueFacet={queueFacet}
          setQueueFacet={setQueueFacet}
          monitoredForQueue={monitoredForQueue}
          setMonitoredForQueue={setMonitoredForQueue}
          seasonFoldersForQueue={seasonFoldersForQueue}
          setSeasonFoldersForQueue={setSeasonFoldersForQueue}
          minAvailabilityForQueue={minAvailabilityForQueue}
          setMinAvailabilityForQueue={setMinAvailabilityForQueue}
          onAddSubmit={onAddSubmit}
          tvdbCandidates={tvdbCandidates}
          addTvdbCandidateToCatalog={addTvdbCandidateToCatalog}
          titleFilter={titleFilter}
          onTitleFilterChange={handleTitleFilterChange}
          onRefreshTitles={handleRefreshTitles}
          titleLoading={titleLoading}
          monitoredTitles={monitoredTitles}
          onOpenOverview={onOpenOverview}
          queueExisting={queueExisting}
        />
      )}
    </div>
  );
}
