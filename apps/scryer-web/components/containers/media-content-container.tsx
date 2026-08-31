import * as React from "react";
import { MediaContentView } from "@/components/views/media-content-view";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";
import type { MediaRenamePlan } from "@/components/common/media-rename-plan-panel";
import {
  addTitleMutation,
  renameTitlesMutation,
  buildSetTitleMonitoredBatchMutation,
  buildUpdateTitleBatchMutation,
  updateTitleMutation,
  createLibraryMutation,
  deleteMediaFileMutation,
  deleteLibraryMutation,
  queueBestReleaseMutation,
  queueExistingMutation,
  queueReplacementMutation,
  scanLibraryMutation,
  deleteTitlesMutation,
  setPrimaryMovieFileMutation,
  setTitleMonitoredMutation,
  updateLibraryMutation,
  updateRuleSetMutation,
} from "@/lib/graphql/mutations";
import {
  browsePathQuery,
  deleteMediaFilePreviewQuery,
  deleteTitlePreviewQuery,
  downloadClientRoutingQuery,
  jobRunEventsSubscription,
  jobRunsQuery,
  librariesQuery,
  libraryDownloadClientsQuery,
  librarySettingsQuery,
  externalSubtitlesQuery,
  mediaRenamePreviewQuery,
  catalogDiscoveryQuery,
  discoveryItemDetailQuery,
  ruleSetsQuery,
  routingPageInitQuery,
  movieSidePanelTitleQuery,
  titleReleaseBlocklistQuery,
  titleCatalogFilterOptionsQuery,
  buildTitlesQuery,
} from "@/lib/graphql/queries";
import { selectedOverviewUsesMovieRecord } from "@/lib/utils/selected-overview-policy";
import {
  CATEGORY_SCOPE_MAP,
  QUALITY_PROFILE_INHERIT_VALUE,
  viewToFacet,
} from "@/lib/constants/settings";
import {
  CATALOG_TITLES_REFRESH_EVENT,
  catalogTitlesRefreshDetail,
} from "@/lib/events/catalog-titles";
import { isAbortError } from "@/lib/graphql/urql-client";
import { runIterativeReleaseSearch } from "@/lib/graphql/release-search";
import type { InteractiveSearchProgress } from "@/lib/graphql/release-search";
import { useClient } from "urql";
import type {
  ContentSettingsSection,
  OverviewTitleTarget,
  ViewId,
} from "@/components/root/types";
import { toProfileOptions } from "@/lib/utils/quality-profiles";
import {
  discoveryItemFacet,
  metadataResultForDiscoveryItem,
} from "@/lib/utils/discovery-actions";
import {
  normalizeLibraryFilterSelection,
  singleSelectedLibraryId,
} from "@/lib/utils/library-filter";
import {
  activeLibraryScanSessionsForSelection,
  didActiveLibraryScanSessionEnd,
  isLibraryScanTargetBusy,
  libraryScanProgressKey,
  libraryScanSessionIds,
} from "@/lib/utils/library-scan-sessions";
import {
  EMPTY_TITLE_ADVANCED_FILTERS,
  EMPTY_TITLE_QUICK_FILTERS,
  buildTitleCatalogQueryVariables,
  titleCatalogProjectionForTable,
  titleCatalogQueryKey,
  type TitleCatalogAdvancedFilters,
} from "@/lib/utils/title-catalog-query";
import {
  hasPrimaryMediaFile,
  releaseQueueScopeInput,
} from "@/lib/utils/release-queue-scope";
import { validateLibraryRootPaths } from "@/lib/utils/library-root-validation";
import {
  catalogRootValidationState,
  configuredCatalogLibraries,
  resolveCatalogSurfacePhase,
  type CatalogRootValidationState,
  type CatalogSurfacePhase,
} from "@/lib/utils/catalog-bootstrap-policy";
import { isMediaSettingsSection } from "@/lib/utils/routes";
import { useBulkDelete } from "@/lib/hooks/use-bulk-delete";
import { useBulkRename } from "@/lib/hooks/use-bulk-rename";
import { useDownloadClientRouting } from "@/lib/hooks/use-download-client-routing";
import { useIndexerRouting } from "@/lib/hooks/use-indexer-routing";
import { useMediaSettings } from "@/lib/hooks/use-media-settings";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { useQueueFormState } from "@/lib/hooks/use-queue-form-state";
import { useTitleListReactiveRefresh } from "@/lib/hooks/use-title-list-reactive-refresh";
import { useTitleManagementState } from "@/lib/hooks/use-title-management-state";
import { fetchTitleMoreLikeThis } from "@/lib/title-overview-loader";
import type {
  DownloadClientRecord,
  DownloadClientRoutingEntry,
  JobRun,
  LibraryRecord,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  Release,
  RootFolderOption,
  TitleReleaseBlocklistEntry,
  TitleRecord,
  TitleCatalogFilterOptionsRecord,
  CatalogDiscoveryGroup,
  CatalogDiscoveryInput,
  CatalogDiscoveryItem,
  CatalogDiscoveryPayload,
  Facet,
  RuleSetRecord,
} from "@/lib/types";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { DeletePreview } from "@/lib/types/delete-preview";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import { BulkRenamePreviewSummary } from "@/components/common/bulk-rename-preview-summary";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useLibraryScanProgress } from "@/lib/context/library-scan-progress-context";
import { useSearchContext } from "@/lib/context/search-context";
import {
  reactiveRefreshEpoch,
  useReactiveRefresh,
} from "@/lib/context/reactive-refresh-context";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import { useOverviewWindowScrollRestoration } from "@/lib/hooks/use-overview-window-scroll-restoration";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
import { titleMatchesOptionUpdates } from "@/lib/utils/title-edit-dialog";
import { isTerminalJobRunStatus, normalizeJobRun } from "@/lib/utils/job-runs";
import { toast } from "sonner";
import { BulkTitleEditDialog } from "@/components/views/media-content/bulk-title-edit-dialog";
import {
  readStoredContentViewMode,
  writeStoredContentViewMode,
  type ContentViewMode,
} from "@/components/views/media-content/content-view-mode";
import {
  filterTitlesByQuickFilters,
  type TitleQuickFilterCounts,
  type TitleQuickFilters,
} from "@/components/views/media-content/title-quick-filters";
import {
  DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
  defaultSortDirectionForTitleKey,
  isTitleTableColumnSupportedForView,
  type TitleTableColumnKey,
  type TitleTableSortDirection,
  type TitleTableSortKey,
  type TitleTableVisibleColumns,
} from "@/components/views/media-content/title-table-shared";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";

const HYDRATION_POSTER_REFRESH_WINDOW_MS = 5 * 60 * 1000;
const HYDRATION_POSTER_REFRESH_INTERVAL_MS = 2_500;
const TITLE_DELETION_JOB_FALLBACK_DELAYS_MS = [
  10_000, 60_000, 180_000,
] as const;
const TITLE_CATALOG_PAGE_SIZE = 72;
const TITLE_CATALOG_FILTER_DEBOUNCE_MS = 250;
const LIBRARY_SCAN_TITLE_REFRESH_THROTTLE_MS = 5_000;
const ALL_LIBRARIES_VALUE = "__all__";

const EMPTY_TITLE_CATALOG_FILTER_OPTIONS: TitleCatalogFilterOptionsRecord = {
  genres: [],
  themes: [],
  minimumYear: null,
  maximumYear: null,
};

function createAdvancedTitleFiltersByFacet(): Record<
  Facet,
  TitleCatalogAdvancedFilters
> {
  return {
    MOVIE: { ...EMPTY_TITLE_ADVANCED_FILTERS },
    SERIES: { ...EMPTY_TITLE_ADVANCED_FILTERS },
    ANIME: { ...EMPTY_TITLE_ADVANCED_FILTERS },
  };
}

function createSelectedLibraryIdsByFacet(): Record<Facet, string[]> {
  return { MOVIE: [], SERIES: [], ANIME: [] };
}

type CatalogBootstrapState = {
  key: string;
  phase: CatalogSurfacePhase;
  error: string | null;
};

type MediaContentContainerProps = {
  view: ViewId;
  contentSettingsSection: ContentSettingsSection;
  canManageConfig: boolean;
  canManageSystemSettings: boolean;
  canManageCatalogSettings: boolean;
  canManageLibrarySettings: boolean;
  canViewCatalog: boolean;
  canManageTitle: boolean;
  canManageTitlesInLibrary: (libraryId: string | null | undefined) => boolean;
  canRequestMedia: boolean;
  authorizationSignature: string;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  routeOverviewTitleId: string | null;
  routeOverviewPending: boolean;
  routeOverviewEpisodeId: string | null;
  onCloseOverview: () => void;
};

type SelectedOverviewMediaFile = NonNullable<TitleRecord["mediaFiles"]>[number];

type SelectedOverviewMediaFileDeleteTarget = {
  titleId: string;
  file: SelectedOverviewMediaFile;
};

type TitleCatalogState = {
  queryKey: string;
  hasMore: boolean;
  nextOffset: number;
  totalCount: number;
  managedBytes: number;
  filterCounts: TitleQuickFilterCounts;
  loadingMore: boolean;
};

type CatalogReloadOptions = {
  advancedFilters?: TitleCatalogAdvancedFilters;
  mode?: "initial" | "background";
};

function markCatalogTiming(name: string) {
  if (import.meta.env.DEV && typeof performance !== "undefined") {
    performance.mark(`catalog:${name}`);
  }
}

type TitleCatalogSortState = {
  key: TitleTableSortKey;
  direction: TitleTableSortDirection;
};

const emptyTitleCatalogState: TitleCatalogState = {
  queryKey: "",
  hasMore: false,
  nextOffset: 0,
  totalCount: 0,
  managedBytes: 0,
  filterCounts: {
    all: 0,
    monitored: 0,
    unmonitored: 0,
    continuing: 0,
    ended: 0,
  },
  loadingMore: false,
};

function titleCatalogFilterCountsFromPage(
  page: { filterCounts?: Partial<TitleQuickFilterCounts> | null },
  fallback: TitleQuickFilterCounts = emptyTitleCatalogState.filterCounts,
): TitleQuickFilterCounts {
  const counts = page.filterCounts;
  return {
    all: typeof counts?.all === "number" ? counts.all : fallback.all,
    monitored:
      typeof counts?.monitored === "number"
        ? counts.monitored
        : fallback.monitored,
    unmonitored:
      typeof counts?.unmonitored === "number"
        ? counts.unmonitored
        : fallback.unmonitored,
    continuing:
      typeof counts?.continuing === "number"
        ? counts.continuing
        : fallback.continuing,
    ended: typeof counts?.ended === "number" ? counts.ended : fallback.ended,
  };
}

const defaultTitleCatalogSortState: TitleCatalogSortState = {
  key: "name",
  direction: "asc",
};

const CATALOG_DISCOVERY_LIMIT_PER_GROUP = 12;
const CATALOG_DISCOVERY_MAX_GROUPS = 6;
const DISCOVERY_FACETS: Facet[] = ["MOVIE", "SERIES", "ANIME"];

type ActiveCatalogListFilters = {
  facet: TitleRecord["facet"];
  query: string;
  libraryIds: readonly string[];
};

function mergePreferLoadedImageFields(
  current: TitleRecord,
  incoming: TitleRecord,
): TitleRecord {
  const incomingHasPoster = Boolean(
    incoming.posterUrl || incoming.posterSourceUrl,
  );
  const incomingHasBackground = Boolean(
    incoming.backgroundUrl || incoming.backgroundSourceUrl,
  );

  return {
    ...incoming,
    posterUrl: incomingHasPoster
      ? incoming.posterUrl
      : (current.posterUrl ?? null),
    posterSourceUrl: incomingHasPoster
      ? incoming.posterSourceUrl
      : (current.posterSourceUrl ?? null),
    backgroundUrl: incomingHasBackground
      ? incoming.backgroundUrl
      : (current.backgroundUrl ?? null),
    backgroundSourceUrl: incomingHasBackground
      ? incoming.backgroundSourceUrl
      : (current.backgroundSourceUrl ?? null),
    overview:
      incoming.overview === undefined ? current.overview : incoming.overview,
    runtimeMinutes:
      incoming.runtimeMinutes === undefined
        ? current.runtimeMinutes
        : incoming.runtimeMinutes,
    language:
      incoming.language === undefined ? current.language : incoming.language,
    firstAired:
      incoming.firstAired === undefined
        ? current.firstAired
        : incoming.firstAired,
    network:
      incoming.network === undefined ? current.network : incoming.network,
    studio: incoming.studio === undefined ? current.studio : incoming.studio,
    country:
      incoming.country === undefined ? current.country : incoming.country,
    metadataLanguage:
      incoming.metadataLanguage === undefined
        ? current.metadataLanguage
        : incoming.metadataLanguage,
    monitorType:
      incoming.monitorType === undefined
        ? current.monitorType
        : incoming.monitorType,
    useSeasonFolders:
      incoming.useSeasonFolders === undefined
        ? current.useSeasonFolders
        : incoming.useSeasonFolders,
    monitorSpecials:
      incoming.monitorSpecials === undefined
        ? current.monitorSpecials
        : incoming.monitorSpecials,
    interSeasonMovies:
      incoming.interSeasonMovies === undefined
        ? current.interSeasonMovies
        : incoming.interSeasonMovies,
    fillerPolicy:
      incoming.fillerPolicy === undefined
        ? current.fillerPolicy
        : incoming.fillerPolicy,
    recapPolicy:
      incoming.recapPolicy === undefined
        ? current.recapPolicy
        : incoming.recapPolicy,
    collections:
      incoming.collections === undefined
        ? current.collections
        : incoming.collections,
    mediaFiles:
      incoming.mediaFiles === undefined
        ? current.mediaFiles
        : incoming.mediaFiles,
    sizeBytes:
      incoming.sizeBytes === undefined ? current.sizeBytes : incoming.sizeBytes,
    imdbId: incoming.imdbId === undefined ? current.imdbId : incoming.imdbId,
    externalIds:
      incoming.externalIds === undefined ? current.externalIds : incoming.externalIds,
    canonicalTags:
      incoming.canonicalTags === undefined
        ? current.canonicalTags
        : incoming.canonicalTags,
    ratings: incoming.ratings === undefined ? current.ratings : incoming.ratings,
    // Catalog list refreshes omit credits; treating that as "no cast" would
    // blank the overview rail on every list refresh.
    credits: incoming.credits === undefined ? current.credits : incoming.credits,
    metadataFetchedAt: incoming.metadataFetchedAt ?? current.metadataFetchedAt,
  };
}

function mergeCatalogTitlesPreservingImages(
  currentTitles: TitleRecord[],
  incomingTitles: TitleRecord[],
): TitleRecord[] {
  const currentById = new Map(currentTitles.map((title) => [title.id, title]));

  return incomingTitles.map((title) => {
    const current = currentById.get(title.id);
    if (!current) {
      return title;
    }
    const merged = mergePreferLoadedImageFields(current, title);
    return JSON.stringify(current) === JSON.stringify(merged) ? current : merged;
  });
}

function appendCatalogTitlesPreservingImages(
  currentTitles: TitleRecord[],
  incomingTitles: TitleRecord[],
): TitleRecord[] {
  const currentById = new Map(currentTitles.map((title) => [title.id, title]));
  const next = [...currentTitles];

  for (const title of incomingTitles) {
    const current = currentById.get(title.id);
    if (current) {
      const merged = mergePreferLoadedImageFields(current, title);
      const nextTitle =
        JSON.stringify(current) === JSON.stringify(merged) ? current : merged;
      currentById.set(title.id, nextTitle);
      const index = next.findIndex((candidate) => candidate.id === title.id);
      if (index !== -1) {
        next[index] = nextTitle;
      }
      continue;
    }
    currentById.set(title.id, title);
    next.push(title);
  }

  return next;
}

function buildActiveCatalogListFilters(
  facet: TitleRecord["facet"],
  query: string,
  libraryIds: readonly string[],
): ActiveCatalogListFilters {
  return {
    facet,
    query: query.trim().toLocaleLowerCase(),
    libraryIds: [...libraryIds],
  };
}

function catalogTitleMatchesActiveListFilters(
  title: TitleRecord,
  filters: ActiveCatalogListFilters,
): boolean {
  if (title.facet !== filters.facet) {
    return false;
  }

  if (
    filters.libraryIds.length > 0 &&
    !filters.libraryIds.includes(title.libraryId)
  ) {
    return false;
  }

  return (
    filters.query.length === 0 ||
    title.name.toLocaleLowerCase().includes(filters.query)
  );
}

function upsertCatalogTitleRecord(
  titles: TitleRecord[],
  title: TitleRecord,
  filters?: ActiveCatalogListFilters,
): TitleRecord[] {
  const existingIndex = titles.findIndex((item) => item.id === title.id);
  if (filters && !catalogTitleMatchesActiveListFilters(title, filters)) {
    if (existingIndex === -1) {
      return titles;
    }
    const next = [...titles];
    next.splice(existingIndex, 1);
    return next;
  }

  const next = [...titles];
  if (existingIndex === -1) {
    next.push(title);
  } else {
    next[existingIndex] = mergePreferLoadedImageFields(
      next[existingIndex],
      title,
    );
  }
  return next;
}

function isPendingHydrationPosterTitle(
  title: TitleRecord,
  nowMs: number,
): boolean {
  if (
    title.posterUrl ||
    title.posterSourceUrl ||
    title.metadataFetchedAt != null
  ) {
    return false;
  }

  const createdAtMs = title.createdAt
    ? Date.parse(title.createdAt)
    : Number.NaN;
  if (!Number.isFinite(createdAtMs)) {
    return true;
  }

  return nowMs - createdAtMs <= HYDRATION_POSTER_REFRESH_WINDOW_MS;
}

function hasSelectedTitlePanelDetails(title: TitleRecord): boolean {
  return title.canonicalTags !== undefined;
}

function hasSelectedTitleMovieMediaDetails(title: TitleRecord): boolean {
  return title.mediaFiles !== undefined;
}

function sameIdSet(
  left: ReadonlySet<string>,
  right: ReadonlySet<string>,
): boolean {
  if (left.size !== right.size) {
    return false;
  }

  for (const value of left) {
    if (!right.has(value)) {
      return false;
    }
  }

  return true;
}

function sameStringArray(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function batchItemAlias(index: number): string {
  return `item${index}`;
}

function batchFailureDetail(error: unknown): string | null {
  if (error instanceof Error) {
    const message = error.message.trim();
    return message || null;
  }

  if (typeof error === "string") {
    const trimmed = error.trim();
    return trimmed || null;
  }

  return null;
}

function withFailureDetail(message: string, detail: string | null): string {
  return detail ? `${message} ${detail}` : message;
}

function libraryRootsInput(roots: RootFolderOption[]) {
  return roots
    .map((root) => ({
      path: root.path.trim(),
      isDefault: root.isDefault,
    }))
    .filter((root) => root.path.length > 0);
}

function librarySettingsInput(
  settings: LibrarySettingsDraft | undefined,
): LibrarySettingsDraft | undefined {
  if (!settings) {
    return undefined;
  }
  return {
    requiredAudioLanguages: settings.requiredAudioLanguages,
    metadataLanguage: settings.metadataLanguage,
    useSeasonFolders: settings.useSeasonFolders,
    qualityProfileId: settings.qualityProfileId,
    requestQualityProfileIds: settings.requestQualityProfileIds,
    scoringPersona: settings.scoringPersona,
    fillerPolicy: settings.fillerPolicy,
    recapPolicy: settings.recapPolicy,
    monitorSpecials: settings.monitorSpecials,
    interSeasonMovies: settings.interSeasonMovies,
    monitorFillerMovies: settings.monitorFillerMovies,
    nfoWriteOnImport: settings.nfoWriteOnImport,
    plexmatchWriteOnImport: settings.plexmatchWriteOnImport,
    importMode: settings.importMode,
    setPermissionsLinux: settings.setPermissionsLinux,
    fileChmod: settings.fileChmod,
    folderChmod: settings.folderChmod,
    chownGroup: settings.chownGroup,
    indexerRouting: settings.indexerRouting,
    downloadClientRouting: settings.downloadClientRouting,
  };
}

function splitSucceededTitleIds(
  targets: TitleRecord[],
  predicate: (title: TitleRecord) => boolean,
): { succeededIds: string[]; failedIds: string[] } {
  const succeededIds: string[] = [];
  const failedIds: string[] = [];

  targets.forEach((title) => {
    if (predicate(title)) {
      succeededIds.push(title.id);
    } else {
      failedIds.push(title.id);
    }
  });

  return { succeededIds, failedIds };
}

function inferMonitoredBatchOutcome(
  targets: TitleRecord[],
  refreshedTitles: TitleRecord[],
  monitored: boolean,
): { succeededIds: string[]; failedIds: string[] } {
  const refreshedById = new Map(
    refreshedTitles.map((title) => [title.id, title]),
  );
  return splitSucceededTitleIds(
    targets,
    (title) => refreshedById.get(title.id)?.monitored === monitored,
  );
}

function inferTitleUpdateBatchOutcome(
  targets: TitleRecord[],
  refreshedTitles: TitleRecord[],
  changes: TitleOptionUpdates,
): { succeededIds: string[]; failedIds: string[] } {
  const refreshedById = new Map(
    refreshedTitles.map((title) => [title.id, title]),
  );
  return splitSucceededTitleIds(targets, (title) => {
    const refreshed = refreshedById.get(title.id);
    return refreshed !== undefined && titleMatchesOptionUpdates(refreshed, changes);
  });
}

function aggregateDeletePreviews(
  previews: DeletePreview[],
): DeletePreview | null {
  if (previews.length === 0) {
    return null;
  }

  const samplePaths = Array.from(
    new Set(previews.flatMap((preview) => preview.samplePaths)),
  ).slice(0, 12);
  const typedPrompt =
    previews.find((preview) => preview.requiresTypedConfirmation)
      ?.typedConfirmationPrompt ?? null;
  const mediaCount = previews.reduce(
    (sum, preview) => sum + preview.mediaCount,
    0,
  );
  const requiresTypedConfirmation =
    mediaCount > 50 ||
    previews.some((preview) => preview.requiresTypedConfirmation);

  return {
    fingerprint: "",
    totalFileCount: previews.reduce(
      (sum, preview) => sum + preview.totalFileCount,
      0,
    ),
    mediaCount,
    subtitleCount: previews.reduce(
      (sum, preview) => sum + preview.subtitleCount,
      0,
    ),
    imageCount: previews.reduce((sum, preview) => sum + preview.imageCount, 0),
    otherCount: previews.reduce((sum, preview) => sum + preview.otherCount, 0),
    directoryCount: previews.reduce(
      (sum, preview) => sum + preview.directoryCount,
      0,
    ),
    requiresTypedConfirmation,
    typedConfirmationPrompt:
      typedPrompt ??
      (requiresTypedConfirmation
        ? "Type DELETE to confirm this large delete."
        : null),
    targetLabel: "",
    samplePaths,
  };
}

export const MediaContentContainer = React.memo(function MediaContentContainer({
  view,
  contentSettingsSection,
  canManageConfig,
  canManageSystemSettings,
  canManageCatalogSettings,
  canManageLibrarySettings,
  canViewCatalog,
  canManageTitle,
  canManageTitlesInLibrary,
  canRequestMedia,
  authorizationSignature,
  onOpenOverview,
  routeOverviewTitleId,
  routeOverviewPending,
  routeOverviewEpisodeId,
  onCloseOverview,
}: MediaContentContainerProps) {
  const searchState = useSearchContext();
  const {
    addMetadataSearchResultToCatalog,
    catalogConfigLoading,
    catalogQualityProfileOptions,
    ensureCatalogConfigReady,
    librariesByFacet,
    queueFacet,
    requestableLibrariesByFacet,
    requestMetadataSearchResult,
    resolveDefaultQualityProfileIdForFacet,
    rootFoldersByFacet,
    runTvdbSearch,
    setQueueFacet,
    tvdbCandidates,
  } = searchState;
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { registerInteractiveJobRun } = useJobRunToasts();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();
  const { queueCatalogTitleRefresh } = useReactiveRefresh();
  const [titleDeleteTypedConfirmation, setTitleDeleteTypedConfirmation] =
    React.useState("");
  const [pendingDeletedTitleIds, setPendingDeletedTitleIds] = React.useState<
    Set<string>
  >(() => new Set());
  const pendingDeletedTitleIdsRef = React.useRef(pendingDeletedTitleIds);
  React.useLayoutEffect(() => {
    pendingDeletedTitleIdsRef.current = pendingDeletedTitleIds;
  }, [pendingDeletedTitleIds]);
  const deletionJobIdsRef = React.useRef(new Set<string>());
  const deletionFallbackTimersRef = React.useRef<
    ReturnType<typeof setTimeout>[]
  >([]);
  const [libraryScanUiStateByLibraryId, setLibraryScanUiStateByLibraryId] =
    React.useState<
      Record<
        string,
        {
          loading: boolean;
          sessionId: string | null;
          notice: string | null;
          summary: ReturnType<
            typeof useTitleManagementState
          >["libraryScanSummary"];
        }
      >
    >({});
  const libraryScanReconcileTimersRef = React.useRef(
    new Map<
      string,
      {
        refreshTimers: number[];
        releaseTimer: number;
      }
    >(),
  );
  const activeFacet = viewToFacet[view as keyof typeof viewToFacet] ?? "MOVIE";
  const canManageCatalogDiscovery =
    (librariesByFacet[activeFacet] ?? []).length > 0;
  const canRequestCatalogDiscovery =
    (requestableLibrariesByFacet[activeFacet] ?? []).length > 0;
  const manageableDiscoveryFacets = React.useMemo(
    () =>
      DISCOVERY_FACETS.filter(
        (facet) => (librariesByFacet[facet] ?? []).length > 0,
      ),
    [librariesByFacet],
  );
  const requestableDiscoveryFacets = React.useMemo(
    () =>
      DISCOVERY_FACETS.filter(
        (facet) => (requestableLibrariesByFacet[facet] ?? []).length > 0,
      ),
    [requestableLibrariesByFacet],
  );
  const [selectedLibraryIdsByFacet, setSelectedLibraryIdsByFacet] =
    React.useState<Record<Facet, string[]>>(createSelectedLibraryIdsByFacet);
  const selectedLibraryIds = selectedLibraryIdsByFacet[activeFacet];
  const setSelectedLibraryIds = React.useCallback<
    React.Dispatch<React.SetStateAction<string[]>>
  >(
    (nextSelection) => {
      setSelectedLibraryIdsByFacet((current) => {
        const currentSelection = current[activeFacet];
        const next =
          typeof nextSelection === "function"
            ? nextSelection(currentSelection)
            : nextSelection;
        return { ...current, [activeFacet]: next };
      });
    },
    [activeFacet],
  );
  const [advancedTitleFiltersByFacet, setAdvancedTitleFiltersByFacet] =
    React.useState<Record<Facet, TitleCatalogAdvancedFilters>>(
      createAdvancedTitleFiltersByFacet,
    );
  const [debouncedAdvancedTitleFiltersByFacet, setDebouncedAdvancedTitleFiltersByFacet] =
    React.useState<Record<Facet, TitleCatalogAdvancedFilters>>(
      createAdvancedTitleFiltersByFacet,
    );
  const [titleCatalogFilterOptionsByFacet, setTitleCatalogFilterOptionsByFacet] =
    React.useState<Record<Facet, TitleCatalogFilterOptionsRecord>>(() => ({
      MOVIE: EMPTY_TITLE_CATALOG_FILTER_OPTIONS,
      SERIES: EMPTY_TITLE_CATALOG_FILTER_OPTIONS,
      ANIME: EMPTY_TITLE_CATALOG_FILTER_OPTIONS,
    }));
  const [titleCatalogFilterOptionsErrorByFacet, setTitleCatalogFilterOptionsErrorByFacet] =
    React.useState<Record<Facet, boolean>>(() => ({
      MOVIE: false,
      SERIES: false,
      ANIME: false,
  }));
  const advancedTitleFilters = advancedTitleFiltersByFacet[activeFacet];
  const debouncedAdvancedTitleFilters =
    debouncedAdvancedTitleFiltersByFacet[activeFacet];
  const effectiveAdvancedTitleFilters = React.useMemo(
    () => ({
      ...advancedTitleFilters,
      minimumYear: debouncedAdvancedTitleFilters.minimumYear,
      maximumYear: debouncedAdvancedTitleFilters.maximumYear,
      minimumRating: debouncedAdvancedTitleFilters.minimumRating,
    }),
    [advancedTitleFilters, debouncedAdvancedTitleFilters],
  );
  const titleCatalogFilterOptions =
    titleCatalogFilterOptionsByFacet[activeFacet];
  const titleCatalogFilterOptionsError =
    titleCatalogFilterOptionsErrorByFacet[activeFacet];
  const titleCatalogFilterOptionsRequestIdRef = React.useRef(0);
  const reloadCatalogForAdvancedFiltersRef = React.useRef<
    ((filters: TitleCatalogAdvancedFilters) => void) | null
  >(null);
  const setAdvancedTitleFilters = React.useCallback<
    React.Dispatch<React.SetStateAction<TitleCatalogAdvancedFilters>>
  >(
    (nextFilters) => {
      setAdvancedTitleFiltersByFacet((current) => {
        const currentFilters = current[activeFacet];
        const next =
          typeof nextFilters === "function"
            ? nextFilters(currentFilters)
            : nextFilters;
        return { ...current, [activeFacet]: next };
      });
    },
    [activeFacet],
  );
  const updateAdvancedTitleFilters = React.useCallback(
    (updates: Partial<TitleCatalogAdvancedFilters>) => {
      const nextFilters = { ...advancedTitleFilters, ...updates };
      setAdvancedTitleFilters(nextFilters);
      if (
        updates.rootFolderIds !== undefined ||
        updates.genreTagKeys !== undefined ||
        updates.themeTagKeys !== undefined
      ) {
        markCatalogTiming("filter-intent");
        reloadCatalogForAdvancedFiltersRef.current?.(nextFilters);
      }
    },
    [advancedTitleFilters, setAdvancedTitleFilters],
  );
  const clearAdvancedTitleFilters = React.useCallback(() => {
    setSelectedLibraryIds([]);
    setAdvancedTitleFilters({ ...EMPTY_TITLE_ADVANCED_FILTERS });
  }, [setAdvancedTitleFilters, setSelectedLibraryIds]);

  React.useEffect(() => {
    const timeout = window.setTimeout(() => {
      setDebouncedAdvancedTitleFiltersByFacet((current) => ({
        ...current,
        [activeFacet]: {
          ...current[activeFacet],
          minimumYear: advancedTitleFilters.minimumYear,
          maximumYear: advancedTitleFilters.maximumYear,
          minimumRating: advancedTitleFilters.minimumRating,
        },
      }));
    }, TITLE_CATALOG_FILTER_DEBOUNCE_MS);
    return () => window.clearTimeout(timeout);
  }, [
    activeFacet,
    advancedTitleFilters.maximumYear,
    advancedTitleFilters.minimumRating,
    advancedTitleFilters.minimumYear,
  ]);
  const catalogDiscoveryRequestIdRef = React.useRef(0);
  const [catalogDiscoveryGroups, setCatalogDiscoveryGroups] =
    React.useState<CatalogDiscoveryGroup[]>([]);
  const [addDiscoveryDialogTarget, setAddDiscoveryDialogTarget] =
    React.useState<{ result: MetadataTvdbSearchItem; facet: Facet } | null>(
      null,
    );
  const [requestDiscoveryDialogTarget, setRequestDiscoveryDialogTarget] =
    React.useState<{ result: MetadataTvdbSearchItem; facet: Facet } | null>(
      null,
    );
  const authorizationSignatureRef = React.useRef(authorizationSignature);
  React.useLayoutEffect(() => {
    authorizationSignatureRef.current = authorizationSignature;
    catalogDiscoveryRequestIdRef.current += 1;
    setCatalogDiscoveryGroups([]);
    setAddDiscoveryDialogTarget(null);
    setRequestDiscoveryDialogTarget(null);
  }, [authorizationSignature]);
  const {
    sessions: libraryScanSessions,
    getActiveSession,
    getSessionById,
    refreshSessions: refreshLibraryScanSessions,
  } = useLibraryScanProgress();
  const isMobile = useIsMobile();
  const activeQualityScopeId =
    CATEGORY_SCOPE_MAP[view as keyof typeof CATEGORY_SCOPE_MAP] ?? "movie";
  const isMediaView =
    view === "movies" || view === "series" || view === "anime";
  const shouldLoadCatalogTitles =
    isMediaView && contentSettingsSection === "overview";
  const shouldLoadMediaSettingsForSection =
    isMediaView && isMediaSettingsSection(contentSettingsSection);
  const refreshCatalogDiscovery = React.useCallback(async () => {
    const requestId = catalogDiscoveryRequestIdRef.current + 1;
    catalogDiscoveryRequestIdRef.current = requestId;
    if (
      !shouldLoadCatalogTitles ||
      !catalogDependentRequestsAllowedRef.current ||
      (!canManageCatalogDiscovery && !canRequestCatalogDiscovery)
    ) {
      setCatalogDiscoveryGroups([]);
      return;
    }
    setCatalogDiscoveryGroups([]);
    const libraryIds = selectedLibraryIds.filter(
      (libraryId) => libraryId !== ALL_LIBRARIES_VALUE,
    );
    const input: CatalogDiscoveryInput = {
      facet: activeFacet,
      libraryIds,
      includeUnresolved: true,
      limitPerGroup: CATALOG_DISCOVERY_LIMIT_PER_GROUP,
      maxGroups: CATALOG_DISCOVERY_MAX_GROUPS,
    };
    try {
      const { data, error } = await client
        .query<{ catalogDiscovery?: CatalogDiscoveryPayload }>(
          catalogDiscoveryQuery,
          { input },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      if (catalogDiscoveryRequestIdRef.current === requestId) {
        setCatalogDiscoveryGroups(
          data?.catalogDiscovery?.groups ?? [],
        );
      }
    } catch (error) {
      console.error("[catalog-discovery] refresh failed:", error);
      if (catalogDiscoveryRequestIdRef.current === requestId) {
        setCatalogDiscoveryGroups([]);
      }
    }
  }, [
    activeFacet,
    canManageCatalogDiscovery,
    canRequestCatalogDiscovery,
    client,
    selectedLibraryIds,
    shouldLoadCatalogTitles,
  ]);

  const refreshTitleCatalogFilterOptions = React.useCallback(async () => {
    const requestId = titleCatalogFilterOptionsRequestIdRef.current + 1;
    titleCatalogFilterOptionsRequestIdRef.current = requestId;
    if (
      !shouldLoadCatalogTitles ||
      !catalogDependentRequestsAllowedRef.current
    ) {
      return;
    }
    const libraryIds = selectedLibraryIds.filter(
      (libraryId) => libraryId !== ALL_LIBRARIES_VALUE,
    );
    try {
      const { data, error } = await client
        .query<{
          titleCatalogFilterOptions?: TitleCatalogFilterOptionsRecord;
        }>(
          titleCatalogFilterOptionsQuery,
          {
            facet: activeFacet,
            libraryIds: libraryIds.length > 0 ? libraryIds : null,
            rootFolderIds:
              advancedTitleFilters.rootFolderIds.length > 0
                ? advancedTitleFilters.rootFolderIds
                : null,
          },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      if (titleCatalogFilterOptionsRequestIdRef.current === requestId) {
        const nextOptions =
          data?.titleCatalogFilterOptions ??
          EMPTY_TITLE_CATALOG_FILTER_OPTIONS;
        setTitleCatalogFilterOptionsByFacet((current) => ({
          ...current,
          [activeFacet]: nextOptions,
        }));
        setTitleCatalogFilterOptionsErrorByFacet((current) => ({
          ...current,
          [activeFacet]: false,
        }));
        const optionMinimumYear = nextOptions.minimumYear;
        const optionMaximumYear = nextOptions.maximumYear;
        if (optionMinimumYear !== null && optionMaximumYear !== null) {
          setAdvancedTitleFilters((current) => {
            let minimumYear = current.minimumYear;
            let maximumYear = current.maximumYear;
            if (
              minimumYear !== null &&
              (minimumYear <= optionMinimumYear ||
                minimumYear > optionMaximumYear)
            ) {
              minimumYear = null;
            }
            if (
              maximumYear !== null &&
              (maximumYear >= optionMaximumYear ||
                maximumYear < optionMinimumYear)
            ) {
              maximumYear = null;
            }
            if (
              minimumYear !== null &&
              maximumYear !== null &&
              minimumYear > maximumYear
            ) {
              minimumYear = null;
              maximumYear = null;
            }
            return minimumYear === current.minimumYear &&
              maximumYear === current.maximumYear
              ? current
              : { ...current, minimumYear, maximumYear };
          });
        }
      }
    } catch (error) {
      console.error("[title-catalog-filters] options refresh failed:", error);
      if (titleCatalogFilterOptionsRequestIdRef.current === requestId) {
        setTitleCatalogFilterOptionsErrorByFacet((current) => ({
          ...current,
          [activeFacet]: true,
        }));
      }
    }
  }, [
    activeFacet,
    advancedTitleFilters.rootFolderIds,
    client,
    selectedLibraryIds,
    setAdvancedTitleFilters,
    shouldLoadCatalogTitles,
  ]);

  const [desktopViewModes, setDesktopViewModes] = React.useState<
    Partial<Record<ViewId, ContentViewMode>>
  >(() => ({ [view]: readStoredContentViewMode(view) }));
  const desktopViewMode =
    desktopViewModes[view] ?? readStoredContentViewMode(view);
  const effectiveViewMode: ContentViewMode = desktopViewMode;
  const [selectedTitleIds, setSelectedTitleIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [selectedOverviewTitleId, setSelectedOverviewTitleId] = React.useState<
    string | null
  >(null);
  const selectedOverviewTitleIdRef = React.useRef<string | null>(null);
  const [selectedOverviewDetailTitle, setSelectedOverviewDetailTitle] =
    React.useState<TitleRecord | null>(null);
  const [selectedOverviewDetailLoading, setSelectedOverviewDetailLoading] =
    React.useState(false);
  const [selectedOverviewBlocklistState, setSelectedOverviewBlocklistState] =
    React.useState<{
      titleId: string | null;
      entries: TitleReleaseBlocklistEntry[];
  }>({ titleId: null, entries: [] });
  const selectedOverviewBlocklistEntries =
    selectedOverviewBlocklistState.titleId === selectedOverviewTitleId
      ? selectedOverviewBlocklistState.entries
      : [];
  const [
    selectedOverviewExternalSubtitleState,
    setSelectedOverviewExternalSubtitleState,
  ] = React.useState<{
    titleId: string | null;
    entries: ExternalSubtitleRecord[];
  }>({ titleId: null, entries: [] });
  const selectedOverviewExternalSubtitles =
    selectedOverviewExternalSubtitleState.titleId === selectedOverviewTitleId
      ? selectedOverviewExternalSubtitleState.entries
      : [];
  const [titleQuickFilters, setTitleQuickFilters] =
    React.useState<TitleQuickFilters>(EMPTY_TITLE_QUICK_FILTERS);
  const [titleCatalogSort, setTitleCatalogSort] =
    React.useState<TitleCatalogSortState>(defaultTitleCatalogSortState);
  const [visibleTitleTableColumns, setVisibleTitleTableColumns] =
    React.useState<TitleTableVisibleColumns>(() => ({
      ...DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
    }));
  const setTitleTableColumnVisible = React.useCallback(
    (key: TitleTableColumnKey, checked: boolean) => {
      setVisibleTitleTableColumns((current) => ({
        ...current,
        [key]: checked,
      }));
    },
    [],
  );
  const effectiveTitleCatalogSort = React.useMemo<TitleCatalogSortState>(() => {
    if (effectiveViewMode === "poster") {
      return defaultTitleCatalogSortState;
    }
    if (titleCatalogSort.key === "name") {
      return titleCatalogSort;
    }
    const columnKey = titleCatalogSort.key as TitleTableColumnKey;
    if (
      !isTitleTableColumnSupportedForView(columnKey, view) ||
      visibleTitleTableColumns[columnKey] !== true
    ) {
      return defaultTitleCatalogSortState;
    }
    return titleCatalogSort;
  }, [effectiveViewMode, titleCatalogSort, view, visibleTitleTableColumns]);
  const titleCatalogProjection = React.useMemo(
    () =>
      titleCatalogProjectionForTable({
        facet: activeFacet,
        visibleColumns:
          effectiveViewMode === "poster" ? {} : visibleTitleTableColumns,
        sort: effectiveTitleCatalogSort,
      }),
    [activeFacet, effectiveTitleCatalogSort, effectiveViewMode, visibleTitleTableColumns],
  );
  const [bulkActionBusy, setBulkActionBusy] = React.useState(false);
  const [bulkEditDialogOpen, setBulkEditDialogOpen] = React.useState(false);
  const shouldLoadMediaSettings =
    shouldLoadMediaSettingsForSection || bulkEditDialogOpen;
  const [debouncedTitleFilter, setDebouncedTitleFilter] = React.useState("");
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [librariesFacet, setLibrariesFacet] = React.useState<Facet | null>(null);
  const effectiveLibraryScanTargetId = React.useMemo(() => {
    const explicitSelectedLibraryIds = selectedLibraryIds.filter(
      (libraryId) => libraryId !== ALL_LIBRARIES_VALUE,
    );
    const selectedLibraryId = singleSelectedLibraryId(
      explicitSelectedLibraryIds,
    );
    if (selectedLibraryId) {
      return selectedLibraryId;
    }
    return explicitSelectedLibraryIds.length === 0 && libraries.length === 1
      ? (libraries[0]?.id ?? null)
      : null;
  }, [libraries, selectedLibraryIds]);
  const activeTargetLibraryScanSession = effectiveLibraryScanTargetId
    ? getActiveSession(activeFacet, effectiveLibraryScanTargetId)
    : null;
  const activeLibraryScanUiState = effectiveLibraryScanTargetId
    ? libraryScanUiStateByLibraryId[effectiveLibraryScanTargetId]
    : undefined;
  const relevantActiveLibraryScanSessions = React.useMemo(
    () =>
      activeLibraryScanSessionsForSelection(
        libraryScanSessions,
        activeFacet,
        selectedLibraryIds.filter(
          (libraryId) => libraryId !== ALL_LIBRARIES_VALUE,
        ),
      ),
    [activeFacet, libraryScanSessions, selectedLibraryIds],
  );
  const [librariesLoading, setLibrariesLoading] = React.useState(false);
  React.useEffect(() => {
    if (librariesFacet !== activeFacet) {
      return;
    }
    const explicitLibraryIds = selectedLibraryIds.filter(
      (libraryId) => libraryId !== ALL_LIBRARIES_VALUE,
    );
    const eligibleLibraryIds = new Set(
      explicitLibraryIds.length > 0
        ? explicitLibraryIds
        : libraries.map((library) => library.id),
    );
    const validRootFolderIds = new Set(
      libraries
        .filter((library) => eligibleLibraryIds.has(library.id))
        .flatMap((library) => library.roots.map((root) => root.id)),
    );
    setAdvancedTitleFilters((current) => {
      const rootFolderIds = current.rootFolderIds.filter((rootFolderId) =>
        validRootFolderIds.has(rootFolderId),
      );
      return rootFolderIds.length === current.rootFolderIds.length
        ? current
        : { ...current, rootFolderIds };
    });
  }, [activeFacet, libraries, librariesFacet, selectedLibraryIds, setAdvancedTitleFilters]);
  const [libraryDownloadClients, setLibraryDownloadClients] = React.useState<
    DownloadClientRecord[]
  >([]);
  const [libraryDownloadClientsLoading, setLibraryDownloadClientsLoading] =
    React.useState(false);
  const [catalogBootstrapState, setCatalogBootstrapState] =
    React.useState<CatalogBootstrapState>({
    key: "",
    phase: "resolving",
    error: null,
  });
  const catalogBootstrapInFlightKeyRef = React.useRef<string | null>(null);
  const catalogDependentRequestsAllowedRef = React.useRef(false);
  const [rootValidationLibraries, setRootValidationLibraries] = React.useState<
    LibraryRecord[]
  >([]);
  const [rootValidationLibrariesLoading, setRootValidationLibrariesLoading] =
    React.useState(false);
  const [, setInvalidRootLibraryIds] = React.useState<string[]>([]);
  const [invalidRootPathsByLibraryId, setInvalidRootPathsByLibraryId] =
    React.useState<Record<string, string[]>>({});
  const [rootValidationUnavailable, setRootValidationUnavailable] =
    React.useState(false);
  const [, setValidatedRootFolderSnapshotKey] = React.useState<string | null>(
    null,
  );
  const [librarySettingsSaving, setLibrarySettingsSaving] =
    React.useState(false);
  const rootFolderValidationSnapshot = React.useMemo(() => {
    if (
      !isMediaView ||
      librariesLoading ||
      contentSettingsSection !== "library"
    ) {
      return null;
    }

    const explicitSelectedLibraryIds = selectedLibraryIds.filter(
      (libraryId) => libraryId !== ALL_LIBRARIES_VALUE,
    );
    const selectedLibraryIdSet =
      explicitSelectedLibraryIds.length > 0
        ? new Set(explicitSelectedLibraryIds)
        : null;
    const relevantLibraries = libraries.filter((library) =>
      selectedLibraryIdSet ? selectedLibraryIdSet.has(library.id) : true,
    );
    const librariesWithConfiguredRoots = relevantLibraries.filter((library) =>
      library.roots.some((root) => root.path.trim().length > 0),
    );
    const key = librariesWithConfiguredRoots
      .map((library) => {
        const rootsKey = library.roots
          .map((root) => root.path.trim())
          .filter((path) => path.length > 0)
          .sort()
          .join("\u001f");
        return `${library.id}:${rootsKey}`;
      })
      .sort()
      .join("\u001e");

    return { key, librariesWithConfiguredRoots };
  }, [
    contentSettingsSection,
    isMediaView,
    libraries,
    librariesLoading,
    selectedLibraryIds,
  ]);
  const activeCatalogQueryRef = React.useRef("");
  const interactiveSearchAbortRef = React.useRef<AbortController | null>(null);
  const activeCatalogListFiltersRef = React.useRef<ActiveCatalogListFilters>({
    facet: activeFacet,
    query: "",
    libraryIds: [],
  });
  const catalogTitleRequestSeqRef = React.useRef(0);
  const catalogBootstrapRequestSeqRef = React.useRef(0);
  const catalogPageLoadInFlightRef = React.useRef(false);
  const catalogQueryKeyRef = React.useRef("");
  const libraryScanTitleRefreshRef = React.useRef<{
    key: string;
    refreshedAt: number;
    sessionIds: string[];
  }>({ key: "", refreshedAt: 0, sessionIds: [] });
  const latestCriticalMutationEpochRef = React.useRef(0);
  const selectedPanelHydrationKeyRef = React.useRef<string | null>(null);
  const skipNextCatalogOverviewReloadRef = React.useRef(false);
  const [catalogPaginationState, setCatalogPaginationState] =
    React.useState<TitleCatalogState>(emptyTitleCatalogState);

  React.useEffect(() => {
    catalogQueryKeyRef.current = catalogPaginationState.queryKey;
  }, [catalogPaginationState.queryKey]);

  React.useEffect(() => {
    return () => {
      interactiveSearchAbortRef.current?.abort();
      interactiveSearchAbortRef.current = null;
    };
  }, []);

  const {
    titleNameForQueue,
    setTitleNameForQueue,
    monitoredForQueue,
    setMonitoredForQueue,
    seasonFoldersForQueue,
    setSeasonFoldersForQueue,
    minAvailabilityForQueue,
    setMinAvailabilityForQueue,
  } = useQueueFormState();

  const {
    titleFilter,
    setTitleFilter,
    monitoredTitles,
    setMonitoredTitles,
    titleLoading,
    setTitleLoading,
    titleStatus,
    setTitleStatus,
    titleToDelete,
    setTitleToDelete,
    deleteFilesOnDisk,
    setDeleteFilesOnDisk,
    deleteTitleLoadingById,
    setDeleteTitleLoadingById,
  } = useTitleManagementState();
  const [titleContextTitles, setTitleContextTitles] = React.useState<
    TitleRecord[]
  >([]);
  const [
    selectedOverviewMediaFileToDelete,
    setSelectedOverviewMediaFileToDelete,
  ] = React.useState<SelectedOverviewMediaFileDeleteTarget | null>(null);
  const [
    selectedOverviewMediaFileDeleteLoading,
    setSelectedOverviewMediaFileDeleteLoading,
  ] = React.useState(false);
  const [pendingMediaFileDeletionIds, setPendingMediaFileDeletionIds] =
    React.useState<Set<string>>(() => new Set());
  const mediaFileDeletionUnregistersRef = React.useRef(new Set<() => void>());
  const [
    selectedOverviewMediaFileDeleteTypedConfirmation,
    setSelectedOverviewMediaFileDeleteTypedConfirmation,
  ] = React.useState("");
  const [
    selectedOverviewPrimaryMovieFileUpdatingId,
    setSelectedOverviewPrimaryMovieFileUpdatingId,
  ] = React.useState<string | null>(null);
  const mergeTitleContextTitles = React.useCallback(
    (incomingTitles: TitleRecord[]) => {
      const activeFacetTitles = incomingTitles.filter(
        (title) =>
          title.facet === activeFacet &&
          !pendingDeletedTitleIdsRef.current.has(title.id),
      );
      if (activeFacetTitles.length === 0) {
        return;
      }
      setTitleContextTitles((current) =>
        appendCatalogTitlesPreservingImages(
          current.filter((title) => title.facet === activeFacet),
          activeFacetTitles,
        ),
      );
    },
    [activeFacet],
  );
  const libraryScanInProgress = isLibraryScanTargetBusy(
    libraryScanUiStateByLibraryId,
    effectiveLibraryScanTargetId,
    activeTargetLibraryScanSession,
    getSessionById,
  );
  const catalogBootstrapKey = `${activeFacet}:${selectedLibraryIds.join(",")}`;
  const catalogSurfaceState =
    shouldLoadCatalogTitles &&
    catalogBootstrapState.key === catalogBootstrapKey
      ? catalogBootstrapState
      : {
          key: catalogBootstrapKey,
          phase: "resolving" as const,
          error: null,
        };
  const catalogInitialLoadComplete =
    shouldLoadCatalogTitles &&
    catalogSurfaceState.phase !== "resolving";
  const catalogBootstrapLoading =
    shouldLoadCatalogTitles && catalogSurfaceState.phase === "resolving";
  const catalogSurfaceAllowsDependentRequests =
    shouldLoadCatalogTitles &&
    (catalogSurfaceState.phase === "content" ||
      catalogSurfaceState.phase === "empty");
  catalogDependentRequestsAllowedRef.current =
    catalogSurfaceAllowsDependentRequests;
  React.useEffect(() => {
    if (!catalogSurfaceAllowsDependentRequests) {
      setCatalogDiscoveryGroups([]);
      return;
    }

    void refreshCatalogDiscovery();
    void refreshTitleCatalogFilterOptions();
  }, [
    catalogSurfaceAllowsDependentRequests,
    refreshCatalogDiscovery,
    refreshTitleCatalogFilterOptions,
  ]);
  const retryCatalogBootstrap = React.useCallback(() => {
    catalogBootstrapRequestSeqRef.current += 1;
    catalogBootstrapInFlightKeyRef.current = null;
    setCatalogBootstrapState({
      key: "",
      phase: "resolving",
      error: null,
    });
  }, []);
  const titleDeletePreviewVariables = React.useMemo(
    () =>
      titleToDelete && deleteFilesOnDisk ? { titleId: titleToDelete.id } : null,
    [deleteFilesOnDisk, titleToDelete],
  );
  const {
    preview: titleDeletePreview,
    loading: titleDeletePreviewLoading,
    error: titleDeletePreviewError,
  } = useDeletePreview(
    deleteTitlePreviewQuery,
    "deleteTitlePreview",
    titleDeletePreviewVariables,
    titleToDelete !== null && deleteFilesOnDisk,
  );
  const selectedOverviewMediaFileDeletePreviewVariables = React.useMemo(
    () =>
      selectedOverviewMediaFileToDelete
        ? { fileId: selectedOverviewMediaFileToDelete.file.id }
        : null,
    [selectedOverviewMediaFileToDelete],
  );
  const {
    preview: selectedOverviewMediaFileDeletePreview,
    loading: selectedOverviewMediaFileDeletePreviewLoading,
    error: selectedOverviewMediaFileDeletePreviewError,
  } = useDeletePreview(
    deleteMediaFilePreviewQuery,
    "deleteMediaFilePreview",
    selectedOverviewMediaFileDeletePreviewVariables,
    selectedOverviewMediaFileToDelete !== null,
  );
  const effectiveTitleQuickFilters = React.useMemo<TitleQuickFilters>(
    () => ({
      ...titleQuickFilters,
      continuing:
        activeFacet === "MOVIE" ? false : titleQuickFilters.continuing,
      ended: activeFacet === "MOVIE" ? false : titleQuickFilters.ended,
    }),
    [activeFacet, titleQuickFilters],
  );
  const libraryNameById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library.name])),
    [libraries],
  );
  const librarySlugById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library.slug])),
    [libraries],
  );
  const catalogSourceTitlesWithLibraries = React.useMemo(
    () =>
      monitoredTitles.map((title) => {
        const libraryName =
          title.libraryName ??
          libraryNameById.get(title.libraryId) ??
          title.libraryId;
        const librarySlug =
          title.librarySlug ?? librarySlugById.get(title.libraryId) ?? null;
        return title.libraryName === libraryName && title.librarySlug === librarySlug
          ? title
          : { ...title, libraryName, librarySlug };
      }),
    [libraryNameById, librarySlugById, monitoredTitles],
  );
  const titleContextSourceTitles = React.useMemo(
    () =>
      titleContextTitles
        .filter(
          (title) =>
            title.facet === activeFacet && !pendingDeletedTitleIds.has(title.id),
        )
        .map((title) => {
          const libraryName =
            title.libraryName ??
            libraryNameById.get(title.libraryId) ??
            title.libraryId;
          const librarySlug =
            title.librarySlug ?? librarySlugById.get(title.libraryId) ?? null;
          return title.libraryName === libraryName && title.librarySlug === librarySlug
            ? title
            : { ...title, libraryName, librarySlug };
        }),
    [
      activeFacet,
      libraryNameById,
      librarySlugById,
      pendingDeletedTitleIds,
      titleContextTitles,
    ],
  );
  const titleQuickFilterCounts = catalogPaginationState.filterCounts;
  React.useEffect(() => {
    if (pendingDeletedTitleIds.size === 0) {
      return;
    }
    setTitleContextTitles((current) =>
      current.filter((title) => !pendingDeletedTitleIds.has(title.id)),
    );
  }, [pendingDeletedTitleIds]);
  const visibleTitles = React.useMemo(
    () =>
      filterTitlesByQuickFilters(
        catalogSourceTitlesWithLibraries.filter(
          (title) =>
            title.facet === activeFacet &&
            !pendingDeletedTitleIds.has(title.id),
        ),
        effectiveTitleQuickFilters,
      ),
    [
      activeFacet,
      catalogSourceTitlesWithLibraries,
      effectiveTitleQuickFilters,
      pendingDeletedTitleIds,
    ],
  );
  const selectedTitles = React.useMemo(
    () => visibleTitles.filter((title) => selectedTitleIds.has(title.id)),
    [selectedTitleIds, visibleTitles],
  );
  const editDialogTitles = selectedTitles;
  const selectedTitleLibraryIds = React.useMemo(
    () => Array.from(new Set(selectedTitles.map((title) => title.libraryId))),
    [selectedTitles],
  );
  // Renaming rewrites files on disk, so it needs manage rights on every library
  // the selection touches, not just on one of them.
  const canRenameSelectedTitles = React.useMemo(
    () =>
      selectedTitleLibraryIds.length > 0 &&
      selectedTitleLibraryIds.every((libraryId) =>
        canManageTitlesInLibrary(libraryId),
      ),
    [canManageTitlesInLibrary, selectedTitleLibraryIds],
  );
  const editDialogTitleLibraryIds = React.useMemo(
    () => Array.from(new Set(editDialogTitles.map((title) => title.libraryId))),
    [editDialogTitles],
  );
  const editDialogRootFolders = React.useMemo(() => {
    if (editDialogTitleLibraryIds.length !== 1) {
      return [];
    }
    return (
      libraries.find((library) => library.id === editDialogTitleLibraryIds[0])
        ?.roots ?? []
    );
  }, [editDialogTitleLibraryIds, libraries]);

  useOverviewWindowScrollRestoration({
    enabled: shouldLoadCatalogTitles && effectiveViewMode === "poster",
    ready: !titleLoading && visibleTitles.length > 0,
    storageKeySuffix: "window",
  });

  React.useLayoutEffect(() => {
    if (
      !shouldLoadCatalogTitles ||
      effectiveViewMode === "poster" ||
      typeof window === "undefined"
    ) {
      return;
    }

    window.scrollTo({ top: 0, left: 0, behavior: "auto" });
  }, [effectiveViewMode, shouldLoadCatalogTitles]);

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles) {
      setDebouncedTitleFilter("");
      return;
    }

    const timer = window.setTimeout(() => {
      setDebouncedTitleFilter(titleFilter.trim());
    }, 250);

    return () => {
      window.clearTimeout(timer);
    };
  }, [shouldLoadCatalogTitles, titleFilter]);

  React.useEffect(() => {
    setTitleQuickFilters(EMPTY_TITLE_QUICK_FILTERS);
    setSelectedTitleIds(new Set());
    setSelectedOverviewTitleId(null);
    setTitleContextTitles([]);
  }, [activeFacet]);

  // The route (slug deep link / in-app navigation) is the source of truth for
  // which title is selected in the list. Mirror it into local selection state
  // so the inline overview pane reflects the URL on load and live navigation.
  React.useEffect(() => {
    if (routeOverviewPending && !routeOverviewTitleId) {
      return;
    }
    selectedOverviewTitleIdRef.current = routeOverviewTitleId;
    setSelectedOverviewTitleId(routeOverviewTitleId);
  }, [routeOverviewPending, routeOverviewTitleId]);

  React.useEffect(() => {
    selectedOverviewTitleIdRef.current = selectedOverviewTitleId;
    setSelectedOverviewDetailTitle((current) =>
      current?.id === selectedOverviewTitleId ? current : null,
    );
  }, [selectedOverviewTitleId]);

  React.useEffect(() => {
    if (!selectedOverviewTitleId) {
      return;
    }
    const selectedCatalogTitle = visibleTitles.find(
      (title) => title.id === selectedOverviewTitleId,
    );
    if (!selectedCatalogTitle) {
      return;
    }
    setSelectedOverviewDetailTitle((current) =>
      current?.id === selectedOverviewTitleId ? current : selectedCatalogTitle,
    );
  }, [selectedOverviewTitleId, visibleTitles]);

  React.useEffect(() => {
    const visibleTitleIds = new Set(visibleTitles.map((title) => title.id));
    setSelectedTitleIds((current) => {
      let changed = false;
      const next = new Set<string>();
      current.forEach((id) => {
        if (visibleTitleIds.has(id)) {
          next.add(id);
        } else {
          changed = true;
        }
      });
      return changed ? next : current;
    });
  }, [visibleTitles]);

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles || contentSettingsSection !== "overview") {
      setSelectedOverviewTitleId((current) =>
        current === null ? current : null,
      );
      return;
    }

    setSelectedOverviewTitleId((current) => {
      if (current && current === routeOverviewTitleId) {
        return current;
      }
      return current &&
        titleContextSourceTitles.some((title) => title.id === current)
        ? current
        : null;
    });
  }, [
    contentSettingsSection,
    routeOverviewTitleId,
    shouldLoadCatalogTitles,
    titleContextSourceTitles,
  ]);

  React.useEffect(() => {
    activeCatalogQueryRef.current = debouncedTitleFilter;
  }, [debouncedTitleFilter]);

  React.useEffect(() => {
    setDesktopViewModes((current) => {
      if (current[view]) {
        return current;
      }
      return {
        ...current,
        [view]: readStoredContentViewMode(view),
      };
    });
  }, [view]);

  React.useEffect(() => {
    writeStoredContentViewMode(desktopViewMode, view);
  }, [desktopViewMode, view]);

  React.useEffect(() => {
    if (
      effectiveViewMode === "compact" &&
      shouldLoadCatalogTitles &&
      contentSettingsSection === "overview"
    ) {
      return;
    }
    setSelectedTitleIds((current) =>
      current.size === 0 ? current : new Set(),
    );
  }, [
    contentSettingsSection,
    effectiveViewMode,
    shouldLoadCatalogTitles,
    view,
  ]);

  React.useEffect(() => {
    const visibleIdSet = new Set(visibleTitles.map((title) => title.id));
    setSelectedTitleIds((current) => {
      if (current.size === 0) {
        return current;
      }
      const next = new Set(
        [...current].filter((titleId) => visibleIdSet.has(titleId)),
      );
      return sameIdSet(current, next) ? current : next;
    });
  }, [visibleTitles]);

  const {
    moviesPath,
    setMoviesPath,
    seriesPath,
    setSeriesPath,
    saveSetting,
    mediaSettingsLoading,
    mediaSettingsSaving,
    qualityProfiles,
    qualityProfileEntries,
    qualityProfileParseError,
    globalScoringPersona,
    categoryQualityProfileOverrides,
    categoryRequiredAudioLanguages,
    saveCategoryRequiredAudioLanguages,
    categoryPersonaSelections,
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
    localPathStyle,
    saveCategoryQualityProfileOverride,
    saveCategoryScoringPersonaOverride,
    updateCategoryMediaProfileSettings,
    refreshMediaSettings,
  } = useMediaSettings({
    activeQualityScopeId,
    view,
  });

  const contentSettingsLabel =
    view === "movies"
      ? t("settings.moviesSettings")
      : view === "series"
        ? t("settings.seriesSettings")
        : t("settings.animeSettings");
  const activeFacetLabel =
    activeFacet === "MOVIE"
      ? t("nav.movies")
      : activeFacet === "SERIES"
        ? t("nav.series")
        : t("nav.anime");
  const {
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving,
    hydrateDownloadClientRouting,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
  } = useDownloadClientRouting({
    activeQualityScopeId,
  });
  const {
    indexers,
    activeScopeRouting: activeScopeIndexerRouting,
    activeScopeRoutingOrder: activeScopeIndexerRoutingOrder,
    indexerRoutingLoading,
    indexerRoutingSaving,
    hydrateIndexerRouting,
    setIndexerEnabledForScope,
    updateIndexerRoutingForScope,
    moveIndexerInScope,
  } = useIndexerRouting({
    activeQualityScopeId,
  });
  const [routingInitLoading, setRoutingInitLoading] = React.useState(false);

  const [ruleSets, setRuleSets] = React.useState<RuleSetRecord[]>([]);
  const [rulesLoading, setRulesLoading] = React.useState(true);
  const [rulesSaving, setRulesSaving] = React.useState(false);
  const libraryScanNotice = activeLibraryScanUiState?.notice ?? null;
  const libraryScanSummary = activeLibraryScanUiState?.summary ?? null;
  const [titleMonitoringLoadingById, setTitleMonitoringLoadingById] =
    React.useState<Record<string, boolean>>({});

  React.useEffect(() => {
    setLibraryScanUiStateByLibraryId((current) => {
      let next = current;
      for (const [libraryId, state] of Object.entries(current)) {
        if (!state.sessionId) {
          continue;
        }
        const session = getSessionById(state.sessionId);
        if (
          !session ||
          (session.status !== "COMPLETED" &&
            session.status !== "WARNING" &&
            session.status !== "FAILED" &&
            session.status !== "CANCELED")
        ) {
          continue;
        }
        if (next === current) {
          next = { ...current };
        }
        next[libraryId] = {
          ...state,
          loading: false,
          sessionId: null,
          summary: session.summary ?? state.summary,
        };
      }
      return next;
    });
  }, [getSessionById, libraryScanSessions]);

  React.useEffect(() => {
    const unreconciledSessionIds = new Set(
      Object.values(libraryScanUiStateByLibraryId)
        .map((state) => state.sessionId)
        .filter(
          (sessionId): sessionId is string =>
            sessionId !== null && !getSessionById(sessionId),
        ),
    );

    for (const [sessionId, timers] of libraryScanReconcileTimersRef.current) {
      if (unreconciledSessionIds.has(sessionId)) {
        continue;
      }
      timers.refreshTimers.forEach((timer) => window.clearTimeout(timer));
      window.clearTimeout(timers.releaseTimer);
      libraryScanReconcileTimersRef.current.delete(sessionId);
    }

    for (const sessionId of unreconciledSessionIds) {
      if (libraryScanReconcileTimersRef.current.has(sessionId)) {
        continue;
      }
      const refreshTimers = [0, 400, 1_200].map((delayMs) =>
        window.setTimeout(() => {
          void refreshLibraryScanSessions().catch((error) => {
            console.error(
              `[library-scan] failed to reconcile started scan session ${sessionId}:`,
              error,
            );
          });
        }, delayMs),
      );
      const releaseTimer = window.setTimeout(() => {
        setLibraryScanUiStateByLibraryId((current) =>
          Object.fromEntries(
            Object.entries(current).map(([libraryId, state]) => [
              libraryId,
              state.sessionId === sessionId
                ? { ...state, loading: false, sessionId: null }
                : state,
            ]),
          ),
        );
        libraryScanReconcileTimersRef.current.delete(sessionId);
      }, 4_000);
      libraryScanReconcileTimersRef.current.set(sessionId, {
        refreshTimers,
        releaseTimer,
      });
    }
  }, [
    getSessionById,
    libraryScanUiStateByLibraryId,
    refreshLibraryScanSessions,
  ]);

  React.useEffect(
    () => () => {
      for (const timers of libraryScanReconcileTimersRef.current.values()) {
        timers.refreshTimers.forEach((timer) => window.clearTimeout(timer));
        window.clearTimeout(timers.releaseTimer);
      }
      libraryScanReconcileTimersRef.current.clear();
    },
    [],
  );

  const refreshRuleSets = React.useCallback(async () => {
    setRulesLoading(true);
    try {
      const { data, error } = await client.query(ruleSetsQuery, {}).toPromise();
      if (error) throw error;
      setRuleSets(data.ruleSets || []);
    } catch {
      // silent — rules panel is non-critical
    } finally {
      setRulesLoading(false);
    }
  }, [client]);

  const onToggleRuleFacet = React.useCallback(
    async (ruleSetId: string, enabled: boolean) => {
      const rule = ruleSets.find((r) => r.id === ruleSetId);
      if (!rule) return;

      const nextFacets = enabled
        ? [...rule.appliedFacets, activeFacet]
        : rule.appliedFacets.filter((f) => f !== activeFacet);

      setRulesSaving(true);
      try {
        const { error } = await client
          .mutation(updateRuleSetMutation, {
            input: {
              id: ruleSetId,
              name: rule.name,
              description: rule.description,
              regoSource: rule.regoSource,
              priority: rule.priority,
              appliedFacets: nextFacets,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("status.ruleToggled", {
            name: rule.name,
            state: enabled ? t("label.enabled") : t("label.disabled"),
          }),
        );
        await refreshRuleSets();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setRulesSaving(false);
      }
    },
    [activeFacet, client, refreshRuleSets, ruleSets, setGlobalStatus, t],
  );
  const reloadTitles = React.useCallback(
    async (
      queryOverride?: string,
      libraryIdsOverride?: string[],
      options: CatalogReloadOptions = {},
    ): Promise<TitleRecord[] | null> => {
      const isInitial = options.mode === "initial";
      if (isInitial) {
        setTitleLoading(true);
        setTitleStatus(t("title.loading"));
      }
      const query = (queryOverride ?? activeCatalogQueryRef.current).trim();
      const libraryIds = libraryIdsOverride ?? selectedLibraryIds;
      const advancedFilters =
        options.advancedFilters ?? effectiveAdvancedTitleFilters;
      const queryKey = titleCatalogQueryKey({
        facet: activeFacet,
        query,
        libraryIds,
        filters: effectiveTitleQuickFilters,
        advancedFilters,
        sort: effectiveTitleCatalogSort,
        projection: titleCatalogProjection,
      });
      activeCatalogListFiltersRef.current = buildActiveCatalogListFilters(
        activeFacet,
        query,
        libraryIds,
      );
      const requestSeq = ++catalogTitleRequestSeqRef.current;
      catalogPageLoadInFlightRef.current = true;
      catalogQueryKeyRef.current = queryKey;
      if (isInitial) {
        setCatalogPaginationState({ ...emptyTitleCatalogState, queryKey });
      }

      try {
        markCatalogTiming("request-dispatch");
        const { data, error } = await client
          .query(
            buildTitlesQuery(titleCatalogProjection),
            buildTitleCatalogQueryVariables({
              facet: activeFacet,
              libraryIds,
              query,
              filters: effectiveTitleQuickFilters,
              advancedFilters,
              sort: effectiveTitleCatalogSort,
              limit: TITLE_CATALOG_PAGE_SIZE,
              offset: 0,
            }),
            { requestPolicy: "network-only" },
          )
          .toPromise();
        markCatalogTiming("response-received");
        if (error) {
          throw error;
        }
        if (
          requestSeq !== catalogTitleRequestSeqRef.current ||
          catalogQueryKeyRef.current !== queryKey
        ) {
          return null;
        }

        const page = data?.titles ?? {};
        const nextTitles = (page.items ?? []) as TitleRecord[];
        const filterCounts = titleCatalogFilterCountsFromPage(page);
        setMonitoredTitles((current) =>
          mergeCatalogTitlesPreservingImages(current, nextTitles),
        );
        setCatalogPaginationState({
          queryKey,
          hasMore: Boolean(page.hasMore),
          nextOffset: nextTitles.length,
          totalCount:
            typeof page.totalCount === "number"
              ? page.totalCount
              : nextTitles.length,
          managedBytes:
            typeof page.managedBytes === "number" ? page.managedBytes : 0,
          filterCounts,
          loadingMore: false,
        });
        if (isInitial) {
          setTitleStatus(
            t("title.statusTemplate", {
              count:
                typeof page.totalCount === "number"
                  ? page.totalCount
                  : nextTitles.length,
            }),
          );
        }
        markCatalogTiming("page-commit");
        return nextTitles;
      } catch (error) {
        if (requestSeq !== catalogTitleRequestSeqRef.current) {
          return null;
        }
        if (isInitial) {
          setTitleStatus(
            error instanceof Error ? error.message : t("status.failedToLoad"),
          );
        } else {
          console.error("[title-catalog] background refresh failed:", error);
        }
        return null;
      } finally {
        if (requestSeq === catalogTitleRequestSeqRef.current) {
          catalogPageLoadInFlightRef.current = false;
          if (isInitial) {
            setTitleLoading(false);
          }
        }
      }
    },
    [
      activeFacet,
      client,
      effectiveAdvancedTitleFilters,
      effectiveTitleQuickFilters,
      effectiveTitleCatalogSort,
      selectedLibraryIds,
      setMonitoredTitles,
      setTitleLoading,
      setTitleStatus,
      t,
      titleCatalogProjection,
    ],
  );

  React.useEffect(() => {
    reloadCatalogForAdvancedFiltersRef.current = (advancedFilters) => {
      void reloadTitles(undefined, undefined, {
        advancedFilters,
        mode: "background",
      });
    };
    return () => {
      reloadCatalogForAdvancedFiltersRef.current = null;
    };
  }, [reloadTitles]);

  const refreshTitles = React.useCallback(
    async (query?: string) => {
      await reloadTitles(query ?? titleFilter);
    },
    [reloadTitles, titleFilter],
  );

  const handleCatalogDiscoveryAction = React.useCallback(
    async (item: CatalogDiscoveryItem) => {
      const actionAuthorizationSignature = authorizationSignature;
      if (item.ownedInInput) {
        return;
      }
      const facet = discoveryItemFacet(item);
      if (!facet) {
        setGlobalStatus(t("status.apiError"));
        return;
      }
      const canManageFacet = (librariesByFacet[facet] ?? []).length > 0;
      const canRequestFacet =
        (requestableLibrariesByFacet[facet] ?? []).length > 0;
      if (!canManageFacet && !canRequestFacet) {
        setGlobalStatus(t("status.permissionDenied"));
        return;
      }
      try {
        await ensureCatalogConfigReady(facet);
        if (
          authorizationSignatureRef.current !== actionAuthorizationSignature
        ) {
          return;
        }
        const { data, error } = await client
          .query(
            discoveryItemDetailQuery,
            {
              input: {
                targetKey: item.targetKey,
                includeUnresolved: true,
              },
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (
          authorizationSignatureRef.current !== actionAuthorizationSignature
        ) {
          return;
        }
        if (error) {
          throw error;
        }
        const detailItem =
          (data?.discoveryItemDetail as CatalogDiscoveryItem | null | undefined) ??
          item;
        const detailFacet = discoveryItemFacet(detailItem);
        if (!detailFacet || detailFacet !== facet) {
          throw new Error(t("status.permissionDenied"));
        }
        const canManageDetailFacet =
          (librariesByFacet[detailFacet] ?? []).length > 0;
        const canRequestDetailFacet =
          (requestableLibrariesByFacet[detailFacet] ?? []).length > 0;
        if (!canManageDetailFacet && !canRequestDetailFacet) {
          throw new Error(t("status.permissionDenied"));
        }
        const target = {
          result: metadataResultForDiscoveryItem(detailItem),
          facet: detailFacet,
        };
        if (canManageDetailFacet) {
          setAddDiscoveryDialogTarget(target);
        } else {
          setRequestDiscoveryDialogTarget(target);
        }
      } catch (caught) {
        setGlobalStatus(
          caught instanceof Error ? caught.message : t("status.apiError"),
        );
      }
    },
    [
      authorizationSignature,
      client,
      ensureCatalogConfigReady,
      librariesByFacet,
      requestableLibrariesByFacet,
      setGlobalStatus,
      t,
    ],
  );

  const loadMoreCatalogTitles = React.useCallback(async () => {
    if (
      !shouldLoadCatalogTitles ||
      !catalogPaginationState.hasMore ||
      catalogPaginationState.loadingMore ||
      catalogPageLoadInFlightRef.current
    ) {
      return;
    }

    const requestSeq = catalogTitleRequestSeqRef.current;
    const query = activeCatalogQueryRef.current.trim();
    const queryKey = titleCatalogQueryKey({
      facet: activeFacet,
      query,
      libraryIds: selectedLibraryIds,
      filters: effectiveTitleQuickFilters,
      advancedFilters: effectiveAdvancedTitleFilters,
      sort: effectiveTitleCatalogSort,
      projection: titleCatalogProjection,
    });
    if (catalogPaginationState.queryKey !== queryKey) {
      return;
    }
    const offset = catalogPaginationState.nextOffset;
    catalogPageLoadInFlightRef.current = true;
    setCatalogPaginationState((current) => ({ ...current, loadingMore: true }));

    try {
      const { data, error } = await client
        .query(
          buildTitlesQuery(titleCatalogProjection),
          buildTitleCatalogQueryVariables({
            facet: activeFacet,
            libraryIds: selectedLibraryIds,
            query,
            filters: effectiveTitleQuickFilters,
            advancedFilters: effectiveAdvancedTitleFilters,
            sort: effectiveTitleCatalogSort,
            limit: TITLE_CATALOG_PAGE_SIZE,
            offset,
          }),
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      if (
        requestSeq !== catalogTitleRequestSeqRef.current ||
        catalogQueryKeyRef.current !== queryKey
      ) {
        return;
      }

      const page = data?.titles ?? {};
      const nextTitles = (page.items ?? []) as TitleRecord[];
      const filterCounts = titleCatalogFilterCountsFromPage(
        page,
        catalogPaginationState.filterCounts,
      );
      setMonitoredTitles((current) =>
        appendCatalogTitlesPreservingImages(current, nextTitles),
      );
      setCatalogPaginationState({
        queryKey,
        hasMore: Boolean(page.hasMore),
        nextOffset: offset + nextTitles.length,
        totalCount:
          typeof page.totalCount === "number"
            ? page.totalCount
            : catalogPaginationState.totalCount,
        managedBytes:
          typeof page.managedBytes === "number"
            ? page.managedBytes
            : catalogPaginationState.managedBytes,
        filterCounts,
        loadingMore: false,
      });
    } catch (error) {
      if (
        requestSeq === catalogTitleRequestSeqRef.current &&
        catalogQueryKeyRef.current === queryKey
      ) {
        setTitleStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      }
    } finally {
      if (
        requestSeq === catalogTitleRequestSeqRef.current &&
        catalogQueryKeyRef.current === queryKey
      ) {
        catalogPageLoadInFlightRef.current = false;
        setCatalogPaginationState((current) => ({
          ...current,
          loadingMore: false,
        }));
      }
    }
  }, [
    activeFacet,
    catalogPaginationState.hasMore,
    catalogPaginationState.filterCounts,
    catalogPaginationState.loadingMore,
    catalogPaginationState.nextOffset,
    catalogPaginationState.queryKey,
    catalogPaginationState.totalCount,
    catalogPaginationState.managedBytes,
    client,
    effectiveAdvancedTitleFilters,
    effectiveTitleQuickFilters,
    effectiveTitleCatalogSort,
    selectedLibraryIds,
    setMonitoredTitles,
    setTitleStatus,
    shouldLoadCatalogTitles,
    t,
    titleCatalogProjection,
  ]);

  const refreshLoadedCatalogTitlesQuietly = React.useCallback(async ({
    firstPageOnly = false,
  }: { firstPageOnly?: boolean } = {}) => {
    if (
      !shouldLoadCatalogTitles ||
      titleLoading ||
      catalogPageLoadInFlightRef.current ||
      catalogPaginationState.queryKey === ""
    ) {
      return;
    }

    const requestSeq = catalogTitleRequestSeqRef.current;
    const query = activeCatalogQueryRef.current.trim();
    const queryKey = titleCatalogQueryKey({
      facet: activeFacet,
      query,
      libraryIds: selectedLibraryIds,
      filters: effectiveTitleQuickFilters,
      advancedFilters: effectiveAdvancedTitleFilters,
      sort: effectiveTitleCatalogSort,
      projection: titleCatalogProjection,
    });
    if (catalogPaginationState.queryKey !== queryKey) {
      return;
    }

    const limit = firstPageOnly
      ? TITLE_CATALOG_PAGE_SIZE
      : Math.max(
          TITLE_CATALOG_PAGE_SIZE,
          catalogPaginationState.nextOffset || TITLE_CATALOG_PAGE_SIZE,
        );
    const includePageMetadata = !firstPageOnly;

    try {
      const { data, error } = await client
        .query(
          buildTitlesQuery(titleCatalogProjection, { includePageMetadata }),
          buildTitleCatalogQueryVariables({
            facet: activeFacet,
            libraryIds: selectedLibraryIds,
            query,
            filters: effectiveTitleQuickFilters,
            advancedFilters: effectiveAdvancedTitleFilters,
            sort: effectiveTitleCatalogSort,
            limit,
            offset: 0,
          }),
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      if (
        requestSeq !== catalogTitleRequestSeqRef.current ||
        catalogQueryKeyRef.current !== queryKey
      ) {
        return;
      }

      const page = data?.titles ?? {};
      const nextTitles = (page.items ?? []) as TitleRecord[];
      const filterCounts = includePageMetadata
        ? titleCatalogFilterCountsFromPage(
            page,
            catalogPaginationState.filterCounts,
          )
        : catalogPaginationState.filterCounts;
      setMonitoredTitles((current) =>
        mergeCatalogTitlesPreservingImages(current, nextTitles),
      );
      setCatalogPaginationState((current) => {
        if (current.queryKey !== queryKey) {
          return current;
        }
        return {
          ...current,
          hasMore: includePageMetadata ? Boolean(page.hasMore) : current.hasMore,
          nextOffset: includePageMetadata ? nextTitles.length : current.nextOffset,
          totalCount:
            includePageMetadata && typeof page.totalCount === "number"
              ? page.totalCount
              : current.totalCount,
          managedBytes:
            includePageMetadata && typeof page.managedBytes === "number"
              ? page.managedBytes
              : current.managedBytes,
          filterCounts,
        };
      });
    } catch (error) {
      console.error("[title-list-reactive-refresh] catalog refresh failed:", error);
    }
  }, [
    activeFacet,
    catalogPaginationState.filterCounts,
    catalogPaginationState.nextOffset,
    catalogPaginationState.queryKey,
    client,
    effectiveAdvancedTitleFilters,
    effectiveTitleQuickFilters,
    effectiveTitleCatalogSort,
    selectedLibraryIds,
    setMonitoredTitles,
    shouldLoadCatalogTitles,
    titleCatalogProjection,
    titleLoading,
  ]);

  const recordCriticalCatalogMutation = React.useCallback(() => {
    latestCriticalMutationEpochRef.current = reactiveRefreshEpoch();
  }, []);

  const clearDeletionFallbackTimers = React.useCallback(() => {
    for (const timer of deletionFallbackTimersRef.current) {
      clearTimeout(timer);
    }
    deletionFallbackTimersRef.current = [];
  }, []);

  const handleTitleDeletionJobSnapshot = React.useCallback(
    (run: JobRun | null) => {
      if (
        !run ||
        run.jobKey !== "TITLE_DELETION" ||
        !deletionJobIdsRef.current.has(run.id) ||
        !isTerminalJobRunStatus(run.status)
      ) {
        return false;
      }

      deletionJobIdsRef.current.delete(run.id);
      if (deletionJobIdsRef.current.size === 0) {
        clearDeletionFallbackTimers();
        // Physically remove the deleted titles from the context overlay before
        // dropping the pending-id render filter, or the union with
        // titleContextTitles resurrects their cards after the delete completes.
        setTitleContextTitles((current) =>
          current.filter(
            (title) => !pendingDeletedTitleIdsRef.current.has(title.id),
          ),
        );
        setPendingDeletedTitleIds(new Set());
      }
      void refreshTitles();
      return true;
    },
    [clearDeletionFallbackTimers, refreshTitles],
  );

  const refreshTrackedDeletionJobs = React.useCallback(async () => {
    if (deletionJobIdsRef.current.size === 0) {
      return;
    }

    try {
      const { data, error } = await client
        .query<{
          jobRuns?: unknown[];
        }>(
          jobRunsQuery,
          { jobKey: "TITLE_DELETION", limit: 10 },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }

      for (const rawRun of data?.jobRuns ?? []) {
        if (handleTitleDeletionJobSnapshot(normalizeJobRun(rawRun))) {
          break;
        }
      }
    } catch (error) {
      console.error("[title-deletion-job-runs] refresh failed:", error);
    }
  }, [client, handleTitleDeletionJobSnapshot]);

  const scheduleDeletionJobFallbackChecks = React.useCallback(() => {
    clearDeletionFallbackTimers();
    deletionFallbackTimersRef.current =
      TITLE_DELETION_JOB_FALLBACK_DELAYS_MS.map((delayMs) =>
        setTimeout(() => {
          void refreshTrackedDeletionJobs();
        }, delayMs),
      );
  }, [clearDeletionFallbackTimers, refreshTrackedDeletionJobs]);

  React.useEffect(
    () => clearDeletionFallbackTimers,
    [clearDeletionFallbackTimers],
  );

  React.useEffect(() => {
    const refreshIfTrackingDeletion = () => {
      if (deletionJobIdsRef.current.size > 0) {
        void refreshTrackedDeletionJobs();
      }
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        refreshIfTrackingDeletion();
      }
    };

    window.addEventListener("focus", refreshIfTrackingDeletion);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.removeEventListener("focus", refreshIfTrackingDeletion);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [refreshTrackedDeletionJobs]);

  React.useEffect(() => {
    const handleCatalogTitlesRefresh = (event: Event) => {
      const { facet } = catalogTitlesRefreshDetail(event);
      if (facet && facet !== activeFacet) {
        return;
      }
      void refreshTitles();
      void refreshCatalogDiscovery();
    };

    window.addEventListener(
      CATALOG_TITLES_REFRESH_EVENT,
      handleCatalogTitlesRefresh,
    );
    return () =>
      window.removeEventListener(
        CATALOG_TITLES_REFRESH_EVENT,
        handleCatalogTitlesRefresh,
      );
  }, [activeFacet, refreshCatalogDiscovery, refreshTitles]);

  const {
    bulkDeleteDialogOpen,
    setBulkDeleteDialogOpen,
    bulkDeleteFilesOnDisk,
    setBulkDeleteFilesOnDisk,
    bulkDeleteTypedConfirmation,
    setBulkDeleteTypedConfirmation,
    bulkDeletePreviewLoading,
    setBulkDeletePreviewLoading,
    bulkDeletePreviewError,
    setBulkDeletePreviewError,
    setBulkDeletePreviewsByTitleId,
    closeBulkDeleteDialog,
    bulkDeletePreview,
    bulkDeleteConfirmDisabled,
    confirmBulkDeleteTitles,
    openBulkTitleDelete,
  } = useBulkDelete({
    selectedTitles,
    selectedTitleLibraryIds,
    bulkActionBusy,
    setBulkActionBusy,
    client,
    t,
    setGlobalStatus,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    scheduleDeletionJobFallbackChecks,
    setPendingDeletedTitleIds,
    setSelectedTitleIds,
    deletionJobIdsRef,
    batchFailureDetail,
    withFailureDetail,
    aggregateDeletePreviews,
  });

  const {
    bulkRenameDialogOpen,
    bulkRenamePreviewLoading,
    bulkRenamePreviewError,
    bulkRenamePlansByTitleId,
    bulkRenameSummary,
    bulkRenameConfirmDisabled,
    closeBulkRenameDialog,
    confirmBulkRenameTitles,
    openBulkTitleRename,
  } = useBulkRename({
    selectedTitles,
    canRenameSelectedTitles,
    bulkActionBusy,
    setBulkActionBusy,
    client,
    t,
    setGlobalStatus,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    setSelectedTitleIds,
    batchFailureDetail,
    withFailureDetail,
  });

  React.useEffect(() => {
    if (selectedTitles.length > 0) {
      return;
    }
    setBulkEditDialogOpen(false);
    setBulkDeleteDialogOpen(false);
    setBulkDeleteFilesOnDisk(false);
    setBulkDeleteTypedConfirmation("");
    setBulkDeletePreviewLoading(false);
    setBulkDeletePreviewError(null);
    setBulkDeletePreviewsByTitleId({});
    closeBulkRenameDialog();
  }, [
    selectedTitles.length,
    closeBulkRenameDialog,
    setBulkDeleteDialogOpen,
    setBulkDeleteFilesOnDisk,
    setBulkDeletePreviewError,
    setBulkDeletePreviewLoading,
    setBulkDeletePreviewsByTitleId,
    setBulkDeleteTypedConfirmation,
  ]);

  useDeferredWsSubscription<{ data?: { jobRunEvents?: unknown } }>({
    requestKey: "mediaContentTitleDeletionJobRuns",
    request: { query: jobRunEventsSubscription },
    onNext(result) {
      handleTitleDeletionJobSnapshot(
        normalizeJobRun(result.data?.jobRunEvents),
      );
    },
    onError(error) {
      console.error("[title-deletion-job-runs] subscription error:", error);
    },
  });

  const applySelectedOverviewDetail = React.useCallback(
    (titleId: string, title: TitleRecord | null, requestEpoch: number) => {
      if (requestEpoch <= latestCriticalMutationEpochRef.current) {
        return;
      }

      setSelectedOverviewDetailTitle((current) => {
        if (
          selectedOverviewTitleIdRef.current !== titleId &&
          current?.id !== titleId
        ) {
          return current;
        }
        if (!title) {
          return current?.id === titleId ? null : current;
        }
        if (current?.id !== titleId) {
          return title;
        }

        const refreshedTitle = mergePreferLoadedImageFields(current, title);
        // Catalog refreshes deliberately omit recommendation data. Keep the
        // selected panel's current response instead of treating that omission
        // as a reason to reload the rail.
        return title.moreLikeThis === undefined
          ? { ...refreshedTitle, moreLikeThis: current.moreLikeThis }
          : refreshedTitle;
      });

      setTitleContextTitles((current) => {
        const existingIndex = current.findIndex((item) => item.id === titleId);
        if (!title) {
          if (existingIndex === -1) {
            return current;
          }
          return current.filter((item) => item.id !== titleId);
        }
        if (existingIndex === -1) {
          return current;
        }
        const next = [...current];
        next[existingIndex] = mergePreferLoadedImageFields(
          next[existingIndex],
          title,
        );
        return next;
      });
    },
    [],
  );

  const applyRefreshedTitleRecord = React.useCallback(
    (titleId: string, title: TitleRecord | null, requestEpoch: number) => {
      if (requestEpoch <= latestCriticalMutationEpochRef.current) {
        return;
      }

      applySelectedOverviewDetail(titleId, title, requestEpoch);

      setMonitoredTitles((current) => {
        const next = [...current];
        const existingIndex = next.findIndex((item) => item.id === titleId);

        if (!title) {
          if (existingIndex !== -1) {
            next.splice(existingIndex, 1);
          }
          setTitleStatus(t("title.statusTemplate", { count: next.length }));
          return next;
        }

        if (
          !catalogTitleMatchesActiveListFilters(
            title,
            activeCatalogListFiltersRef.current,
          )
        ) {
          if (existingIndex === -1) {
            return current;
          }
          next.splice(existingIndex, 1);
          setTitleStatus(t("title.statusTemplate", { count: next.length }));
          return next;
        }

        if (existingIndex === -1) {
          return current;
        } else {
          next[existingIndex] = mergePreferLoadedImageFields(
            next[existingIndex],
            title,
          );
        }
        setTitleStatus(t("title.statusTemplate", { count: next.length }));
        return next;
      });
    },
    [applySelectedOverviewDetail, setMonitoredTitles, setTitleStatus, t],
  );

  const applyTitleMoreLikeThis = React.useCallback(
    (
      titleId: string,
      moreLikeThis: CatalogDiscoveryItem[],
      requestEpoch: number,
    ) => {
      if (requestEpoch <= latestCriticalMutationEpochRef.current) {
        return;
      }

      const merge = (title: TitleRecord): TitleRecord =>
        title.id === titleId ? { ...title, moreLikeThis } : title;
      const sourceTitle = titleContextSourceTitlesRef.current.find(
        (title) => title.id === titleId,
      );

      setSelectedOverviewDetailTitle((current) => {
        const base =
          current?.id === titleId
            ? current
            : selectedOverviewTitleIdRef.current === titleId
              ? sourceTitle
              : null;
        return base ? merge(base) : current;
      });
    },
    [],
  );

  useTitleListReactiveRefresh({
    facet: activeFacet,
    pause: !shouldLoadCatalogTitles,
    projection: titleCatalogProjection,
    onTitleRefreshed: applyRefreshedTitleRecord,
  });

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles) {
      libraryScanTitleRefreshRef.current = {
        key: "",
        refreshedAt: 0,
        sessionIds: [],
      };
      return;
    }

    if (relevantActiveLibraryScanSessions.length === 0) {
      if (libraryScanTitleRefreshRef.current.sessionIds.length > 0) {
        libraryScanTitleRefreshRef.current = {
          key: "",
          refreshedAt: 0,
          sessionIds: [],
        };
        void refreshLoadedCatalogTitlesQuietly();
        void refreshTitleCatalogFilterOptions();
      }
      return;
    }

    const progressKey = libraryScanProgressKey(
      relevantActiveLibraryScanSessions,
    );
    const now =
      typeof performance !== "undefined" ? performance.now() : Date.now();
    const previous = libraryScanTitleRefreshRef.current;
    const sessionIds = libraryScanSessionIds(
      relevantActiveLibraryScanSessions,
    );
    if (
      didActiveLibraryScanSessionEnd(
        previous.sessionIds,
        relevantActiveLibraryScanSessions,
      )
    ) {
      libraryScanTitleRefreshRef.current = {
        key: progressKey,
        refreshedAt: now,
        sessionIds,
      };
      void refreshLoadedCatalogTitlesQuietly();
      void refreshTitleCatalogFilterOptions();
      return;
    }
    if (
      previous.key === progressKey ||
      (previous.key !== "" &&
        now - previous.refreshedAt < LIBRARY_SCAN_TITLE_REFRESH_THROTTLE_MS)
    ) {
      libraryScanTitleRefreshRef.current = { ...previous, sessionIds };
      return;
    }

    libraryScanTitleRefreshRef.current = {
      key: progressKey,
      refreshedAt: now,
      sessionIds,
    };
    void refreshLoadedCatalogTitlesQuietly({
      firstPageOnly: effectiveViewMode !== "poster",
    });
  }, [
    effectiveViewMode,
    relevantActiveLibraryScanSessions,
    refreshLoadedCatalogTitlesQuietly,
    refreshTitleCatalogFilterOptions,
    shouldLoadCatalogTitles,
  ]);

  const pendingHydrationPosterTitleIds = React.useMemo(() => {
    const nowMs = Date.now();
    return monitoredTitles
      .filter((title) => isPendingHydrationPosterTitle(title, nowMs))
      .map((title) => title.id);
  }, [monitoredTitles]);
  const pendingHydrationPosterTitleIdsKey = React.useMemo(
    () => pendingHydrationPosterTitleIds.join("|"),
    [pendingHydrationPosterTitleIds],
  );
  const selectedOverviewUsesMovieSidePanelRecord =
    selectedOverviewUsesMovieRecord(view);

  const refreshMovieSidePanelOverview = React.useCallback(
    async (titleId: string) => {
      const requestEpoch = reactiveRefreshEpoch();
      const detailResult = await client
        .query<{ title?: TitleRecord | null }>(
          movieSidePanelTitleQuery,
          { id: titleId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (detailResult.error) {
        throw detailResult.error;
      }
      if (detailResult.data?.title) {
        applyRefreshedTitleRecord(
          titleId,
          detailResult.data.title,
          requestEpoch,
        );
      }
    },
    [applyRefreshedTitleRecord, client],
  );

  const refreshMovieTitleOptions = React.useCallback(
    async (title: TitleRecord) => {
      await Promise.all([
        refreshMovieSidePanelOverview(title.id),
        reloadTitles(),
      ]);
    },
    [refreshMovieSidePanelOverview, reloadTitles],
  );

  const updateMovieTitleOptions = React.useCallback(
    async (title: TitleRecord, options: TitleOptionUpdates) => {
      if (title.facet !== "MOVIE") {
        return;
      }
      recordCriticalCatalogMutation();
      const { error } = await client
        .mutation(updateTitleMutation, {
          input: { titleId: title.id, options },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      await refreshMovieTitleOptions(title);
    },
    [client, recordCriticalCatalogMutation, refreshMovieTitleOptions],
  );

  // Movie overviews use the selected-title panel. When a slug deep link selects
  // a movie that is not part of the current catalog page, fetch its panel detail
  // so the pane can render it instead of the list bouncing back.
  const titleContextSourceTitlesRef = React.useRef(titleContextSourceTitles);
  titleContextSourceTitlesRef.current = titleContextSourceTitles;
  React.useEffect(() => {
    const titleId = routeOverviewTitleId;
    if (!titleId) {
      setSelectedOverviewDetailLoading(false);
      return;
    }
    if (!selectedOverviewUsesMovieSidePanelRecord) {
      setSelectedOverviewDetailLoading(false);
      return;
    }
    if (
      titleContextSourceTitlesRef.current.some((title) => title.id === titleId)
    ) {
      setSelectedOverviewDetailLoading(false);
      return;
    }
    let cancelled = false;
    setSelectedOverviewDetailLoading(true);
    void refreshMovieSidePanelOverview(titleId)
      .catch(() => {
        if (!cancelled) {
          onCloseOverview();
        }
      })
      .finally(() => {
        if (!cancelled) {
          setSelectedOverviewDetailLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [
    routeOverviewTitleId,
    refreshMovieSidePanelOverview,
    onCloseOverview,
    selectedOverviewUsesMovieSidePanelRecord,
  ]);

  const previewTitleRename = React.useCallback(
    async (title: TitleRecord): Promise<MediaRenamePlan | null> => {
      try {
        const { data, error } = await client
          .query<{ mediaRenamePreview: MediaRenamePlan }>(
            mediaRenamePreviewQuery,
            {
              input: {
                facet: title.facet,
                titleId: title.id,
                dryRun: true,
              },
            },
          )
          .toPromise();
        if (error) {
          throw error;
        }

        const plan = data?.mediaRenamePreview ?? null;
        if (plan) {
          setGlobalStatus(
            t("status.renamePreviewGenerated", {
              total: plan.total,
              renamable: plan.renamable,
            }),
          );
        }
        return plan;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
        return null;
      }
    },
    [client, setGlobalStatus, t],
  );

  const applyTitleRename = React.useCallback(
    async (title: TitleRecord, _plan: MediaRenamePlan) => {
      try {
        recordCriticalCatalogMutation();
        // One title can be a thousand files, so this starts a job and the
        // title stays locked until the job is done with it.
        const { data, error } = await client
          .mutation<{
            renameTitles: {
              acceptedTitleIds: string[];
              jobRun?: unknown;
            };
          }>(renameTitlesMutation, {
            input: {
              facet: title.facet,
              titleIds: [title.id],
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }
        if ((data?.renameTitles.acceptedTitleIds.length ?? 0) === 0) {
          throw new Error(t("status.bulkRenameFailed"));
        }
        const run = normalizeJobRun(data?.renameTitles.jobRun);
        if (run) {
          registerInteractiveJobRun(run);
        }

        setGlobalStatus(t("status.renameQueued"));
        return true;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
        return false;
      }
    },
    [
      client,
      recordCriticalCatalogMutation,
      registerInteractiveJobRun,
      setGlobalStatus,
      t,
    ],
  );

  const requestDeleteSelectedOverviewMediaFile = React.useCallback(
    (title: TitleRecord, fileId: string) => {
      if (pendingMediaFileDeletionIds.has(fileId)) {
        return;
      }
      const file =
        title.mediaFiles?.find((candidate) => candidate.id === fileId) ?? null;
      if (!file) {
        return;
      }
      setSelectedOverviewMediaFileToDelete({
        titleId: title.id,
        file,
      });
      setSelectedOverviewMediaFileDeleteTypedConfirmation("");
    },
    [pendingMediaFileDeletionIds],
  );

  React.useEffect(
    () => () => {
      for (const unregister of mediaFileDeletionUnregistersRef.current) {
        unregister();
      }
      mediaFileDeletionUnregistersRef.current.clear();
    },
    [],
  );

  const makeSelectedOverviewMovieFilePrimary = React.useCallback(
    async (title: TitleRecord, fileId: string) => {
      if (title.facet !== "MOVIE") {
        return;
      }
      setSelectedOverviewPrimaryMovieFileUpdatingId(fileId);
      try {
        recordCriticalCatalogMutation();
        const { error } = await client
          .mutation(setPrimaryMovieFileMutation, {
            input: {
              titleId: title.id,
              fileId,
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }
        setGlobalStatus(t("status.primaryMovieFileUpdated"));
        await refreshMovieSidePanelOverview(title.id);
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.apiError")),
        );
      } finally {
        setSelectedOverviewPrimaryMovieFileUpdatingId(null);
      }
    },
    [
      client,
      recordCriticalCatalogMutation,
      refreshMovieSidePanelOverview,
      setGlobalStatus,
      t,
    ],
  );

  const selectedOverviewTitleRecord = React.useMemo(
    () => {
      if (!selectedOverviewTitleId) {
        return null;
      }
      return (
        (selectedOverviewDetailTitle?.id === selectedOverviewTitleId
          ? selectedOverviewDetailTitle
          : null) ??
        titleContextSourceTitles.find(
          (title) => title.id === selectedOverviewTitleId,
        ) ??
        null
      );
    },
    [
      selectedOverviewDetailTitle,
      selectedOverviewTitleId,
      titleContextSourceTitles,
    ],
  );
  const selectedPanelHydrationTitleId = selectedOverviewUsesMovieSidePanelRecord
    ? selectedOverviewTitleRecord?.id ?? null
    : null;
  const selectedPanelHydrationMetadataFetchedAt =
    selectedOverviewTitleRecord?.metadataFetchedAt ?? "";
  const selectedPanelHydrationCreatedAt =
    selectedOverviewTitleRecord?.createdAt ?? "";
  const selectedPanelNeedsPanelDetails =
    selectedOverviewTitleRecord !== null
      ? !hasSelectedTitlePanelDetails(selectedOverviewTitleRecord)
      : false;
  const selectedPanelNeedsMovieMediaDetails =
    selectedOverviewTitleRecord !== null
      ? !hasSelectedTitleMovieMediaDetails(selectedOverviewTitleRecord)
      : false;
  const activeCatalogDiscoveryGroups = React.useMemo(() => {
    if (!canManageCatalogDiscovery && !canRequestCatalogDiscovery) {
      return [];
    }
    return catalogDiscoveryGroups.flatMap((group) => {
      const items = group.items.filter(
        (item) => discoveryItemFacet(item) === activeFacet,
      );
      if (items.length === 0) {
        return [];
      }
      return [
        {
          ...group,
          totalCount:
            items.length === group.items.length ? group.totalCount : items.length,
          items,
        },
      ];
    });
  }, [
    activeFacet,
    canManageCatalogDiscovery,
    canRequestCatalogDiscovery,
    catalogDiscoveryGroups,
  ]);

  const loadSelectedOverviewExternalSubtitles = React.useCallback(
    async (titleId: string) => {
      const { data, error } = await client
        .query<{ externalSubtitles?: ExternalSubtitleRecord[] }>(
          externalSubtitlesQuery,
          { titleId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      return data?.externalSubtitles ?? [];
    },
    [client],
  );
  const refreshSelectedOverviewExternalSubtitles = React.useCallback(
    async () => {
      const titleId = selectedPanelHydrationTitleId;
      if (!shouldLoadCatalogTitles || !titleId) {
        setSelectedOverviewExternalSubtitleState({
          titleId: null,
          entries: [],
        });
        return;
      }

      try {
        const entries = await loadSelectedOverviewExternalSubtitles(titleId);
        setSelectedOverviewExternalSubtitleState((current) =>
          current.titleId === titleId ? { titleId, entries } : current,
        );
      } catch (error) {
        console.error(
          "[selected-title-external-subtitles-refresh] refresh failed:",
          error,
        );
        setSelectedOverviewExternalSubtitleState((current) =>
          current.titleId === titleId ? { titleId, entries: [] } : current,
        );
      }
    },
    [
      loadSelectedOverviewExternalSubtitles,
      selectedPanelHydrationTitleId,
      shouldLoadCatalogTitles,
    ],
  );

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles || !selectedPanelHydrationTitleId) {
      selectedPanelHydrationKeyRef.current = null;
      setSelectedOverviewBlocklistState({ titleId: null, entries: [] });
      setSelectedOverviewExternalSubtitleState({ titleId: null, entries: [] });
      return;
    }

    const titleId = selectedPanelHydrationTitleId;
    const requestKey = [
      titleId,
      selectedPanelHydrationMetadataFetchedAt,
      selectedPanelHydrationCreatedAt,
    ].join(":");
    if (selectedPanelHydrationKeyRef.current === requestKey) {
      return;
    }
    selectedPanelHydrationKeyRef.current = requestKey;

    const needsTitleDetails =
      selectedPanelNeedsPanelDetails || selectedPanelNeedsMovieMediaDetails;
    const requestEpoch = reactiveRefreshEpoch();
    // Start every selected-title request together. Keep recommendations
    // independent, though: a slow supporting-panel request must not leave its
    // own ready response loading indefinitely.
    const titleDetailsRequest = needsTitleDetails
      ? client
          .query<{ title?: TitleRecord | null }>(
            movieSidePanelTitleQuery,
            { id: titleId },
            { requestPolicy: "network-only" },
          )
          .toPromise()
      : Promise.resolve(null);
    const recommendationsResult = fetchTitleMoreLikeThis(client, titleId).then(
      (items) => ({ status: "fulfilled" as const, items }),
      (error: unknown) => ({ status: "rejected" as const, error }),
    );

    void recommendationsResult.then((result) => {
      if (selectedPanelHydrationKeyRef.current !== requestKey) {
        return;
      }
      if (result.status === "rejected") {
        console.error(
          "[selected-title-more-like-this-refresh] refresh failed:",
          result.error,
        );
        return;
      }
      window.requestAnimationFrame(() => {
        if (selectedPanelHydrationKeyRef.current !== requestKey) {
          return;
        }
        applyTitleMoreLikeThis(titleId, result.items, requestEpoch);
      });
    });

    void Promise.allSettled([
      titleDetailsRequest,
      loadSelectedOverviewExternalSubtitles(titleId),
      client
        .query<{ titleReleaseBlocklist?: TitleReleaseBlocklistEntry[] }>(
          titleReleaseBlocklistQuery,
          { titleId, limit: 6 },
        )
        .toPromise(),
    ] as const).then(
      ([titleDetailsResult, externalSubtitlesResult, blocklistResult]) => {
        if (selectedPanelHydrationKeyRef.current !== requestKey) {
          return;
        }

        if (titleDetailsResult.status === "rejected") {
          console.error(
            "[selected-title-panel-refresh] refresh failed:",
            titleDetailsResult.reason,
          );
        } else if (titleDetailsResult.value?.error) {
          console.error(
            "[selected-title-panel-refresh] refresh failed:",
            titleDetailsResult.value.error,
          );
        } else if (titleDetailsResult.value?.data?.title) {
          applySelectedOverviewDetail(
            titleId,
            titleDetailsResult.value.data.title,
            requestEpoch,
          );
        }

        const externalSubtitles =
          externalSubtitlesResult.status === "fulfilled"
            ? externalSubtitlesResult.value
            : [];
        if (externalSubtitlesResult.status === "rejected") {
          console.error(
            "[selected-title-external-subtitles-refresh] refresh failed:",
            externalSubtitlesResult.reason,
          );
        }
        setSelectedOverviewExternalSubtitleState({
          titleId,
          entries: externalSubtitles,
        });

        const blocklistEntries =
          blocklistResult.status === "fulfilled" && !blocklistResult.value.error
            ? (blocklistResult.value.data?.titleReleaseBlocklist ?? [])
            : [];
        if (blocklistResult.status === "rejected") {
          console.error(
            "[selected-title-blocklist-refresh] refresh failed:",
            blocklistResult.reason,
          );
        } else if (blocklistResult.value.error) {
          console.error(
            "[selected-title-blocklist-refresh] refresh failed:",
            blocklistResult.value.error,
          );
        }
        setSelectedOverviewBlocklistState({ titleId, entries: blocklistEntries });
      },
    );
  }, [
    applySelectedOverviewDetail,
    applyTitleMoreLikeThis,
    client,
    loadSelectedOverviewExternalSubtitles,
    selectedPanelHydrationCreatedAt,
    selectedPanelHydrationMetadataFetchedAt,
    selectedPanelHydrationTitleId,
    selectedPanelNeedsMovieMediaDetails,
    selectedPanelNeedsPanelDetails,
    shouldLoadCatalogTitles,
  ]);

  React.useEffect(() => {
    if (
      !shouldLoadCatalogTitles ||
      pendingHydrationPosterTitleIds.length === 0
    ) {
      return;
    }

    const refreshPendingHydrationPosters = () => {
      pendingHydrationPosterTitleIds.forEach((titleId) => {
        queueCatalogTitleRefresh({
          titleId,
          projection: titleCatalogProjection,
          apply(title, requestEpoch) {
            applyRefreshedTitleRecord(titleId, title, requestEpoch);
          },
          onError(error) {
            console.error(
              "[catalog-hydration-poster-refresh] refresh failed:",
              error,
            );
          },
        });
      });
    };

    refreshPendingHydrationPosters();
    const intervalId = window.setInterval(
      refreshPendingHydrationPosters,
      HYDRATION_POSTER_REFRESH_INTERVAL_MS,
    );

    return () => {
      window.clearInterval(intervalId);
    };
  }, [
    applyRefreshedTitleRecord,
    pendingHydrationPosterTitleIds,
    pendingHydrationPosterTitleIdsKey,
    queueCatalogTitleRefresh,
    shouldLoadCatalogTitles,
    titleCatalogProjection,
  ]);

  const onAddSubmit = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!titleNameForQueue.trim()) {
        setGlobalStatus(t("status.titleRequired"));
        return;
      }
      if (!queueFacet) {
        setGlobalStatus(t("status.facetRequired"));
        return;
      }

      await runTvdbSearch(titleNameForQueue.trim());
    },
    [queueFacet, runTvdbSearch, setGlobalStatus, titleNameForQueue, t],
  );

  const addTvdbToCatalog = React.useCallback(
    async (candidate: MetadataTvdbSearchItem) => {
      const name = candidate.name.trim();
      if (!name) {
        setGlobalStatus(t("status.titleRequired"));
        return;
      }

      const tvdbId = String(candidate.tvdbId).trim();
      const smgId = candidate.smgId == null ? "" : String(candidate.smgId).trim();
      const tmdbId = candidate.tmdbId == null ? "" : String(candidate.tmdbId).trim();
      const imdbId = candidate.imdbId?.trim();
      const externalIds = [
        ...(candidate.externalIds ?? []),
        ...(smgId ? [{ source: "smg", value: smgId }] : []),
        ...(tvdbId ? [{ source: "tvdb", value: tvdbId }] : []),
        ...(tmdbId ? [{ source: "tmdb", value: tmdbId }] : []),
        ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
      ];

      const monitorType = monitoredForQueue ? "ALL_EPISODES" : "NONE";
      try {
        const { data, error } = await client
          .mutation(addTitleMutation, {
            input: {
              name,
              facet: queueFacet,
              monitored: monitoredForQueue,
              tags: [],
              options: {
                monitorType,
                ...(queueFacet === "MOVIE"
                  ? {}
                  : { useSeasonFolders: seasonFoldersForQueue }),
                ...(queueFacet === "ANIME"
                  ? {
                      monitorSpecials: false,
                      interSeasonMovies: true,
                    }
                  : {}),
              },
              externalIds,
              smgId: candidate.smgId ?? undefined,
              tvdbId: tvdbId || undefined,
              tmdbId: candidate.tmdbId ?? undefined,
              imdbId: imdbId || undefined,
              ...(queueFacet === "MOVIE"
                ? { minAvailability: minAvailabilityForQueue }
                : {}),
            },
          })
          .toPromise();
        if (error) throw error;
        setTitleNameForQueue(data.addTitle.title.name);
        setGlobalStatus(
          t(
            monitoredForQueue
              ? "status.catalogAddSuccessAutoSearch"
              : "status.catalogAddSuccess",
            { name: data.addTitle.title.name },
          ),
        );
        if (shouldLoadCatalogTitles && data?.addTitle?.title) {
          mergeTitleContextTitles([data.addTitle.title as TitleRecord]);
          setMonitoredTitles((current) => {
            const title = data.addTitle.title as TitleRecord;
            const next = upsertCatalogTitleRecord(
              current,
              title,
              activeCatalogListFiltersRef.current,
            );
            if (next !== current) {
              setTitleStatus(t("title.statusTemplate", { count: next.length }));
            }
            return next;
          });
        }
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.queueFailed"),
        );
      }
    },
    [
      minAvailabilityForQueue,
      monitoredForQueue,
      queueFacet,
      client,
      mergeTitleContextTitles,
      shouldLoadCatalogTitles,
      setMonitoredTitles,
      setGlobalStatus,
      setTitleStatus,
      t,
      seasonFoldersForQueue,
      setTitleNameForQueue,
    ],
  );

  const queueExisting = React.useCallback(
    async (title: TitleRecord) => {
      try {
        const input = {
          titleId: title.id,
          scope: { title: true },
        };
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client
              .mutation(queueBestReleaseMutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return data?.queueBestRelease;
          },
          "A download is already in progress for this title.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(
          payload,
          "A download is already in progress for this title.",
        );
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.queueFailed")),
        );
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  const runInteractiveSearchForTitle = React.useCallback(
    async (
      title: TitleRecord,
      onUpdate?: (snapshot: InteractiveSearchProgress) => void,
    ) => {
      interactiveSearchAbortRef.current?.abort();
      const abortController = new AbortController();
      interactiveSearchAbortRef.current = abortController;

      try {
        const results = await runIterativeReleaseSearch(
          client,
          { titleId: title.id },
          { signal: abortController.signal, onUpdate },
        );
        if (abortController.signal.aborted) {
          return [];
        }
        return results;
      } catch (error) {
        if (abortController.signal.aborted || isAbortError(error)) {
          return [];
        }
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.searchFailed"),
        );
        return [];
      } finally {
        if (interactiveSearchAbortRef.current === abortController) {
          interactiveSearchAbortRef.current = null;
        }
      }
    },
    [client, setGlobalStatus, t],
  );

  const queueExistingFromRelease = React.useCallback(
    async (title: TitleRecord, release: Release) => {
      if (!release.candidateToken) {
        const message = t("status.releaseMissingCandidateToken");
        setGlobalStatus(message);
        throw new Error(message);
      }

      try {
        const input = {
          titleId: title.id,
          scope: releaseQueueScopeInput(release, { title: true }),
          candidateToken: release.candidateToken,
          sizeBytes: release.sizeBytes ?? null,
        };
        const replacesPrimary = hasPrimaryMediaFile(title.mediaFiles);
        const mutation = replacesPrimary
          ? queueReplacementMutation
          : queueExistingMutation;
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client
              .mutation(mutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return replacesPrimary
              ? data?.queueReplacementRelease
              : data?.queueExistingTitleDownload;
          },
          "A download is already in progress for this title.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(
          payload,
          "A download is already in progress for this title.",
        );
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.queueFailed")),
        );
        throw error;
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  const queueAdditionalFromRelease = React.useCallback(
    async (title: TitleRecord, release: Release) => {
      if (!release.candidateToken) {
        const message = t("status.releaseMissingCandidateToken");
        setGlobalStatus(message);
        throw new Error(message);
      }

      try {
        const { data, error } = await client
          .mutation(queueExistingMutation, {
            input: {
              titleId: title.id,
              scope: releaseQueueScopeInput(release, { title: true }),
              candidateToken: release.candidateToken,
              sizeBytes: release.sizeBytes ?? null,
              purpose: "ADDITIONAL_FILE",
            },
          })
          .toPromise();
        if (error) throw error;
        assertNoReplaceConflict(
          data?.queueExistingTitleDownload,
          "A download is already in progress for this title.",
        );
        setGlobalStatus(t("status.queuedLatest", { name: title.name }));
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.queueFailed")),
        );
        throw error;
      }
    },
    [client, setGlobalStatus, t],
  );

  const toggleTitleMonitored = React.useCallback(
    async (title: TitleRecord, monitored: boolean) => {
      const titleId = title.id;
      setTitleMonitoringLoadingById((previous) => ({
        ...previous,
        [titleId]: true,
      }));
      try {
        const { error } = await client
          .mutation(setTitleMonitoredMutation, {
            input: { titleId, monitored },
          })
          .toPromise();
        if (error) throw error;
        setMonitoredTitles((previous) =>
          previous.map((item) =>
            item.id === titleId ? { ...item, monitored } : item,
          ),
        );
        setTitleContextTitles((previous) =>
          previous.map((item) =>
            item.id === titleId ? { ...item, monitored } : item,
          ),
        );
        setGlobalStatus(
          monitored
            ? t("status.titleMonitoringEnabled")
            : t("status.titleMonitoringDisabled"),
        );
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
      } finally {
        setTitleMonitoringLoadingById((previous) => {
          const next = { ...previous };
          delete next[titleId];
          return next;
        });
      }
    },
    [client, setGlobalStatus, setMonitoredTitles, t],
  );

  const toggleTitleSelection = React.useCallback((titleId: string) => {
    setSelectedTitleIds((current) => {
      const next = new Set(current);
      if (next.has(titleId)) {
        next.delete(titleId);
      } else {
        next.add(titleId);
      }
      return next;
    });
  }, []);

  const toggleTitleQuickMonitoringFilter = React.useCallback(
    (nextFilter: "monitored" | "unmonitored") => {
      React.startTransition(() => {
        setTitleQuickFilters((current) => ({
          ...current,
          monitored: nextFilter === "monitored" ? !current.monitored : current.monitored,
          unmonitored:
            nextFilter === "unmonitored"
              ? !current.unmonitored
              : current.unmonitored,
        }));
      });
    },
    [],
  );

  const toggleTitleQuickStatusFilter = React.useCallback(
    (nextFilter: "continuing" | "ended") => {
      React.startTransition(() => {
        setTitleQuickFilters((current) => ({
          ...current,
          continuing:
            nextFilter === "continuing" ? !current.continuing : current.continuing,
          ended: nextFilter === "ended" ? !current.ended : current.ended,
        }));
      });
    },
    [],
  );

  const clearTitleQuickFilters = React.useCallback(() => {
    React.startTransition(() => {
      setTitleQuickFilters(EMPTY_TITLE_QUICK_FILTERS);
    });
  }, []);

  const updateTitleCatalogSort = React.useCallback(
    (nextKey: TitleTableSortKey) => {
      setTitleCatalogSort((current) => {
        if (current.key === nextKey) {
          return {
            key: nextKey,
            direction: current.direction === "asc" ? "desc" : "asc",
          };
        }

        return {
          key: nextKey,
          direction: defaultSortDirectionForTitleKey(nextKey),
        };
      });
    },
    [],
  );

  const toggleAllVisibleTitles = React.useCallback(
    (checked: boolean) => {
      setSelectedTitleIds(
        checked ? new Set(visibleTitles.map((title) => title.id)) : new Set(),
      );
    },
    [visibleTitles],
  );

  const clearSelectedTitles = React.useCallback(() => {
    setSelectedTitleIds((current) =>
      current.size === 0 ? current : new Set(),
    );
  }, []);

  const selectOverviewTitle = React.useCallback((titleId: string | null) => {
    setSelectedOverviewTitleId(titleId);
  }, []);

  const clearSelectedOverviewTitle = React.useCallback(() => {
    selectedOverviewTitleIdRef.current = null;
    setSelectedOverviewTitleId(null);
    setSelectedOverviewDetailTitle(null);
  }, []);

  const handleCloseOverview = React.useCallback(() => {
    clearSelectedOverviewTitle();
    onCloseOverview();
  }, [clearSelectedOverviewTitle, onCloseOverview]);

  const setViewMode = React.useCallback(
    (nextMode: ContentViewMode) => {
      setDesktopViewModes((current) => ({
        ...current,
        [view]: nextMode,
      }));
    },
    [view],
  );

  const bulkMonitorTitles = React.useCallback(
    async (monitored: boolean) => {
      const targets = [...selectedTitles];
      if (targets.length === 0 || bulkActionBusy) {
        return;
      }

      setBulkActionBusy(true);
      try {
        const variables = Object.fromEntries(
          targets.map((title, index) => [
            `input${index}`,
            { titleId: title.id, monitored },
          ]),
        );
        const result = await client
          .mutation<
            Record<string, { id: string; monitored: boolean }>
          >(buildSetTitleMonitoredBatchMutation(targets.length), variables)
          .toPromise();
        const payload = result.data ?? {};
        const refreshedTitles = await reloadTitles();
        let { succeededIds, failedIds } = refreshedTitles
          ? inferMonitoredBatchOutcome(targets, refreshedTitles, monitored)
          : {
              succeededIds: [] as string[],
              failedIds: [...targets.map((title) => title.id)],
            };
        if (!refreshedTitles && !result.error) {
          succeededIds = [];
          failedIds = [];
          targets.forEach((title, index) => {
            if (payload[batchItemAlias(index)]) {
              succeededIds.push(title.id);
            } else {
              failedIds.push(title.id);
            }
          });
        }
        setSelectedTitleIds(new Set(failedIds));

        const detail = batchFailureDetail(result.error);
        if (succeededIds.length === 0) {
          setGlobalStatus(
            withFailureDetail(
              monitored
                ? t("status.bulkMonitorFailed")
                : t("status.bulkUnmonitorFailed"),
              detail,
            ),
          );
          return;
        }

        if (failedIds.length > 0) {
          setGlobalStatus(
            withFailureDetail(
              monitored
                ? t("status.bulkMonitorPartial", {
                    count: succeededIds.length,
                    failed: failedIds.length,
                  })
                : t("status.bulkUnmonitorPartial", {
                    count: succeededIds.length,
                    failed: failedIds.length,
                  }),
              detail,
            ),
          );
          return;
        }

        setGlobalStatus(
          monitored
            ? t("status.bulkMonitorSuccess", { count: succeededIds.length })
            : t("status.bulkUnmonitorSuccess", { count: succeededIds.length }),
        );
      } catch (error) {
        setGlobalStatus(
          withFailureDetail(
            monitored
              ? t("status.bulkMonitorFailed")
              : t("status.bulkUnmonitorFailed"),
            batchFailureDetail(error),
          ),
        );
      } finally {
        setBulkActionBusy(false);
      }
    },
    [bulkActionBusy, client, reloadTitles, selectedTitles, setGlobalStatus, t],
  );

  const applyBulkTitleOptions = React.useCallback(
    async (changes: TitleOptionUpdates) => {
      const targets = [...editDialogTitles];
      if (targets.length === 0 || bulkActionBusy) {
        return;
      }

      setBulkActionBusy(true);
      try {
        const variables = Object.fromEntries(
          targets.map((title, index) => [
            `input${index}`,
            {
              titleId: title.id,
              options: changes,
            },
          ]),
        );
        const result = await client
          .mutation<
            Record<string, { id: string }>
          >(buildUpdateTitleBatchMutation(targets.length), variables)
          .toPromise();
        const payload = result.data ?? {};
        const refreshedTitles = await reloadTitles();
        let { succeededIds, failedIds } = refreshedTitles
          ? inferTitleUpdateBatchOutcome(targets, refreshedTitles, changes)
          : {
              succeededIds: [] as string[],
              failedIds: [...targets.map((title) => title.id)],
            };
        if (!refreshedTitles && !result.error) {
          succeededIds = [];
          failedIds = [];
          targets.forEach((title, index) => {
            if (payload[batchItemAlias(index)]) {
              succeededIds.push(title.id);
            } else {
              failedIds.push(title.id);
            }
          });
        }
        setSelectedTitleIds(new Set(failedIds));

        const detail = batchFailureDetail(result.error);
        if (succeededIds.length === 0) {
          setGlobalStatus(
            withFailureDetail(t("status.bulkTitleUpdateFailed"), detail),
          );
          return;
        }

        setBulkEditDialogOpen(false);
        if (failedIds.length > 0) {
          setGlobalStatus(
            withFailureDetail(
              t("status.bulkTitleUpdatePartial", {
                count: succeededIds.length,
                failed: failedIds.length,
              }),
              detail,
            ),
          );
          return;
        }

        setGlobalStatus(
          t("status.bulkTitleUpdateSuccess", { count: succeededIds.length }),
        );
      } catch (error) {
        setGlobalStatus(
          withFailureDetail(
            t("status.bulkTitleUpdateFailed"),
            batchFailureDetail(error),
          ),
        );
      } finally {
        setBulkActionBusy(false);
      }
    },
    [
      bulkActionBusy,
      client,
      editDialogTitles,
      reloadTitles,
      setGlobalStatus,
      t,
    ],
  );

  React.useEffect(() => {
    if (rootFolderValidationSnapshot === null) {
      setInvalidRootLibraryIds([]);
      setInvalidRootPathsByLibraryId({});
      setRootValidationUnavailable(false);
      setValidatedRootFolderSnapshotKey(null);
      return;
    }

    const { key, librariesWithConfiguredRoots } =
      rootFolderValidationSnapshot;

    if (librariesWithConfiguredRoots.length === 0) {
      setInvalidRootLibraryIds([]);
      setInvalidRootPathsByLibraryId({});
      setRootValidationUnavailable(false);
      setValidatedRootFolderSnapshotKey(key);
      return;
    }

    let cancelled = false;

    const validateRoots = async () => {
      const invalidIds = new Set<string>();
      const invalidPathsByLibraryId: Record<string, string[]> = {};
      const pathEntries = librariesWithConfiguredRoots.flatMap((library) =>
        library.roots
          .map((root) => root.path.trim())
          .filter((path) => path.length > 0)
          .map((path) => ({ libraryId: library.id, path })),
      );
      const validation = await validateLibraryRootPaths(
        pathEntries.map(({ path }) => path),
        async (path) => {
          const { error } = await client
            .query(
              browsePathQuery,
              { path },
              { requestPolicy: "network-only" },
            )
            .toPromise();
          return error;
        },
      );
      const invalidPaths = new Set(validation.invalidPaths);
      for (const { libraryId, path } of pathEntries) {
        if (!invalidPaths.has(path)) continue;
        invalidIds.add(libraryId);
        (invalidPathsByLibraryId[libraryId] ??= []).push(path);
      }

      if (!cancelled) {
        setInvalidRootLibraryIds([...invalidIds]);
        setInvalidRootPathsByLibraryId(invalidPathsByLibraryId);
        setRootValidationUnavailable(validation.unavailable);
        setValidatedRootFolderSnapshotKey(key);
      }
    };

    void validateRoots().catch((error) => {
      console.error(
        "[library-root-validation] failed to validate root folders:",
        error,
      );
      if (!cancelled) {
        setInvalidRootLibraryIds([]);
        setInvalidRootPathsByLibraryId({});
        setRootValidationUnavailable(true);
        setValidatedRootFolderSnapshotKey(key);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [client, rootFolderValidationSnapshot]);

  const openBulkTitleEdit = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    if (selectedTitleLibraryIds.length !== 1) {
      setGlobalStatus("Bulk actions require titles from one library.");
      return;
    }
    setBulkEditDialogOpen(true);
  }, [
    bulkActionBusy,
    selectedTitleLibraryIds.length,
    selectedTitles.length,
    setGlobalStatus,
  ]);

  const requestDeleteTitle = React.useCallback(
    (title: TitleRecord) => {
      setTitleToDelete(title);
      setDeleteFilesOnDisk(false);
      setTitleDeleteTypedConfirmation("");
    },
    [setTitleDeleteTypedConfirmation, setTitleToDelete, setDeleteFilesOnDisk],
  );

  const closeDeleteTitleDialog = React.useCallback(() => {
    setTitleToDelete(null);
    setDeleteFilesOnDisk(false);
    setTitleDeleteTypedConfirmation("");
  }, [setDeleteFilesOnDisk, setTitleDeleteTypedConfirmation, setTitleToDelete]);

  React.useEffect(() => {
    if (!deleteFilesOnDisk) {
      setTitleDeleteTypedConfirmation("");
    }
  }, [deleteFilesOnDisk]);

  const confirmDeleteTitle = React.useCallback(async () => {
    if (!titleToDelete) {
      return;
    }

    const titleId = titleToDelete.id;
    setDeleteTitleLoadingById((previous) => ({
      ...previous,
      [titleId]: true,
    }));

    try {
      let previewFingerprint: string | undefined;
      if (deleteFilesOnDisk) {
        if (!titleDeletePreview) {
          throw new Error("Delete preview is not ready yet.");
        }
        previewFingerprint = titleDeletePreview.fingerprint;
      }

      const result = await client
        .mutation<{
          deleteTitles?: {
            acceptedTitleIds?: string[];
            jobRun?: unknown;
          };
        }>(deleteTitlesMutation, {
          input: {
            items: [
              {
                titleId,
                ...(deleteFilesOnDisk ? { previewFingerprint } : {}),
              },
            ],
            deleteFilesOnDisk,
            ...(deleteFilesOnDisk && titleDeleteTypedConfirmation.trim()
              ? { typedConfirmation: titleDeleteTypedConfirmation.trim() }
              : {}),
          },
        })
        .toPromise();
      if (result.error) throw result.error;
      const acceptedIds = result.data?.deleteTitles?.acceptedTitleIds ?? [];
      if (acceptedIds.length > 0) {
        recordCriticalCatalogMutation();
      }
      const run = normalizeJobRun(result.data?.deleteTitles?.jobRun);
      if (run) {
        deletionJobIdsRef.current.add(run.id);
        registerInteractiveJobRun(run);
        scheduleDeletionJobFallbackChecks();
      }
      setPendingDeletedTitleIds((current) => {
        const next = new Set(current);
        for (const id of acceptedIds) {
          next.add(id);
        }
        return next;
      });
      if (
        routeOverviewTitleId !== null &&
        acceptedIds.includes(routeOverviewTitleId)
      ) {
        handleCloseOverview();
      }
      setGlobalStatus(`Queued deletion for ${titleToDelete.name}.`);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setDeleteTitleLoadingById((previous) => {
        const next = { ...previous };
        delete next[titleId];
        return next;
      });
      closeDeleteTitleDialog();
    }
  }, [
    closeDeleteTitleDialog,
    deleteFilesOnDisk,
    client,
    handleCloseOverview,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    scheduleDeletionJobFallbackChecks,
    titleDeletePreview,
    titleDeleteTypedConfirmation,
    t,
    titleToDelete,
    routeOverviewTitleId,
    setGlobalStatus,
    setDeleteTitleLoadingById,
  ]);

  const closeSelectedOverviewMediaFileDeleteDialog = React.useCallback(() => {
    if (selectedOverviewMediaFileDeleteLoading) {
      return;
    }
    setSelectedOverviewMediaFileToDelete(null);
    setSelectedOverviewMediaFileDeleteTypedConfirmation("");
  }, [selectedOverviewMediaFileDeleteLoading]);

  const confirmDeleteSelectedOverviewMediaFile = React.useCallback(async () => {
    if (
      !selectedOverviewMediaFileToDelete ||
      !selectedOverviewMediaFileDeletePreview
    ) {
      return;
    }
    setSelectedOverviewMediaFileDeleteLoading(true);
    try {
      const target = selectedOverviewMediaFileToDelete;
      const { data, error } = await client
        .mutation<{
          deleteMediaFile?: { jobRun?: unknown };
        }>(deleteMediaFileMutation, {
          input: {
            fileId: target.file.id,
            deleteFromDisk: true,
            previewFingerprint: selectedOverviewMediaFileDeletePreview.fingerprint,
            typedConfirmation:
              selectedOverviewMediaFileDeleteTypedConfirmation.trim() ||
              undefined,
          },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      const run = normalizeJobRun(data?.deleteMediaFile?.jobRun);
      if (!run) {
        throw new Error(t("status.apiError"));
      }
      setPendingMediaFileDeletionIds((current) => new Set(current).add(target.file.id));
      const unregister = registerInteractiveJobRun(run, () => {
        unregister();
        mediaFileDeletionUnregistersRef.current.delete(unregister);
        setPendingMediaFileDeletionIds((current) => {
          const next = new Set(current);
          next.delete(target.file.id);
          return next;
        });
        void refreshMovieSidePanelOverview(target.titleId);
      });
      mediaFileDeletionUnregistersRef.current.add(unregister);
      recordCriticalCatalogMutation();
      setGlobalStatus("Queued media file deletion.");
      setSelectedOverviewMediaFileToDelete(null);
      setSelectedOverviewMediaFileDeleteTypedConfirmation("");
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.apiError")),
      );
    } finally {
      setSelectedOverviewMediaFileDeleteLoading(false);
    }
  }, [
    client,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    refreshMovieSidePanelOverview,
    selectedOverviewMediaFileDeletePreview,
    selectedOverviewMediaFileDeleteTypedConfirmation,
    selectedOverviewMediaFileToDelete,
    setGlobalStatus,
    t,
  ]);

  const deleteTitleConfirmDisabled =
    deleteFilesOnDisk &&
    (titleDeletePreviewLoading ||
      !!titleDeletePreviewError ||
      !titleDeletePreview ||
      (titleDeletePreview.requiresTypedConfirmation &&
        titleDeleteTypedConfirmation.trim() !== "DELETE"));
  const deleteSelectedOverviewMediaFileConfirmDisabled =
    selectedOverviewMediaFileDeletePreviewLoading ||
    !!selectedOverviewMediaFileDeletePreviewError ||
    !selectedOverviewMediaFileDeletePreview ||
    (selectedOverviewMediaFileDeletePreview.requiresTypedConfirmation &&
      selectedOverviewMediaFileDeleteTypedConfirmation.trim() !== "DELETE");

  const refreshLibraries = React.useCallback(async (): Promise<
    LibraryRecord[] | null
  > => {
    if (!isMediaView) {
      setLibraries([]);
      setLibrariesFacet(null);
      return [];
    }
    const permission =
      contentSettingsSection === "library" && !canManageConfig
        ? "MANAGE_LIBRARY"
        : "VIEW";
    setLibrariesLoading(true);
    try {
      const { data, error } = await client
        .query(
          librariesQuery,
          { facet: activeFacet, permission },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const nextLibraries = (data?.libraries ?? []) as LibraryRecord[];
      setLibraries(nextLibraries);
      setLibrariesFacet(activeFacet);
      setSelectedLibraryIds((current) => {
        const normalized = normalizeLibraryFilterSelection(current, nextLibraries);
        return sameStringArray(current, normalized) ? current : normalized;
      });
      return nextLibraries;
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
      return null;
    } finally {
      setLibrariesLoading(false);
    }
  }, [
    activeFacet,
    canManageConfig,
    client,
    contentSettingsSection,
    isMediaView,
    setGlobalStatus,
    setSelectedLibraryIds,
    t,
  ]);

  const refreshRootValidationLibraries = React.useCallback(async (): Promise<
    LibraryRecord[] | null
  > => {
    if (!isMediaView) {
      setRootValidationLibraries([]);
      return [];
    }
    const permission =
      contentSettingsSection === "library" && !canManageConfig
        ? "MANAGE_LIBRARY"
        : "VIEW";
    setRootValidationLibrariesLoading(true);
    try {
      const { data, error } = await client
        .query(
          librariesQuery,
          { facet: null, permission },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const nextLibraries = (data?.libraries ?? []) as LibraryRecord[];
      setRootValidationLibraries(nextLibraries);
      return nextLibraries;
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
      return null;
    } finally {
      setRootValidationLibrariesLoading(false);
    }
  }, [
    canManageConfig,
    client,
    contentSettingsSection,
    isMediaView,
    setGlobalStatus,
    t,
  ]);

  const loadLibrarySettings = React.useCallback(
    async (libraryId: string): Promise<LibrarySettingsRecord | null> => {
      const { data, error } = await client
        .query<{
          librarySettings: LibrarySettingsRecord;
        }>(
          librarySettingsQuery,
          { libraryId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      return data?.librarySettings ?? null;
    },
    [client],
  );

  const loadFacetDownloadClientRouting = React.useCallback(
    async (
      scopeId: LibraryRecord["facet"],
    ): Promise<DownloadClientRoutingEntry[]> => {
      const { data, error } = await client
        .query<{
          downloadClientRouting: DownloadClientRoutingEntry[];
        }>(
          downloadClientRoutingQuery,
          { scopeId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      return data?.downloadClientRouting ?? [];
    },
    [client],
  );

  React.useEffect(() => {
    const canManageDownloadClientRouting =
      canManageSystemSettings || canManageCatalogSettings;

    if (
      !isMediaView ||
      contentSettingsSection !== "library" ||
      !canManageDownloadClientRouting
    ) {
      setLibraryDownloadClients([]);
      setLibraryDownloadClientsLoading(false);
      return;
    }

    let cancelled = false;
    setLibraryDownloadClientsLoading(true);
    void client
      .query<{ downloadClientConfigs: DownloadClientRecord[] }>(
        libraryDownloadClientsQuery,
        {},
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          throw error;
        }
        setLibraryDownloadClients(data?.downloadClientConfigs ?? []);
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      })
      .finally(() => {
        if (!cancelled) {
          setLibraryDownloadClientsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    canManageCatalogSettings,
    canManageSystemSettings,
    client,
    contentSettingsSection,
    isMediaView,
    setGlobalStatus,
    t,
  ]);

  const createLibrary = React.useCallback(
    async (input: {
      name: string;
      roots: RootFolderOption[];
      settings?: LibrarySettingsDraft;
    }) => {
      setLibrarySettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ createLibrary: LibraryRecord }>(createLibraryMutation, {
            input: {
              facet: activeFacet,
              name: input.name,
              roots: libraryRootsInput(input.roots),
              settings: librarySettingsInput(input.settings),
            },
          })
          .toPromise();
        if (error) throw error;
        const library = data?.createLibrary ?? null;
        await refreshLibraries();
        await refreshRootValidationLibraries();
        if (library) {
          setSelectedLibraryIds([library.id]);
          setGlobalStatus(t("settings.libraryCreated"));
          toast.success(t("settings.libraryCreated"));
        }
        return library;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("settings.librarySaveFailed"),
        );
        return null;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [
      activeFacet,
      client,
      refreshLibraries,
      refreshRootValidationLibraries,
      setGlobalStatus,
      setSelectedLibraryIds,
      t,
    ],
  );

  const updateLibrary = React.useCallback(
    async (
      libraryId: string,
      input: {
        name: string;
        roots: RootFolderOption[];
        settings?: LibrarySettingsDraft;
      },
    ) => {
      setLibrarySettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ updateLibrary: LibraryRecord }>(updateLibraryMutation, {
            input: {
              libraryId,
              name: input.name,
              roots: libraryRootsInput(input.roots),
              settings: librarySettingsInput(input.settings),
            },
          })
          .toPromise();
        if (error) throw error;
        const library = data?.updateLibrary ?? null;
        await refreshLibraries();
        await refreshRootValidationLibraries();
        if (library) {
          setGlobalStatus(t("settings.librarySaved"));
          toast.success(t("settings.librarySaved"));
        }
        return library;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("settings.librarySaveFailed"),
        );
        return null;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [
      client,
      refreshLibraries,
      refreshRootValidationLibraries,
      setGlobalStatus,
      t,
    ],
  );

  const deleteLibrary = React.useCallback(
    async (libraryId: string) => {
      setLibrarySettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ deleteLibrary: { id: string } }>(
            deleteLibraryMutation,
            {
              id: libraryId,
            },
          )
          .toPromise();
        if (error) throw error;
        if (!data?.deleteLibrary?.id) {
          throw new Error(t("settings.libraryDeleteFailed"));
        }
        setSelectedLibraryIds((current) =>
          current.filter(
            (selectedLibraryId) => selectedLibraryId !== libraryId,
          ),
        );
        await refreshLibraries();
        await refreshRootValidationLibraries();
        setGlobalStatus(t("settings.libraryDeleted"));
        return true;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("settings.libraryDeleteFailed"),
        );
        return false;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [
      client,
      refreshLibraries,
      refreshRootValidationLibraries,
      setGlobalStatus,
      setSelectedLibraryIds,
      t,
    ],
  );

  const handleLibraryScan = React.useCallback(
    async (libraryId?: string) => {
      const targetLibraryId = libraryId ?? effectiveLibraryScanTargetId;
      if (!targetLibraryId) {
        setGlobalStatus("Choose a library to scan.");
        return;
      }
      if (getActiveSession(activeFacet, targetLibraryId)) {
        setLibraryScanUiStateByLibraryId((current) => ({
          ...current,
          [targetLibraryId]: {
            loading: false,
            sessionId: current[targetLibraryId]?.sessionId ?? null,
            notice: t("settings.libraryScanAlreadyRunning", {
              facet: activeFacetLabel,
            }),
            summary: current[targetLibraryId]?.summary ?? null,
          },
        }));
        return;
      }

      setLibraryScanUiStateByLibraryId((current) => ({
        ...current,
        [targetLibraryId]: {
          loading: true,
          sessionId: null,
          notice: null,
          summary: null,
        },
      }));
      try {
        const result = await client
          .mutation(scanLibraryMutation, {
            input: { libraryId: targetLibraryId },
          })
          .toPromise();
        if (result.error) throw result.error;
        const sessionId = result.data?.scanLibrary?.sessionId ?? null;
        setLibraryScanUiStateByLibraryId((current) => ({
          ...current,
          [targetLibraryId]: {
            ...(current[targetLibraryId] ?? {
              notice: null,
              summary: null,
            }),
            loading: false,
            sessionId,
          },
        }));
        void refreshLibraryScanSessions().catch((error) => {
          console.error(
            "[library-scan] failed to refresh active scan sessions:",
            error,
          );
        });
      } catch (error) {
        console.error("[library-scan] mutation failed:", error);
        const message =
          error instanceof Error ? error.message : String(error ?? "");
        if (/library scan already running/i.test(message)) {
          setLibraryScanUiStateByLibraryId((current) => ({
            ...current,
            [targetLibraryId]: {
              loading: false,
              sessionId: current[targetLibraryId]?.sessionId ?? null,
              notice: t("settings.libraryScanAlreadyRunning", {
                facet: activeFacetLabel,
              }),
              summary: current[targetLibraryId]?.summary ?? null,
            },
          }));
          return;
        }
        if (
          error != null &&
          typeof error === "object" &&
          "networkError" in error &&
          (error as { networkError?: unknown }).networkError != null
        ) {
          toast.error(
            error instanceof Error
              ? error.message
              : t("settings.libraryScanFailed"),
          );
          setGlobalStatus(
            error instanceof Error
              ? error.message
              : t("settings.libraryScanFailed"),
          );
          return;
        }
        setGlobalStatus(
          error instanceof Error
            ? error.message
            : t("settings.libraryScanFailed"),
        );
      } finally {
        setLibraryScanUiStateByLibraryId((current) => ({
          ...current,
          [targetLibraryId]: {
            ...(current[targetLibraryId] ?? {
              sessionId: null,
              notice: null,
              summary: null,
            }),
            loading: false,
          },
        }));
      }
    },
    [
      activeFacet,
      activeFacetLabel,
      client,
      effectiveLibraryScanTargetId,
      getActiveSession,
      refreshLibraryScanSessions,
      setGlobalStatus,
      t,
    ],
  );

  React.useEffect(() => {
    if (!titleStatus) {
      setTitleStatus(t("title.noManaged"));
    }
  }, [t, titleStatus, setTitleStatus]);

  React.useEffect(() => {
    if (shouldLoadCatalogTitles) {
      return;
    }
    void refreshLibraries();
  }, [refreshLibraries, shouldLoadCatalogTitles]);

  React.useLayoutEffect(() => {
    if (shouldLoadCatalogTitles) {
      return;
    }

    catalogBootstrapRequestSeqRef.current += 1;
    catalogBootstrapInFlightKeyRef.current = null;
    setCatalogBootstrapState((current) =>
      current.key === "" && current.phase === "resolving"
        ? current
        : { key: "", phase: "resolving", error: null },
    );
  }, [shouldLoadCatalogTitles]);

  React.useEffect(() => {
    if (contentSettingsSection !== "library" || !isMediaView) {
      setRootValidationLibraries([]);
      setRootValidationLibrariesLoading(false);
      return;
    }
    void refreshRootValidationLibraries();
  }, [contentSettingsSection, isMediaView, refreshRootValidationLibraries]);

  // Load media settings once per view/scope change (subscription handles live updates).
  // Deferred pattern: StrictMode unmount/remount cancels the stale call.
  React.useEffect(() => {
    if (!shouldLoadMediaSettings) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      if (!cancelled) void refreshMediaSettings();
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [shouldLoadMediaSettings, refreshMediaSettings]);

  React.useEffect(() => {
    if (!isMediaView) {
      return;
    }

    const isGeneralSettingsSection =
      contentSettingsSection === "library" ||
      contentSettingsSection === "general";
    const isRoutingSection = contentSettingsSection === "routing";

    if (shouldLoadCatalogTitles) {
      if (catalogSurfaceState.phase === "resolving") {
        if (catalogBootstrapInFlightKeyRef.current === catalogBootstrapKey) {
          return;
        }

        const requestSeq = ++catalogBootstrapRequestSeqRef.current;
        catalogBootstrapInFlightKeyRef.current = catalogBootstrapKey;
        skipNextCatalogOverviewReloadRef.current = false;
        setCatalogBootstrapState({
          key: catalogBootstrapKey,
          phase: "resolving",
          error: null,
        });

        const commitBootstrapState = (
          phase: CatalogSurfacePhase,
          error: string | null = null,
        ) => {
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }

          catalogBootstrapInFlightKeyRef.current = null;
          skipNextCatalogOverviewReloadRef.current = true;
          setCatalogBootstrapState({
            key: catalogBootstrapKey,
            phase,
            error,
          });
        };

        void (async () => {
          const nextLibraries = await refreshLibraries();
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }
          if (nextLibraries === null) {
            commitBootstrapState("error", t("status.failedToLoad"));
            return;
          }

          const normalizedSelectedLibraryIds = normalizeLibraryFilterSelection(
            selectedLibraryIds,
            nextLibraries,
          );
          if (
            !sameStringArray(selectedLibraryIds, normalizedSelectedLibraryIds)
          ) {
            catalogBootstrapInFlightKeyRef.current = null;
            setSelectedLibraryIds(normalizedSelectedLibraryIds);
            return;
          }

          const scopedLibraries =
            normalizedSelectedLibraryIds.length === 0 ||
            normalizedSelectedLibraryIds.includes(ALL_LIBRARIES_VALUE)
            ? nextLibraries
            : nextLibraries.filter((library) =>
                normalizedSelectedLibraryIds.includes(library.id),
              );
          const configuredLibraries =
            configuredCatalogLibraries(scopedLibraries);
          let hasConfiguredRoots = configuredLibraries.length > 0;

          const rootConfigurationPhase = resolveCatalogSurfacePhase({
            canManageLibrarySettings,
            hasConfiguredRoots,
            loadedTitleCount: null,
            rootValidationState: "notRun",
          });
          if (rootConfigurationPhase === "rootsMissing") {
            setMonitoredTitles([]);
            setCatalogPaginationState({ ...emptyTitleCatalogState });
            commitBootstrapState(rootConfigurationPhase);
            return;
          }

          const nextTitles = await reloadTitles(
            debouncedTitleFilter,
            normalizedSelectedLibraryIds,
            { mode: "initial" },
          );
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }
          if (nextTitles === null) {
            commitBootstrapState("error", t("status.failedToLoad"));
            return;
          }

          let rootValidationState: CatalogRootValidationState = "notRun";
          if (canManageLibrarySettings && nextTitles.length === 0) {
            const configuredRootPaths = [
              ...new Set(
                configuredLibraries.flatMap((library) =>
                  library.roots
                    .map((root) => root.path.trim())
                    .filter((path) => path.length > 0),
                ),
              ),
            ];
            const validation = await validateLibraryRootPaths(
              configuredRootPaths,
              async (path) => {
                const { error } = await client
                  .query(
                    browsePathQuery,
                    { path },
                    { requestPolicy: "network-only" },
                  )
                  .toPromise();
                return error;
              },
            );
            if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
              return;
            }
            rootValidationState = catalogRootValidationState(validation);
            hasConfiguredRoots =
              configuredCatalogLibraries(
                scopedLibraries,
                validation.invalidPaths,
              ).length > 0;
          }

          commitBootstrapState(
            resolveCatalogSurfacePhase({
              canManageLibrarySettings,
              hasConfiguredRoots,
              loadedTitleCount: nextTitles.length,
              rootValidationState,
            }),
          );
        })().catch((error) => {
          console.error("[catalog-bootstrap] failed to resolve catalog:", error);
          commitBootstrapState(
            "error",
            error instanceof Error ? error.message : t("status.failedToLoad"),
          );
        });
        setRoutingInitLoading(false);
        return;
      }

      if (
        catalogSurfaceState.phase === "rootsMissing" ||
        catalogSurfaceState.phase === "rootsInvalid" ||
        catalogSurfaceState.phase === "error"
      ) {
        setRoutingInitLoading(false);
        return;
      }

      if (skipNextCatalogOverviewReloadRef.current) {
        skipNextCatalogOverviewReloadRef.current = false;
      } else if (
        catalogQueryKeyRef.current !==
        titleCatalogQueryKey({
          facet: activeFacet,
          query: debouncedTitleFilter.trim(),
          libraryIds: selectedLibraryIds,
          filters: effectiveTitleQuickFilters,
          advancedFilters: effectiveAdvancedTitleFilters,
          sort: effectiveTitleCatalogSort,
          projection: titleCatalogProjection,
        })
      ) {
        void reloadTitles(debouncedTitleFilter);
      }
      setRoutingInitLoading(false);
      return;
    }
    if (isRoutingSection) {
      let cancelled = false;
      setRoutingInitLoading(true);
      void client
        .query(routingPageInitQuery, { scopeId: activeQualityScopeId })
        .toPromise()
        .then(({ data, error }) => {
          if (cancelled) {
            return;
          }
          if (error) {
            throw error;
          }
          hydrateDownloadClientRouting(
            data?.downloadClientConfigs || [],
            data.downloadClientRouting || [],
          );
          hydrateIndexerRouting(
            data?.indexers || [],
            data.indexerRouting || [],
          );
        })
        .catch((error) => {
          if (cancelled) {
            return;
          }
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.failedToLoad"),
          );
        })
        .finally(() => {
          if (!cancelled) {
            setRoutingInitLoading(false);
          }
        });

      return () => {
        cancelled = true;
      };
    }
    setRoutingInitLoading(false);
    if (isGeneralSettingsSection && canManageConfig) {
      void refreshRuleSets();
    }
  }, [
    activeFacet,
    activeQualityScopeId,
    catalogBootstrapKey,
    catalogSurfaceState.phase,
    canManageConfig,
    canManageLibrarySettings,
    client,
    contentSettingsSection,
    refreshLibraries,
    hydrateDownloadClientRouting,
    hydrateIndexerRouting,
    isMediaView,
    refreshRuleSets,
    debouncedTitleFilter,
    effectiveAdvancedTitleFilters,
    effectiveTitleCatalogSort,
    effectiveTitleQuickFilters,
    reloadTitles,
    selectedLibraryIds,
    setGlobalStatus,
    setMonitoredTitles,
    setSelectedLibraryIds,
    shouldLoadCatalogTitles,
    t,
    titleCatalogProjection,
    view,
  ]);

  const addDiscoveryFacet = addDiscoveryDialogTarget?.facet ?? activeFacet;
  const addDiscoveryResult =
    addDiscoveryDialogTarget?.result ?? EMPTY_SEARCH_RESULT;
  const requestDiscoveryFacet =
    requestDiscoveryDialogTarget?.facet ?? activeFacet;
  const requestDiscoveryResult =
    requestDiscoveryDialogTarget?.result ?? EMPTY_SEARCH_RESULT;
  const handleAddDiscoveryDialogOpenChange = (open: boolean) => {
    if (!open) {
      setAddDiscoveryDialogTarget(null);
    }
  };
  const handleRequestDiscoveryDialogOpenChange = (open: boolean) => {
    if (!open) {
      setRequestDiscoveryDialogTarget(null);
    }
  };

  return (
    <>
      <MediaContentView
        state={{
          view,
          contentSettingsSection,
          canManageConfig,
          canManageSystemSettings,
          canManageCatalogSettings,
          canManageLibrarySettings,
          contentSettingsLabel,
          moviesPath,
          setMoviesPath,
          seriesPath,
          setSeriesPath,
          saveSetting,
          localPathStyle,
          mediaSettingsLoading,
          librarySettingsSaving,
          qualityProfiles: qualityProfiles,
          qualityProfileEntries,
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
          qualityProfileInheritValue: QUALITY_PROFILE_INHERIT_VALUE,
          toProfileOptions,
          handleFacetPersonaSave: saveCategoryScoringPersonaOverride,
          saveCategoryQualityProfileOverride,
          updateCategoryMediaProfileSettings,
          mediaSettingsSaving,
          titleNameForQueue,
          setTitleNameForQueue,
          queueFacet,
          setQueueFacet,
          addTvdbCandidateToCatalog: addTvdbToCatalog,
          monitoredForQueue,
          setMonitoredForQueue,
          seasonFoldersForQueue,
          setSeasonFoldersForQueue,
          minAvailabilityForQueue,
          setMinAvailabilityForQueue,
          tvdbCandidates,
          onAddSubmit,
          titleFilter,
          setTitleFilter,
          refreshTitles,
          titleLoading,
          catalogTotalTitleCount: catalogPaginationState.totalCount,
          catalogManagedBytes: catalogPaginationState.managedBytes,
          catalogHasMoreTitles: catalogPaginationState.hasMore,
          catalogLoadingMoreTitles: catalogPaginationState.loadingMore,
          loadMoreCatalogTitles,
          titleCatalogSortKey: titleCatalogSort.key,
          titleCatalogSortDirection: titleCatalogSort.direction,
          updateTitleCatalogSort,
          visibleTitleTableColumns,
          setTitleTableColumnVisible,
          catalogBootstrapLoading,
          catalogInitialLoadComplete,
          catalogSurfacePhase: catalogSurfaceState.phase,
          catalogSurfaceError: catalogSurfaceState.error,
          retryCatalogBootstrap,
          monitoredTitles: visibleTitles,
          titleContextTitles: titleContextSourceTitles,
          catalogDiscoveryGroups: activeCatalogDiscoveryGroups,
          canViewCatalog,
          canManageTitle,
          canRequestMedia,
          canManageCatalogDiscovery,
          canRequestCatalogDiscovery,
          manageableDiscoveryFacets,
          requestableDiscoveryFacets,
          onCatalogDiscoveryAction: handleCatalogDiscoveryAction,
          titleQuickFilters,
          titleQuickFilterCounts,
          advancedTitleFilters,
          titleCatalogFilterOptions,
          titleCatalogFilterOptionsError,
          retryTitleCatalogFilterOptions: refreshTitleCatalogFilterOptions,
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
          isTogglingTitleMonitoredById: titleMonitoringLoadingById,
          downloadClients,
          activeScopeRouting,
          activeScopeRoutingOrder,
          downloadClientRoutingLoading:
            downloadClientRoutingLoading || routingInitLoading,
          downloadClientRoutingSaving,
          updateDownloadClientRoutingForScope,
          moveDownloadClientInScope,
          indexers,
          activeScopeIndexerRouting,
          activeScopeIndexerRoutingOrder,
          indexerRoutingLoading: indexerRoutingLoading || routingInitLoading,
          indexerRoutingSaving,
          setIndexerEnabledForScope,
          updateIndexerRoutingForScope,
          moveIndexerInScope,
          ruleSets,
          rulesLoading,
          rulesSaving,
          onToggleRuleFacet,
          libraryScanLoading: libraryScanInProgress,
          libraryScanDisabled:
            libraryScanInProgress || effectiveLibraryScanTargetId == null,
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
          allLibrariesValue: ALL_LIBRARIES_VALUE,
          setSelectedLibraryIds,
          loadLibrarySettings,
          loadFacetDownloadClientRouting,
          createLibrary,
          updateLibrary,
          deleteLibrary,
          onOpenOverview,
          onCloseOverview: handleCloseOverview,
          updateMovieTitleOptions,
          refreshMovieTitleOptions,
          selectedOverviewTitleId,
          selectedOverviewTitle: selectedOverviewTitleRecord,
          selectedOverviewDetailLoading,
          routeOverviewPending,
          routeOverviewEpisodeId,
          selectedOverviewBlocklistEntries,
          selectedOverviewExternalSubtitles,
          refreshSelectedOverviewExternalSubtitles,
          deleteSelectedOverviewMediaFile:
            requestDeleteSelectedOverviewMediaFile,
          pendingMediaFileDeletionIds,
          makeSelectedOverviewMovieFilePrimary,
          selectedOverviewPrimaryMovieFileUpdatingId,
          previewTitleRename,
          applyTitleRename,
          setSelectedOverviewTitleId: selectOverviewTitle,
          clearSelectedOverviewTitle,
          scanLibrary: handleLibraryScan,
          deleteCatalogTitle: requestDeleteTitle,
          isDeletingCatalogTitleById: deleteTitleLoadingById,
          isMobile,
          viewMode: effectiveViewMode,
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
        }}
      />
      {canManageTitle ? (
        <AddToCatalogDialog
          open={addDiscoveryDialogTarget !== null}
          onOpenChange={handleAddDiscoveryDialogOpenChange}
          result={addDiscoveryResult}
          facet={addDiscoveryFacet}
          catalogQualityProfileOptions={catalogQualityProfileOptions}
          catalogConfigLoading={catalogConfigLoading}
          defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(
            addDiscoveryFacet,
          )}
          manageableLibraries={librariesByFacet[addDiscoveryFacet] ?? []}
          rootFolderOptions={rootFoldersByFacet[addDiscoveryFacet] ?? []}
          onAdd={(result, facet, options) =>
            // The catalog reload rides the catalog-titles event, same as an add
            // made from global search.
            addMetadataSearchResultToCatalog(result, facet, options)
          }
        />
      ) : null}
      {canRequestMedia ? (
        <RequestMediaDialog
          open={requestDiscoveryDialogTarget !== null}
          onOpenChange={handleRequestDiscoveryDialogOpenChange}
          result={requestDiscoveryResult}
          facet={requestDiscoveryFacet}
          requestableLibraries={
            requestableLibrariesByFacet[requestDiscoveryFacet] ?? []
          }
          qualityProfileOptions={catalogQualityProfileOptions}
          onRequest={async (result, facet, options) => {
            const accepted = await requestMetadataSearchResult(
              result,
              facet,
              options,
            );
            if (accepted) {
              await Promise.all([
                refreshTitles(),
                refreshCatalogDiscovery(),
              ]);
            }
            return accepted;
          }}
        />
      ) : null}
      <BulkTitleEditDialog
        open={bulkEditDialogOpen}
        onOpenChange={setBulkEditDialogOpen}
        view={view}
        selectedTitles={editDialogTitles}
        qualityProfiles={qualityProfiles}
        rootFolders={editDialogRootFolders}
        busy={bulkActionBusy}
        onSubmit={applyBulkTitleOptions}
      />
      <ConfirmDialog
        open={bulkDeleteDialogOpen}
        title={t("title.bulkDeleteTitle")}
        description={t("title.bulkDeleteDescription", {
          count: selectedTitles.length,
        })}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={bulkActionBusy}
        confirmDisabled={bulkDeleteConfirmDisabled}
        onConfirm={confirmBulkDeleteTitles}
        onCancel={closeBulkDeleteDialog}
      >
        <div className="space-y-3">
          <label className="flex items-center gap-2">
            <Checkbox
              checked={bulkDeleteFilesOnDisk}
              onCheckedChange={(checked) =>
                setBulkDeleteFilesOnDisk(checked === true)
              }
              disabled={bulkActionBusy}
            />
            <span className="text-xs text-card-foreground">
              {t("title.deleteFilesOnDisk")}
            </span>
          </label>
          {bulkDeleteFilesOnDisk ? (
            <DeletePreviewSummary
              preview={bulkDeletePreview}
              loading={bulkDeletePreviewLoading}
              error={bulkDeletePreviewError}
              typedConfirmation={bulkDeleteTypedConfirmation}
              onTypedConfirmationChange={setBulkDeleteTypedConfirmation}
            />
          ) : null}
        </div>
      </ConfirmDialog>
      <ConfirmDialog
        open={bulkRenameDialogOpen}
        title={t("title.bulkRenameTitle")}
        description={t("title.bulkRenameDescription", {
          count: selectedTitles.length,
        })}
        confirmLabel={t("rename.applyButton")}
        cancelLabel={t("label.cancel")}
        contentClassName="max-w-4xl"
        confirmButtonVariant="primary"
        confirmButtonId="bulk-rename-apply"
        isBusy={bulkActionBusy}
        confirmDisabled={bulkRenameConfirmDisabled}
        onConfirm={confirmBulkRenameTitles}
        onCancel={closeBulkRenameDialog}
      >
        <BulkRenamePreviewSummary
          titles={selectedTitles}
          plansByTitleId={bulkRenamePlansByTitleId}
          summary={bulkRenameSummary}
          loading={bulkRenamePreviewLoading}
          error={bulkRenamePreviewError}
        />
      </ConfirmDialog>
      <ConfirmDialog
        open={titleToDelete !== null}
        title={t("label.delete")}
        description={
          titleToDelete
            ? t("status.deleteCatalogConfirm", { name: titleToDelete.name })
            : t("label.delete")
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={
          titleToDelete !== null
            ? !!deleteTitleLoadingById[titleToDelete.id]
            : false
        }
        confirmDisabled={deleteTitleConfirmDisabled}
        onConfirm={confirmDeleteTitle}
        onCancel={closeDeleteTitleDialog}
      >
        <div className="space-y-3">
          <label className="flex items-center gap-2">
            <Checkbox
              checked={deleteFilesOnDisk}
              onCheckedChange={(checked) =>
                setDeleteFilesOnDisk(checked === true)
              }
              disabled={
                titleToDelete !== null
                  ? !!deleteTitleLoadingById[titleToDelete.id]
                  : false
              }
            />
            <span className="text-xs text-card-foreground">
              {t("title.deleteFilesOnDisk")}
            </span>
          </label>
          {deleteFilesOnDisk ? (
            <DeletePreviewSummary
              preview={titleDeletePreview}
              loading={titleDeletePreviewLoading}
              error={titleDeletePreviewError}
              typedConfirmation={titleDeleteTypedConfirmation}
              onTypedConfirmationChange={setTitleDeleteTypedConfirmation}
            />
          ) : null}
        </div>
      </ConfirmDialog>
      <ConfirmDialog
        open={selectedOverviewMediaFileToDelete !== null}
        title={t("mediaFile.delete")}
        description={
          selectedOverviewMediaFileToDelete?.file.filePath ??
          t("mediaFile.delete")
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={selectedOverviewMediaFileDeleteLoading}
        confirmDisabled={deleteSelectedOverviewMediaFileConfirmDisabled}
        onConfirm={confirmDeleteSelectedOverviewMediaFile}
        onCancel={closeSelectedOverviewMediaFileDeleteDialog}
      >
        <DeletePreviewSummary
          preview={selectedOverviewMediaFileDeletePreview}
          loading={selectedOverviewMediaFileDeletePreviewLoading}
          error={selectedOverviewMediaFileDeletePreviewError}
          typedConfirmation={selectedOverviewMediaFileDeleteTypedConfirmation}
          onTypedConfirmationChange={
            setSelectedOverviewMediaFileDeleteTypedConfirmation
          }
        />
      </ConfirmDialog>
      {replaceConflictDialog}
    </>
  );
});
