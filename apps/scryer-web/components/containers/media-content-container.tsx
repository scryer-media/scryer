import * as React from "react";
import { RequestsContainer } from "@/components/containers/requests-container";
import { MediaContentView } from "@/components/views/media-content-view";
import {
  addTitleMutation,
  buildSetTitleMonitoredBatchMutation,
  buildUpdateTitleBatchMutation,
  createLibraryMutation,
  deleteLibraryMutation,
  queueBestReleaseMutation,
  queueExistingMutation,
  scanLibraryMutation,
  deleteTitlesMutation,
  setTitleMonitoredMutation,
  updateLibraryMutation,
  updateRuleSetMutation,
} from "@/lib/graphql/mutations";
import {
  browsePathQuery,
  deleteTitlePreviewQuery,
  deleteTitlesPreviewQuery,
  downloadClientRoutingQuery,
  jobRunEventsSubscription,
  jobRunsQuery,
  librariesQuery,
  libraryDownloadClientsQuery,
  librarySettingsQuery,
  ruleSetsQuery,
  routingPageInitQuery,
  searchForTitleQuery,
  titlesQuery,
} from "@/lib/graphql/queries";
import {
  CATEGORY_SCOPE_MAP,
  QUALITY_PROFILE_INHERIT_VALUE,
  viewToFacet,
} from "@/lib/constants/settings";
import { useClient } from "urql";
import type { ContentSettingsSection, OverviewTitleTarget, ViewId } from "@/components/root/types";
import {
  toProfileOptions,
} from "@/lib/utils/quality-profiles";
import {
  normalizeLibraryFilterSelection,
  selectedLibraryIdsToQueryValue,
  singleSelectedLibraryId,
} from "@/lib/utils/library-filter";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";
import { useDownloadClientRouting } from "@/lib/hooks/use-download-client-routing";
import { useIndexerRouting } from "@/lib/hooks/use-indexer-routing";
import { useMediaSettings } from "@/lib/hooks/use-media-settings";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { useQueueFormState } from "@/lib/hooks/use-queue-form-state";
import { useTitleManagementState } from "@/lib/hooks/use-title-management-state";
import type {
  DownloadClientRecord,
  DownloadClientRoutingEntry,
  JobRun,
  LibraryRecord,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  Release,
  RootFolderOption,
  TitleRecord,
  RuleSetRecord,
} from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { DeletePreview, DeleteTitlesPreview } from "@/lib/types/delete-preview";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
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
import { useTitleListReactiveRefresh } from "@/lib/hooks/use-title-list-reactive-refresh";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import type { TitleOptionUpdates } from "@/lib/types/title-options";
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
  type TitleQuickFilters,
} from "@/components/views/media-content/title-quick-filters";
import {
  defaultSortDirectionForTitleKey,
  type TitleTableSortDirection,
  type TitleTableSortKey,
} from "@/components/views/media-content/title-table-shared";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";

const HYDRATION_POSTER_REFRESH_WINDOW_MS = 5 * 60 * 1000;
const HYDRATION_POSTER_REFRESH_INTERVAL_MS = 2_500;
const TITLE_DELETION_JOB_FALLBACK_DELAYS_MS = [10_000, 60_000, 180_000] as const;
const TITLE_CATALOG_PAGE_SIZE = 300;
const TITLE_CATALOG_PREFETCH_DISTANCE_PX = 1200;
const ALL_LIBRARIES_VALUE = "__all__";

type MediaContentContainerProps = {
  view: ViewId;
  contentSettingsSection: ContentSettingsSection;
  canManageConfig: boolean;
  canManageSystemSettings: boolean;
  canManageCatalogSettings: boolean;
  canManageLibrarySettings: boolean;
  onOpenOverview: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
};

type TitleCatalogState = {
  queryKey: string;
  hasMore: boolean;
  nextOffset: number;
  totalCount: number;
  loadingMore: boolean;
};

type TitleCatalogSortState = {
  key: TitleTableSortKey;
  direction: TitleTableSortDirection;
};

const emptyTitleCatalogState: TitleCatalogState = {
  queryKey: "",
  hasMore: false,
  nextOffset: 0,
  totalCount: 0,
  loadingMore: false,
};

const defaultTitleCatalogSortState: TitleCatalogSortState = {
  key: "name",
  direction: "asc",
};

function titleCatalogSortInput(sort: TitleCatalogSortState) {
  const key =
    sort.key === "name"
      ? "title"
      : sort.key === "monitored"
        ? "monitored"
        : sort.key === "quality"
          ? "quality"
          : sort.key === "episodes"
            ? "episodes"
            : sort.key === "status"
              ? "status"
              : "size";

  return {
    key,
    direction: sort.direction,
  };
}

type ActiveCatalogListFilters = {
  facet: TitleRecord["facet"];
  query: string;
  libraryIds: readonly string[];
};

function sortCatalogTitles(titles: TitleRecord[]): TitleRecord[] {
  return [...titles].sort((left, right) => {
    const nameCompare = left.name.toLocaleLowerCase().localeCompare(
      right.name.toLocaleLowerCase(),
    );
    if (nameCompare !== 0) {
      return nameCompare;
    }
    return left.id.localeCompare(right.id);
  });
}

function mergePreferLoadedImageFields(
  current: TitleRecord,
  incoming: TitleRecord,
): TitleRecord {
  const incomingHasPoster = Boolean(incoming.posterUrl || incoming.posterSourceUrl);
  const incomingHasBackground = Boolean(
    incoming.backgroundUrl || incoming.backgroundSourceUrl,
  );

  return {
    ...incoming,
    posterUrl: incomingHasPoster ? incoming.posterUrl : (current.posterUrl ?? null),
    posterSourceUrl: incomingHasPoster
      ? incoming.posterSourceUrl
      : (current.posterSourceUrl ?? null),
    backgroundUrl: incomingHasBackground
      ? incoming.backgroundUrl
      : (current.backgroundUrl ?? null),
    backgroundSourceUrl: incomingHasBackground
      ? incoming.backgroundSourceUrl
      : (current.backgroundSourceUrl ?? null),
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
    return current ? mergePreferLoadedImageFields(current, title) : title;
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
      currentById.set(title.id, merged);
      const index = next.findIndex((candidate) => candidate.id === title.id);
      if (index !== -1) {
        next[index] = merged;
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
    next[existingIndex] = mergePreferLoadedImageFields(next[existingIndex], title);
  }
  return sortCatalogTitles(next);
}

function isPendingHydrationPosterTitle(title: TitleRecord, nowMs: number): boolean {
  if (title.posterUrl || title.posterSourceUrl || title.metadataFetchedAt != null) {
    return false;
  }

  const createdAtMs = title.createdAt ? Date.parse(title.createdAt) : Number.NaN;
  if (!Number.isFinite(createdAtMs)) {
    return true;
  }

  return nowMs - createdAtMs <= HYDRATION_POSTER_REFRESH_WINDOW_MS;
}

function sameIdSet(left: ReadonlySet<string>, right: ReadonlySet<string>): boolean {
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

function titleCatalogFilterInput(filters: TitleQuickFilters) {
  const monitored =
    filters.monitored === filters.unmonitored
      ? null
      : filters.monitored
        ? true
        : false;
  const contentStatuses = [
    filters.continuing ? "continuing" : null,
    filters.ended ? "ended" : null,
  ].filter((value): value is string => Boolean(value));

  if (monitored === null && contentStatuses.length === 0) {
    return null;
  }

  return {
    monitored,
    contentStatuses,
  };
}

function titleCatalogQueryKey({
  facet,
  query,
  libraryIds,
  filters,
  sort,
}: {
  facet: ViewCategoryId;
  query: string;
  libraryIds: string[];
  filters: TitleQuickFilters;
  sort: TitleCatalogSortState;
}) {
  return JSON.stringify({
    facet,
    query: query.trim(),
    libraryIds: [...libraryIds].sort(),
    filter: titleCatalogFilterInput(filters),
    sort: titleCatalogSortInput(sort),
  });
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
  const refreshedById = new Map(refreshedTitles.map((title) => [title.id, title]));
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
  const refreshedById = new Map(refreshedTitles.map((title) => [title.id, title]));
  return splitSucceededTitleIds(targets, (title) => {
    const refreshed = refreshedById.get(title.id);
    if (!refreshed) {
      return false;
    }

    if (
      changes.qualityProfileId !== undefined &&
      (refreshed.qualityProfileId ?? "") !== changes.qualityProfileId
    ) {
      return false;
    }
    if (
      changes.rootFolderId !== undefined &&
      (refreshed.rootFolderId ?? null) !== changes.rootFolderId
    ) {
      return false;
    }
    if (
      changes.monitorType !== undefined &&
      (refreshed.monitorType ?? "") !== changes.monitorType
    ) {
      return false;
    }
    if (
      changes.useSeasonFolders !== undefined &&
      refreshed.useSeasonFolders !== changes.useSeasonFolders
    ) {
      return false;
    }
    if (
      changes.monitorSpecials !== undefined &&
      refreshed.monitorSpecials !== changes.monitorSpecials
    ) {
      return false;
    }
    if (
      changes.interSeasonMovies !== undefined &&
      refreshed.interSeasonMovies !== changes.interSeasonMovies
    ) {
      return false;
    }
    if (
      changes.fillerPolicy !== undefined &&
      (refreshed.fillerPolicy ?? "") !== changes.fillerPolicy
    ) {
      return false;
    }
    if (
      changes.recapPolicy !== undefined &&
      (refreshed.recapPolicy ?? "") !== changes.recapPolicy
    ) {
      return false;
    }

    return true;
  });
}

function aggregateDeletePreviews(previews: DeletePreview[]): DeletePreview | null {
  if (previews.length === 0) {
    return null;
  }

  const samplePaths = Array.from(
    new Set(previews.flatMap((preview) => preview.samplePaths)),
  ).slice(0, 12);
  const typedPrompt =
    previews.find((preview) => preview.requiresTypedConfirmation)
      ?.typedConfirmationPrompt ?? null;
  const mediaCount = previews.reduce((sum, preview) => sum + preview.mediaCount, 0);
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
      typedPrompt ?? (requiresTypedConfirmation ? "Type DELETE to confirm this large delete." : null),
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
  onOpenOverview,
}: MediaContentContainerProps) {
  const searchState = useSearchContext();
  const {
    queueFacet,
    setQueueFacet,
    runTvdbSearch,
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
  const [pendingDeletedTitleIds, setPendingDeletedTitleIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  const deletionJobIdsRef = React.useRef(new Set<string>());
  const deletionFallbackTimersRef = React.useRef<ReturnType<typeof setTimeout>[]>(
    [],
  );
  const [startedLibraryScanSessionId, setStartedLibraryScanSessionId] =
    React.useState<string | null>(null);
  const activeFacet = viewToFacet[view as keyof typeof viewToFacet] ?? "movie";
  const { getActiveSession, getSessionById, refreshSessions: refreshLibraryScanSessions } =
    useLibraryScanProgress();
  const activeLibraryScanSession = getActiveSession(activeFacet);
  const startedLibraryScanSession = startedLibraryScanSessionId
    ? getSessionById(startedLibraryScanSessionId)
    : null;
  const isMobile = useIsMobile();
  const activeQualityScopeId =
    CATEGORY_SCOPE_MAP[view as keyof typeof CATEGORY_SCOPE_MAP] ?? "movie";
  const isMediaView =
    view === "movies" || view === "series" || view === "anime";
  const shouldLoadCatalogTitles =
    isMediaView && contentSettingsSection === "overview";
  const shouldLoadMediaSettings = isMediaView;
  const [desktopViewMode, setDesktopViewMode] = React.useState<ContentViewMode>(
    () => readStoredContentViewMode(),
  );
  const effectiveViewMode: ContentViewMode = isMobile
    ? "poster"
    : desktopViewMode;
  const [selectedTitleIds, setSelectedTitleIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [titleQuickFilters, setTitleQuickFilters] =
    React.useState<TitleQuickFilters>({
      monitored: false,
      unmonitored: false,
      continuing: false,
      ended: false,
    });
  const [titleCatalogSort, setTitleCatalogSort] =
    React.useState<TitleCatalogSortState>(defaultTitleCatalogSortState);
  const effectiveTitleCatalogSort =
    effectiveViewMode === "poster"
      ? defaultTitleCatalogSortState
      : titleCatalogSort;
  const [bulkActionBusy, setBulkActionBusy] = React.useState(false);
  const [bulkEditDialogOpen, setBulkEditDialogOpen] = React.useState(false);
  const [bulkDeleteDialogOpen, setBulkDeleteDialogOpen] = React.useState(false);
  const [bulkDeleteFilesOnDisk, setBulkDeleteFilesOnDisk] =
    React.useState(false);
  const [bulkDeleteTypedConfirmation, setBulkDeleteTypedConfirmation] =
    React.useState("");
  const [bulkDeletePreviewLoading, setBulkDeletePreviewLoading] =
    React.useState(false);
  const [bulkDeletePreviewError, setBulkDeletePreviewError] = React.useState<
    string | null
  >(null);
  const [bulkDeletePreviewsByTitleId, setBulkDeletePreviewsByTitleId] =
    React.useState<Record<string, DeletePreview>>({});
  const [debouncedTitleFilter, setDebouncedTitleFilter] = React.useState("");
  const [libraries, setLibraries] = React.useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = React.useState(false);
  const [libraryDownloadClients, setLibraryDownloadClients] = React.useState<
    DownloadClientRecord[]
  >([]);
  const [libraryDownloadClientsLoading, setLibraryDownloadClientsLoading] =
    React.useState(false);
  const [catalogBootstrapState, setCatalogBootstrapState] = React.useState({
    facet: activeFacet,
    loading: false,
    initialLoadComplete: false,
  });
  const [rootValidationLibraries, setRootValidationLibraries] = React.useState<LibraryRecord[]>([]);
  const [rootValidationLibrariesLoading, setRootValidationLibrariesLoading] = React.useState(false);
  const [invalidRootLibraryIds, setInvalidRootLibraryIds] = React.useState<string[]>([]);
  const [librarySettingsSaving, setLibrarySettingsSaving] = React.useState(false);
  const [selectedLibraryIds, setSelectedLibraryIds] = React.useState<string[]>([]);
  const activeCatalogQueryRef = React.useRef("");
  const activeCatalogListFiltersRef = React.useRef<ActiveCatalogListFilters>({
    facet: activeFacet,
    query: "",
    libraryIds: [],
  });
  const catalogTitleRequestSeqRef = React.useRef(0);
  const catalogBootstrapRequestSeqRef = React.useRef(0);
  const catalogPageLoadInFlightRef = React.useRef(false);
  const catalogQueryKeyRef = React.useRef("");
  const latestCriticalMutationEpochRef = React.useRef(0);
  const skipNextCatalogOverviewReloadRef = React.useRef(false);
  const [catalogPaginationState, setCatalogPaginationState] = React.useState<TitleCatalogState>(
    emptyTitleCatalogState,
  );

  React.useEffect(() => {
    catalogQueryKeyRef.current = catalogPaginationState.queryKey;
  }, [catalogPaginationState.queryKey]);

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
    libraryScanLoading,
    setLibraryScanLoading,
    libraryScanSummary,
    setLibraryScanSummary,
  } = useTitleManagementState();
  const libraryScanInProgress =
    libraryScanLoading ||
    Boolean(activeLibraryScanSession) ||
    Boolean(startedLibraryScanSessionId && !startedLibraryScanSession);
  const catalogInitialLoadComplete =
    shouldLoadCatalogTitles &&
    catalogBootstrapState.facet === activeFacet &&
    catalogBootstrapState.initialLoadComplete;
  const catalogBootstrapInFlight =
    catalogBootstrapState.facet === activeFacet &&
    catalogBootstrapState.loading;
  const catalogBootstrapLoading =
    shouldLoadCatalogTitles &&
    !catalogInitialLoadComplete;
  const titleDeletePreviewVariables = React.useMemo(
    () =>
      titleToDelete && deleteFilesOnDisk
        ? { titleId: titleToDelete.id }
        : null,
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
  const effectiveTitleQuickFilters = React.useMemo<TitleQuickFilters>(
    () => ({
      ...titleQuickFilters,
      continuing: activeFacet === "movie" ? false : titleQuickFilters.continuing,
      ended: activeFacet === "movie" ? false : titleQuickFilters.ended,
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
  const monitoredTitlesWithLibraries = React.useMemo(
    () =>
      monitoredTitles.map((title) => ({
        ...title,
        libraryName:
          title.libraryName ?? libraryNameById.get(title.libraryId) ?? title.libraryId,
        librarySlug:
          title.librarySlug ?? librarySlugById.get(title.libraryId) ?? null,
      })),
    [libraryNameById, librarySlugById, monitoredTitles],
  );
  const visibleTitles = React.useMemo(
    () =>
      filterTitlesByQuickFilters(
        monitoredTitlesWithLibraries.filter(
          (title) => !pendingDeletedTitleIds.has(title.id),
        ),
        effectiveTitleQuickFilters,
      ),
    [effectiveTitleQuickFilters, monitoredTitlesWithLibraries, pendingDeletedTitleIds],
  );
  const selectedTitles = React.useMemo(
    () => visibleTitles.filter((title) => selectedTitleIds.has(title.id)),
    [selectedTitleIds, visibleTitles],
  );
  const selectedTitleLibraryIds = React.useMemo(
    () => Array.from(new Set(selectedTitles.map((title) => title.libraryId))),
    [selectedTitles],
  );
  const selectedTitleLibrary = React.useMemo(
    () =>
      selectedTitleLibraryIds.length === 1
        ? libraries.find((library) => library.id === selectedTitleLibraryIds[0]) ?? null
        : null,
    [libraries, selectedTitleLibraryIds],
  );
  const bulkRootFolders = React.useMemo(
    () => selectedTitleLibrary?.roots ?? [],
    [selectedTitleLibrary],
  );

  useOverviewWindowScrollRestoration({
    enabled: shouldLoadCatalogTitles,
    ready: !titleLoading && visibleTitles.length > 0,
    storageKeySuffix: "window",
  });

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
    setTitleQuickFilters({
      monitored: false,
      unmonitored: false,
      continuing: false,
      ended: false,
    });
    setSelectedTitleIds(new Set());
    setSelectedLibraryIds([]);
  }, [activeFacet]);

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
    activeCatalogQueryRef.current = debouncedTitleFilter;
  }, [debouncedTitleFilter]);

  React.useEffect(() => {
    if (isMobile) {
      return;
    }
    writeStoredContentViewMode(desktopViewMode);
  }, [desktopViewMode, isMobile]);

  React.useEffect(() => {
    if (
      effectiveViewMode === "compact" &&
      shouldLoadCatalogTitles &&
      contentSettingsSection === "overview"
    ) {
      return;
    }
    setSelectedTitleIds((current) => (current.size === 0 ? current : new Set()));
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
  }, [selectedTitles.length]);

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
    globalQualityProfileId,
    globalScoringPersona,
    categoryQualityProfileOverrides,
    categoryRequiredAudioLanguages,
    saveCategoryRequiredAudioLanguages,
    categoryPersonaSelections,
    categoryFolderTemplates,
    setCategoryFolderTemplates,
    categorySeasonFolderTemplates,
    setCategorySeasonFolderTemplates,
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
    activeFacet === "movie"
      ? t("nav.movies")
      : activeFacet === "series"
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
  const [libraryScanNotice, setLibraryScanNotice] = React.useState<
    string | null
  >(null);
  const [titleMonitoringLoadingById, setTitleMonitoringLoadingById] =
    React.useState<Record<string, boolean>>({});

  React.useEffect(() => {
    if (!activeLibraryScanSession) {
      setLibraryScanNotice(null);
    }
  }, [activeLibraryScanSession]);

  React.useEffect(() => {
    if (!startedLibraryScanSessionId) {
      return;
    }

    const session = getSessionById(startedLibraryScanSessionId);
    if (!session) {
      return;
    }

    if (
      session.status !== "completed" &&
      session.status !== "warning" &&
      session.status !== "failed"
    ) {
      return;
    }

    if (session.summary) {
      setLibraryScanSummary(session.summary);
    }

    setStartedLibraryScanSessionId(null);
  }, [getSessionById, setLibraryScanSummary, startedLibraryScanSessionId]);

  React.useEffect(() => {
    if (!startedLibraryScanSessionId || startedLibraryScanSession) {
      return;
    }

    let cancelled = false;
    const retryDelaysMs = [0, 400, 1_200];
    const timers = retryDelaysMs.map((delayMs) =>
      window.setTimeout(() => {
        if (cancelled) {
          return;
        }
        void refreshLibraryScanSessions().catch((error) => {
          console.error(
            "[library-scan] failed to reconcile started scan session:",
            error,
          );
        });
      }, delayMs),
    );
    const releaseTimer = window.setTimeout(() => {
      if (!cancelled) {
        setStartedLibraryScanSessionId(null);
      }
    }, 4_000);

    return () => {
      cancelled = true;
      timers.forEach((timer) => window.clearTimeout(timer));
      window.clearTimeout(releaseTimer);
    };
  }, [
    refreshLibraryScanSessions,
    startedLibraryScanSession,
    startedLibraryScanSessionId,
  ]);

  React.useEffect(() => {
    setLibraryScanNotice(null);
  }, [activeFacet]);

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
    ): Promise<TitleRecord[] | null> => {
      setTitleLoading(true);
      setTitleStatus(t("title.loading"));
      const query = (queryOverride ?? activeCatalogQueryRef.current).trim();
      const libraryIds = libraryIdsOverride ?? selectedLibraryIds;
      const filter = titleCatalogFilterInput(effectiveTitleQuickFilters);
      const sort = titleCatalogSortInput(effectiveTitleCatalogSort);
      const queryKey = titleCatalogQueryKey({
        facet: activeFacet,
        query,
        libraryIds,
        filters: effectiveTitleQuickFilters,
        sort: effectiveTitleCatalogSort,
      });
      activeCatalogListFiltersRef.current = buildActiveCatalogListFilters(
        activeFacet,
        query,
        libraryIds,
      );
      const requestSeq = ++catalogTitleRequestSeqRef.current;
      catalogPageLoadInFlightRef.current = false;
      catalogQueryKeyRef.current = queryKey;
      setCatalogPaginationState({ ...emptyTitleCatalogState, queryKey });

      try {
        const { data, error } = await client
          .query(
            titlesQuery,
            {
              facet: activeFacet,
              libraryIds: selectedLibraryIdsToQueryValue(libraryIds),
              query: query || null,
              filter,
              sort,
              limit: TITLE_CATALOG_PAGE_SIZE,
              offset: 0,
            },
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
          return null;
        }

        const page = data?.titles ?? {};
        const nextTitles = (page.items ?? []) as TitleRecord[];
        setMonitoredTitles((current) =>
          mergeCatalogTitlesPreservingImages(current, nextTitles),
        );
        setCatalogPaginationState({
          queryKey,
          hasMore: Boolean(page.hasMore),
          nextOffset:
            typeof page.offset === "number" && typeof page.limit === "number"
              ? page.offset + nextTitles.length
              : nextTitles.length,
          totalCount:
            typeof page.totalCount === "number" ? page.totalCount : nextTitles.length,
          loadingMore: false,
        });
        setTitleStatus(
          t("title.statusTemplate", {
            count:
              typeof page.totalCount === "number"
                ? page.totalCount
                : nextTitles.length,
          }),
        );
        return nextTitles;
      } catch (error) {
        if (requestSeq !== catalogTitleRequestSeqRef.current) {
          return null;
        }
        setTitleStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
        return null;
      } finally {
        if (requestSeq === catalogTitleRequestSeqRef.current) {
          setTitleLoading(false);
        }
      }
    },
    [
      activeFacet,
      client,
      effectiveTitleQuickFilters,
      effectiveTitleCatalogSort,
      selectedLibraryIds,
      setMonitoredTitles,
      setTitleLoading,
      setTitleStatus,
      t,
    ],
  );

  const refreshTitles = React.useCallback(async (query?: string) => {
    await reloadTitles(query ?? titleFilter);
  }, [reloadTitles, titleFilter]);

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
    const filter = titleCatalogFilterInput(effectiveTitleQuickFilters);
    const sort = titleCatalogSortInput(effectiveTitleCatalogSort);
    const queryKey = titleCatalogQueryKey({
      facet: activeFacet,
      query,
      libraryIds: selectedLibraryIds,
      filters: effectiveTitleQuickFilters,
      sort: effectiveTitleCatalogSort,
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
          titlesQuery,
          {
            facet: activeFacet,
            libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
            query: query || null,
            filter,
            sort,
            limit: TITLE_CATALOG_PAGE_SIZE,
            offset,
          },
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
      setMonitoredTitles((current) =>
        appendCatalogTitlesPreservingImages(current, nextTitles),
      );
      setCatalogPaginationState({
        queryKey,
        hasMore: Boolean(page.hasMore),
        nextOffset:
          typeof page.offset === "number" ? page.offset + nextTitles.length : offset + nextTitles.length,
        totalCount:
          typeof page.totalCount === "number" ? page.totalCount : catalogPaginationState.totalCount,
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
        setCatalogPaginationState((current) => ({ ...current, loadingMore: false }));
      }
    }
  }, [
    activeFacet,
    catalogPaginationState.hasMore,
    catalogPaginationState.loadingMore,
    catalogPaginationState.nextOffset,
    catalogPaginationState.queryKey,
    catalogPaginationState.totalCount,
    client,
    effectiveTitleCatalogSort,
    effectiveTitleQuickFilters,
    selectedLibraryIds,
    setMonitoredTitles,
    setTitleStatus,
    shouldLoadCatalogTitles,
    t,
  ]);

  const recordCriticalCatalogMutation = React.useCallback(() => {
    latestCriticalMutationEpochRef.current = reactiveRefreshEpoch();
  }, []);

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles || effectiveViewMode !== "poster") {
      return;
    }

    const maybeLoadNextPage = () => {
      const scrollElement = document.documentElement;
      const remaining =
        scrollElement.scrollHeight - (window.scrollY + window.innerHeight);
      if (remaining <= TITLE_CATALOG_PREFETCH_DISTANCE_PX) {
        void loadMoreCatalogTitles();
      }
    };

    maybeLoadNextPage();
    window.addEventListener("scroll", maybeLoadNextPage, { passive: true });
    window.addEventListener("resize", maybeLoadNextPage);
    return () => {
      window.removeEventListener("scroll", maybeLoadNextPage);
      window.removeEventListener("resize", maybeLoadNextPage);
    };
  }, [effectiveViewMode, loadMoreCatalogTitles, shouldLoadCatalogTitles]);

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
        run.jobKey !== "title_deletion" ||
        !deletionJobIdsRef.current.has(run.id) ||
        !isTerminalJobRunStatus(run.status)
      ) {
        return false;
      }

      deletionJobIdsRef.current.delete(run.id);
      if (deletionJobIdsRef.current.size === 0) {
        clearDeletionFallbackTimers();
      }
      setPendingDeletedTitleIds(new Set());
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
        .query<{ jobRuns?: unknown[] }>(
          jobRunsQuery,
          { jobKey: "title_deletion", limit: 10 },
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
    deletionFallbackTimersRef.current = TITLE_DELETION_JOB_FALLBACK_DELAYS_MS.map(
      (delayMs) =>
        setTimeout(() => {
          void refreshTrackedDeletionJobs();
        }, delayMs),
    );
  }, [clearDeletionFallbackTimers, refreshTrackedDeletionJobs]);

  React.useEffect(() => clearDeletionFallbackTimers, [clearDeletionFallbackTimers]);

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

  useDeferredWsSubscription<{ data?: { jobRunEvents?: unknown } }>({
    requestKey: "mediaContentTitleDeletionJobRuns",
    request: { query: jobRunEventsSubscription },
    onNext(result) {
      handleTitleDeletionJobSnapshot(normalizeJobRun(result.data?.jobRunEvents));
    },
    onError(error) {
      console.error("[title-deletion-job-runs] subscription error:", error);
    },
  });

  const applyRefreshedTitleRecord = React.useCallback(
    (titleId: string, title: TitleRecord | null, requestEpoch: number) => {
      if (requestEpoch <= latestCriticalMutationEpochRef.current) {
        return;
      }

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

        if (!catalogTitleMatchesActiveListFilters(
          title,
          activeCatalogListFiltersRef.current,
        )) {
          if (existingIndex === -1) {
            return current;
          }
          next.splice(existingIndex, 1);
          setTitleStatus(t("title.statusTemplate", { count: next.length }));
          return next;
        }

        if (existingIndex === -1) {
          next.push(title);
        } else {
          next[existingIndex] = mergePreferLoadedImageFields(
            next[existingIndex],
            title,
          );
        }
        const sorted = sortCatalogTitles(next);
        setTitleStatus(t("title.statusTemplate", { count: next.length }));
        return sorted;
      });
    },
    [setMonitoredTitles, setTitleStatus, t],
  );

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

  useTitleListReactiveRefresh({
    facet: activeFacet,
    pause: !shouldLoadCatalogTitles,
    onTitleRefreshed: applyRefreshedTitleRecord,
  });

  React.useEffect(() => {
    if (!shouldLoadCatalogTitles || pendingHydrationPosterTitleIds.length === 0) {
      return;
    }

    const refreshPendingHydrationPosters = () => {
      pendingHydrationPosterTitleIds.forEach((titleId) => {
        queueCatalogTitleRefresh({
          titleId,
          apply(title, requestEpoch) {
            applyRefreshedTitleRecord(titleId, title, requestEpoch);
          },
          onError(error) {
            console.error("[catalog-hydration-poster-refresh] refresh failed:", error);
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
      const imdbId = candidate.imdbId?.trim();
      const externalIds = [
        { source: "tvdb", value: tvdbId },
        ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
      ];

      const monitorType = monitoredForQueue ? "allEpisodes" : "none";
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
                ...(queueFacet === "movie"
                  ? {}
                  : { useSeasonFolders: seasonFoldersForQueue }),
                ...(queueFacet === "anime"
                  ? {
                      monitorSpecials: false,
                      interSeasonMovies: true,
                    }
                  : {}),
              },
              externalIds,
              ...(queueFacet === "movie"
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
        assertNoReplaceConflict(payload, "A download is already in progress for this title.");
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  const runInteractiveSearchForTitle = React.useCallback(
    async (title: TitleRecord) => {
      try {
        const { data, error } = await client
          .query(searchForTitleQuery, { titleId: title.id })
          .toPromise();
        if (error) throw error;
        return (data?.searchReleases ?? []) as Release[];
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.searchFailed"),
        );
        return [];
      }
    },
    [client, setGlobalStatus, t],
  );

  const queueExistingFromRelease = React.useCallback(
    async (title: TitleRecord, release: Release) => {
      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }

      try {
        const input = {
          titleId: title.id,
          scope: releaseQueueScopeInput(release, { title: true }),
          candidateToken: release.candidateToken,
        };
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client
              .mutation(queueExistingMutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return data?.queueExistingTitleDownload;
          },
          "A download is already in progress for this title.",
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, "A download is already in progress for this title.");
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  const queueAdditionalFromRelease = React.useCallback(
    async (title: TitleRecord, release: Release) => {
      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }

      try {
        const { data, error } = await client
          .mutation(queueExistingMutation, {
            input: {
              titleId: title.id,
              scope: releaseQueueScopeInput(release, { title: true }),
              candidateToken: release.candidateToken,
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
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
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
          [nextFilter]: !current[nextFilter],
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
          [nextFilter]: !current[nextFilter],
        }));
      });
    },
    [],
  );

  const clearTitleQuickFilters = React.useCallback(() => {
    React.startTransition(() => {
      setTitleQuickFilters({
        monitored: false,
        unmonitored: false,
        continuing: false,
        ended: false,
      });
    });
  }, []);

  const updateTitleCatalogSort = React.useCallback((nextKey: TitleTableSortKey) => {
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
  }, []);

  const toggleAllVisibleTitles = React.useCallback(
    (checked: boolean) => {
      setSelectedTitleIds(
        checked ? new Set(visibleTitles.map((title) => title.id)) : new Set(),
      );
    },
    [visibleTitles],
  );

  const clearSelectedTitles = React.useCallback(() => {
    setSelectedTitleIds((current) => (current.size === 0 ? current : new Set()));
  }, []);

  const setViewMode = React.useCallback((nextMode: ContentViewMode) => {
    setDesktopViewMode(nextMode);
  }, []);

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
          .mutation<Record<string, { id: string; monitored: boolean }>>(
            buildSetTitleMonitoredBatchMutation(targets.length),
            variables,
          )
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
      const targets = [...selectedTitles];
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
          .mutation<Record<string, { id: string }>>(
            buildUpdateTitleBatchMutation(targets.length),
            variables,
          )
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
    [bulkActionBusy, client, reloadTitles, selectedTitles, setGlobalStatus, t],
  );

  const closeBulkDeleteDialog = React.useCallback(() => {
    setBulkDeleteDialogOpen(false);
    setBulkDeleteFilesOnDisk(false);
    setBulkDeleteTypedConfirmation("");
    setBulkDeletePreviewLoading(false);
    setBulkDeletePreviewError(null);
    setBulkDeletePreviewsByTitleId({});
  }, []);

  React.useEffect(() => {
    if (!isMediaView || librariesLoading) {
      setInvalidRootLibraryIds([]);
      return;
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

    if (librariesWithConfiguredRoots.length === 0) {
      setInvalidRootLibraryIds([]);
      return;
    }

    let cancelled = false;

    const validateRoots = async () => {
      const invalidIds = new Set<string>();

      await Promise.all(
        librariesWithConfiguredRoots.map(async (library) => {
          const configuredPaths = library.roots
            .map((root) => root.path.trim())
            .filter((path) => path.length > 0);
          if (configuredPaths.length === 0) {
            return;
          }

          const validationResults = await Promise.all(
            configuredPaths.map(async (path) => {
              const { error } = await client
                .query(browsePathQuery, { path }, { requestPolicy: "network-only" })
                .toPromise();
              return error != null;
            }),
          );

          if (validationResults.some(Boolean)) {
            invalidIds.add(library.id);
          }
        }),
      );

      if (!cancelled) {
        setInvalidRootLibraryIds([...invalidIds]);
      }
    };

    void validateRoots().catch((error) => {
      console.error("[library-root-validation] failed to validate root folders:", error);
      if (!cancelled) {
        setInvalidRootLibraryIds([]);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [client, isMediaView, libraries, librariesLoading, selectedLibraryIds]);

  React.useEffect(() => {
    if (!bulkDeleteFilesOnDisk) {
      setBulkDeleteTypedConfirmation("");
      setBulkDeletePreviewLoading(false);
      setBulkDeletePreviewError(null);
      setBulkDeletePreviewsByTitleId({});
    }
  }, [bulkDeleteFilesOnDisk]);

  React.useEffect(() => {
    if (!bulkDeleteDialogOpen || !bulkDeleteFilesOnDisk) {
      return;
    }

    const targets = [...selectedTitles];
    if (targets.length === 0) {
      setBulkDeletePreviewLoading(false);
      setBulkDeletePreviewError(null);
      setBulkDeletePreviewsByTitleId({});
      return;
    }

    let cancelled = false;
    setBulkDeletePreviewLoading(true);
    setBulkDeletePreviewError(null);

    const loadPreviews = async () => {
      try {
        const result = await client
          .query<{ deleteTitlesPreview: DeleteTitlesPreview }>(
            deleteTitlesPreviewQuery,
            { input: { titleIds: targets.map((title) => title.id) } },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (cancelled) {
          return;
        }

        if (result.error || !result.data?.deleteTitlesPreview) {
          throw result.error ?? new Error("delete title preview failed");
        }
        const payload = result.data.deleteTitlesPreview;
        const nextPreviewsByTitleId: Record<string, DeletePreview> = {};
        const failedTitles: string[] = [];

        for (const item of payload.items) {
          if (item.preview) {
            nextPreviewsByTitleId[item.titleId] = item.preview;
          } else {
            const title = targets.find((target) => target.id === item.titleId);
            failedTitles.push(title?.name ?? item.titleId);
          }
        }

        setBulkDeletePreviewsByTitleId(nextPreviewsByTitleId);
        if (payload.failedCount > 0) {
          setBulkDeletePreviewError(
            withFailureDetail(
              t("status.bulkDeletePreviewFailed", { failed: payload.failedCount }),
              failedTitles.slice(0, 5).join(", "),
            ),
          );
        } else {
          setBulkDeletePreviewError(null);
        }
      } catch (error) {
        if (cancelled) {
          return;
        }
        setBulkDeletePreviewsByTitleId({});
        setBulkDeletePreviewError(
          withFailureDetail(
            t("status.bulkDeletePreviewFailed", { failed: targets.length }),
            batchFailureDetail(error),
          ),
        );
      } finally {
        if (!cancelled) {
          setBulkDeletePreviewLoading(false);
        }
      }
    };

    void loadPreviews();
    return () => {
      cancelled = true;
    };
  }, [
    bulkDeleteDialogOpen,
    bulkDeleteFilesOnDisk,
    client,
    selectedTitles,
    t,
  ]);

  const bulkDeletePreview = React.useMemo(
    () =>
      aggregateDeletePreviews(
        Object.values(bulkDeletePreviewsByTitleId).filter(Boolean),
      ),
    [bulkDeletePreviewsByTitleId],
  );
  const bulkDeletePreviewMissing =
    bulkDeleteFilesOnDisk &&
    selectedTitles.some((title) => !bulkDeletePreviewsByTitleId[title.id]);
  const bulkDeleteConfirmDisabled =
    bulkActionBusy ||
    selectedTitles.length === 0 ||
    (bulkDeleteFilesOnDisk &&
      (bulkDeletePreviewLoading ||
        !!bulkDeletePreviewError ||
        bulkDeletePreviewMissing ||
        !bulkDeletePreview ||
        (bulkDeletePreview.requiresTypedConfirmation &&
          bulkDeleteTypedConfirmation.trim() !== "DELETE")));

  const confirmBulkDeleteTitles = React.useCallback(async () => {
    const targets = [...selectedTitles];
    if (targets.length === 0 || bulkActionBusy) {
      return;
    }

    setBulkActionBusy(true);
    try {
      const items = targets.map((title) => {
        const preview = bulkDeletePreviewsByTitleId[title.id];
        if (bulkDeleteFilesOnDisk && !preview) {
          throw new Error("Delete preview is not ready yet.");
        }
        return {
          titleId: title.id,
          ...(bulkDeleteFilesOnDisk
            ? { previewFingerprint: preview?.fingerprint }
            : {}),
        };
      });
      const result = await client
        .mutation<{
          deleteTitles?: {
            acceptedTitleIds?: string[];
            jobRun?: unknown;
          };
        }>(deleteTitlesMutation, {
          input: {
            items,
            deleteFilesOnDisk: bulkDeleteFilesOnDisk,
            ...(bulkDeleteFilesOnDisk && bulkDeleteTypedConfirmation.trim()
              ? { typedConfirmation: bulkDeleteTypedConfirmation.trim() }
              : {}),
          },
        })
        .toPromise();
      if (result.error) {
        throw result.error;
      }
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
      setSelectedTitleIds(new Set());
      closeBulkDeleteDialog();
      setGlobalStatus(
        `Queued deletion for ${acceptedIds.length} title${acceptedIds.length === 1 ? "" : "s"}.`,
      );
    } catch (error) {
      setGlobalStatus(
        withFailureDetail(
          t("status.bulkTitleDeleteFailed"),
          batchFailureDetail(error),
        ),
      );
    } finally {
      setBulkActionBusy(false);
    }
  }, [
    bulkActionBusy,
    bulkDeleteFilesOnDisk,
    bulkDeletePreviewsByTitleId,
    bulkDeleteTypedConfirmation,
    client,
    closeBulkDeleteDialog,
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    scheduleDeletionJobFallbackChecks,
    selectedTitles,
    setGlobalStatus,
    t,
  ]);

  const openBulkTitleEdit = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    if (selectedTitleLibraryIds.length !== 1) {
      setGlobalStatus("Bulk actions require titles from one library.");
      return;
    }
    setBulkEditDialogOpen(true);
  }, [bulkActionBusy, selectedTitleLibraryIds.length, selectedTitles.length, setGlobalStatus]);

  const openBulkTitleDelete = React.useCallback(() => {
    if (selectedTitles.length === 0 || bulkActionBusy) {
      return;
    }
    if (selectedTitleLibraryIds.length !== 1) {
      setGlobalStatus("Bulk actions require titles from one library.");
      return;
    }
    setBulkDeleteFilesOnDisk(false);
    setBulkDeleteTypedConfirmation("");
    setBulkDeletePreviewLoading(false);
    setBulkDeletePreviewError(null);
    setBulkDeletePreviewsByTitleId({});
    setBulkDeleteDialogOpen(true);
  }, [bulkActionBusy, selectedTitleLibraryIds.length, selectedTitles.length, setGlobalStatus]);

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
    recordCriticalCatalogMutation,
    registerInteractiveJobRun,
    scheduleDeletionJobFallbackChecks,
    titleDeletePreview,
    titleDeleteTypedConfirmation,
    t,
    titleToDelete,
    setGlobalStatus,
    setDeleteTitleLoadingById,
  ]);

  const deleteTitleConfirmDisabled =
    deleteFilesOnDisk &&
    (titleDeletePreviewLoading ||
      !!titleDeletePreviewError ||
      !titleDeletePreview ||
      (titleDeletePreview.requiresTypedConfirmation &&
        titleDeleteTypedConfirmation.trim() !== "DELETE"));

  const refreshLibraries = React.useCallback(async (): Promise<LibraryRecord[] | null> => {
    if (!isMediaView) {
      setLibraries([]);
      return [];
    }
    const permission =
      contentSettingsSection === "library" && !canManageConfig ? "manageLibrary" : "view";
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
      setSelectedLibraryIds((current) =>
        normalizeLibraryFilterSelection(current, nextLibraries),
      );
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
    t,
  ]);

  const refreshRootValidationLibraries = React.useCallback(
    async (): Promise<LibraryRecord[] | null> => {
      if (!isMediaView) {
        setRootValidationLibraries([]);
        return [];
      }
      const permission =
        contentSettingsSection === "library" && !canManageConfig ? "manageLibrary" : "view";
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
    },
    [canManageConfig, client, contentSettingsSection, isMediaView, setGlobalStatus, t],
  );

  const loadLibrarySettings = React.useCallback(
    async (libraryId: string): Promise<LibrarySettingsRecord | null> => {
      const { data, error } = await client
        .query<{ librarySettings: LibrarySettingsRecord }>(
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
        .query<{ downloadClientRouting: DownloadClientRoutingEntry[] }>(
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
    async (input: { name: string; roots: RootFolderOption[]; settings?: LibrarySettingsDraft }) => {
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
          error instanceof Error ? error.message : t("settings.librarySaveFailed"),
        );
        return null;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [activeFacet, client, refreshLibraries, refreshRootValidationLibraries, setGlobalStatus, t],
  );

  const updateLibrary = React.useCallback(
    async (
      libraryId: string,
      input: { name: string; roots: RootFolderOption[]; settings?: LibrarySettingsDraft },
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
          error instanceof Error ? error.message : t("settings.librarySaveFailed"),
        );
        return null;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [client, refreshLibraries, refreshRootValidationLibraries, setGlobalStatus, t],
  );

  const deleteLibrary = React.useCallback(
    async (libraryId: string) => {
      setLibrarySettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ deleteLibrary: { id: string; deleted: boolean } }>(deleteLibraryMutation, {
            id: libraryId,
          })
          .toPromise();
        if (error) throw error;
        if (!data?.deleteLibrary?.deleted) {
          throw new Error(t("settings.libraryDeleteFailed"));
        }
        setSelectedLibraryIds((current) =>
          current.filter((selectedLibraryId) => selectedLibraryId !== libraryId),
        );
        await refreshLibraries();
        await refreshRootValidationLibraries();
        setGlobalStatus(t("settings.libraryDeleted"));
        return true;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("settings.libraryDeleteFailed"),
        );
        return false;
      } finally {
        setLibrarySettingsSaving(false);
      }
    },
    [client, refreshLibraries, refreshRootValidationLibraries, setGlobalStatus, t],
  );

  const handleLibraryScan = React.useCallback(async (libraryId?: string) => {
    const targetLibraryId = libraryId ?? singleSelectedLibraryId(selectedLibraryIds);
    if (!targetLibraryId) {
      setLibraryScanNotice("Choose a library to scan.");
      return;
    }
    if (activeLibraryScanSession) {
      setLibraryScanNotice(
        t("settings.libraryScanAlreadyRunning", {
          facet: activeFacetLabel,
        }),
      );
      return;
    }

    setLibraryScanNotice(null);
    setLibraryScanLoading(true);
    setLibraryScanSummary(null);
    setStartedLibraryScanSessionId(null);
    try {
      const result = await client
        .mutation(scanLibraryMutation, { input: { libraryId: targetLibraryId } })
        .toPromise();
      if (result.error) throw result.error;
      const sessionId = result.data?.scanLibrary?.sessionId ?? null;
      setStartedLibraryScanSessionId(sessionId);
      void refreshLibraryScanSessions().catch((error) => {
        console.error("[library-scan] failed to refresh active scan sessions:", error);
      });
    } catch (error) {
      console.error("[library-scan] mutation failed:", error);
      const message =
        error instanceof Error ? error.message : String(error ?? "");
      if (/library scan already running/i.test(message)) {
        setLibraryScanNotice(
          t("settings.libraryScanAlreadyRunning", {
            facet: activeFacetLabel,
          }),
        );
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
      setLibraryScanLoading(false);
    }
  }, [
    activeFacetLabel,
    activeLibraryScanSession,
    client,
    selectedLibraryIds,
    refreshLibraryScanSessions,
    setLibraryScanLoading,
    setLibraryScanNotice,
    setLibraryScanSummary,
    setStartedLibraryScanSessionId,
    setGlobalStatus,
    t,
  ]);

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

  React.useEffect(() => {
    if (
      shouldLoadCatalogTitles ||
      catalogBootstrapState.facet !== activeFacet ||
      !catalogBootstrapState.loading
    ) {
      return;
    }

    setCatalogBootstrapState((current) =>
      current.facet === activeFacet && current.loading
        ? { ...current, loading: false }
        : current,
    );
  }, [
    activeFacet,
    catalogBootstrapState.facet,
    catalogBootstrapState.loading,
    shouldLoadCatalogTitles,
  ]);

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
      if (!catalogInitialLoadComplete) {
        if (catalogBootstrapInFlight) {
          return;
        }

        // Keep bootstrap completion stable across rerenders while loading.
        // Effect-local cleanup would cancel the bootstrap as soon as the
        // loading state rerendered, leaving the catalog permanently blank.
        const requestSeq = ++catalogBootstrapRequestSeqRef.current;
        skipNextCatalogOverviewReloadRef.current = false;
        setCatalogBootstrapState({
          facet: activeFacet,
          loading: true,
          initialLoadComplete: false,
        });

        const finalizeBootstrap = () => {
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }

          skipNextCatalogOverviewReloadRef.current = true;
          setCatalogBootstrapState({
            facet: activeFacet,
            loading: false,
            initialLoadComplete: true,
          });
        };

        const librariesPromise = refreshLibraries();
        void reloadTitles(debouncedTitleFilter, []).then((nextTitles) => {
          if (catalogBootstrapRequestSeqRef.current !== requestSeq) {
            return;
          }

          if ((nextTitles?.length ?? 0) > 0) {
            finalizeBootstrap();
            return;
          }

          void librariesPromise.finally(finalizeBootstrap);
        });
      }

      if (skipNextCatalogOverviewReloadRef.current) {
        skipNextCatalogOverviewReloadRef.current = false;
      } else {
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
    catalogBootstrapInFlight,
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    canManageConfig,
    client,
    contentSettingsSection,
    refreshLibraries,
    hydrateDownloadClientRouting,
    hydrateIndexerRouting,
    isMediaView,
    refreshRuleSets,
    debouncedTitleFilter,
    reloadTitles,
    setGlobalStatus,
    shouldLoadCatalogTitles,
    t,
    view,
  ]);

  if (contentSettingsSection === "requests") {
    return <RequestsContainer facet={activeFacet} />;
  }

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
          globalQualityProfileId,
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
          catalogHasMoreTitles: catalogPaginationState.hasMore,
          catalogLoadingMoreTitles: catalogPaginationState.loadingMore,
          loadMoreCatalogTitles,
          titleCatalogSortKey: titleCatalogSort.key,
          titleCatalogSortDirection: titleCatalogSort.direction,
          updateTitleCatalogSort,
          catalogBootstrapLoading,
          catalogInitialLoadComplete,
          monitoredTitles: visibleTitles,
          titleQuickFilters,
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
          libraryScanDisabled: libraryScanInProgress || selectedLibraryIds.length !== 1,
          libraryScanNotice,
          libraryScanSummary,
          libraries,
          librariesLoading,
          libraryDownloadClients,
          libraryDownloadClientsLoading,
          rootValidationLibraries,
          rootValidationLibrariesLoading,
          invalidRootLibraryIds,
          selectedLibraryIds,
          allLibrariesValue: ALL_LIBRARIES_VALUE,
          setSelectedLibraryIds,
          loadLibrarySettings,
          loadFacetDownloadClientRouting,
          createLibrary,
          updateLibrary,
          deleteLibrary,
          onOpenOverview,
          scanLibrary: handleLibraryScan,
          deleteCatalogTitle: requestDeleteTitle,
          isDeletingCatalogTitleById: deleteTitleLoadingById,
          isMobile,
          viewMode: desktopViewMode,
          setViewMode,
          selectedTitleIds,
          toggleTitleSelection,
          toggleAllVisibleTitles,
          clearSelectedTitles,
          bulkActionBusy,
          bulkMonitorTitles,
          openBulkTitleEdit,
          openBulkTitleDelete,
        }}
      />
      <BulkTitleEditDialog
        open={bulkEditDialogOpen}
        onOpenChange={setBulkEditDialogOpen}
        view={view}
        selectedTitles={selectedTitles}
        qualityProfiles={qualityProfiles}
        rootFolders={bulkRootFolders}
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
      {replaceConflictDialog}
    </>
  );
});
