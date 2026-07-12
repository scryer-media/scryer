
import * as React from "react";
import { Eye, EyeOff, LayoutGrid, LayoutList, Pencil, Trash2, X } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import type { ContentSettingsSection, OverviewTitleTarget, ViewId } from "@/components/root/types";
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
  TitleRecord,
} from "@/lib/types";
import type { ImportMode } from "@/lib/types/settings";
import type { ViewCategoryId } from "./media-content/indexer-category-picker";
import { MediaLibrarySettingsPanel } from "./media-content/media-library-settings-panel";
import { IndexerRoutingPanel } from "./media-content/indexer-routing-panel";
import { DownloadClientRoutingPanel } from "./media-content/download-client-routing-panel";
import { GeneralSettingsPanel } from "./media-content/general-settings-panel";
import { QualitySettingsPanel } from "./media-content/quality-settings-panel";
import { RenameSettingsPanel } from "./media-content/rename-settings-panel";
import { AddTitleForm } from "./media-content/add-title-form";
import { PosterGrid } from "./media-content/poster-grid";
import { TitleTable } from "./media-content/title-table";
import { CompactTitleTable } from "./media-content/compact-title-table";
import {
  TitleTableActionButton,
  type TitleTableSortDirection,
  type TitleTableSortKey,
} from "./media-content/title-table-shared";
import { titleOverviewViewModeId } from "@/lib/utils/dom-ids";
import {
  hasActiveTitleQuickFilters,
  TitleQuickFilterBar,
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
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import type { ContentViewMode } from "./media-content/content-view-mode";

type Facet = "movie" | "series" | "anime";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type QualityProfileOption = {
  value: string;
  label: string;
};

type TvdbSearchItem = MetadataTvdbSearchItem;

type ScopeRoutingRecord = Record<string, NzbgetCategoryRoutingSettings>;
type IndexerRoutingRecord = Record<string, IndexerCategoryRoutingSettings>;

function formatQualityProfileFallback(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.toLowerCase() === "4k") {
    return "4K";
  }
  if (/^\d{3,4}p$/i.test(trimmed)) {
    return trimmed.toUpperCase();
  }
  return trimmed;
}

function CompactTableIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      className="h-4 w-4"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
      <path d="M2 6.5h12M2 10h12M6 2.5v11" />
    </svg>
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
    globalQualityProfileId: string;
    globalScoringPersona: ScoringPersonaId;
    categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
    categoryRequiredAudioLanguages: Record<ViewCategoryId, string[]>;
    saveCategoryRequiredAudioLanguages: (languages: string[]) => Promise<void> | void;
    categoryPersonaSelections: Record<ViewCategoryId, FacetScoringPersonaSelectionRecord>;
    activeQualityScopeId: ViewCategoryId;
    categoryFolderTemplates: Record<ViewCategoryId, string>;
    setCategoryFolderTemplates: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categorySeasonFolderTemplates: Record<ViewCategoryId, string>;
    setCategorySeasonFolderTemplates: React.Dispatch<
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
    qualityProfileInheritValue: string;
    toProfileOptions: (profiles: ParsedQualityProfile[]) => QualityProfileOption[];
    handleFacetPersonaSave: (persona: ScoringPersonaId | null) => Promise<void> | void;
    saveSetting: (scope: string, scopeId: string | undefined, keyName: string, value: string) => void;
    saveCategoryQualityProfileOverride: (value: string) => Promise<void> | void;
    updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
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
    onAddSubmit: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
    addTvdbCandidateToCatalog: (candidate: TvdbSearchItem) => Promise<void> | void;
    titleFilter: string;
    setTitleFilter: (value: string) => void;
    refreshTitles: (query?: string) => Promise<void> | void;
    titleLoading: boolean;
    catalogHasMoreTitles: boolean;
    catalogLoadingMoreTitles: boolean;
    loadMoreCatalogTitles: () => Promise<void> | void;
    titleCatalogSortKey: TitleTableSortKey;
    titleCatalogSortDirection: TitleTableSortDirection;
    updateTitleCatalogSort: (key: TitleTableSortKey) => void;
    catalogBootstrapLoading: boolean;
    catalogInitialLoadComplete: boolean;
    monitoredTitles: TitleRecord[];
    titleQuickFilters: TitleQuickFilters;
    toggleTitleQuickMonitoringFilter: (
      filter: "monitored" | "unmonitored",
    ) => void;
    toggleTitleQuickStatusFilter: (filter: "continuing" | "ended") => void;
    clearTitleQuickFilters: () => void;
    queueExisting: (title: TitleRecord) => Promise<void> | void;
    toggleTitleMonitored: (title: TitleRecord, monitored: boolean) => Promise<void> | void;
    runInteractiveSearchForTitle: (title: TitleRecord) => Promise<Release[]> | Release[];
    queueExistingFromRelease: (title: TitleRecord, release: Release) => Promise<void> | void;
    queueAdditionalFromRelease: (title: TitleRecord, release: Release) => Promise<void> | void;
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
    moveDownloadClientInScope: (clientId: string, direction: "up" | "down") => void;
    indexers: IndexerRecord[];
    activeScopeIndexerRouting: IndexerRoutingRecord;
    activeScopeIndexerRoutingOrder: string[];
    indexerRoutingLoading: boolean;
    indexerRoutingSaving: boolean;
    setIndexerEnabledForScope: (indexerId: string, enabled: boolean) => Promise<void> | void;
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
    invalidRootLibraryIds: string[];
    selectedLibraryIds: string[];
    allLibrariesValue: string;
    setSelectedLibraryIds: (value: string[]) => void;
    libraryDownloadClients: DownloadClientRecord[];
    libraryDownloadClientsLoading: boolean;
    loadLibrarySettings: (libraryId: string) => Promise<LibrarySettingsRecord | null>;
    loadFacetDownloadClientRouting: (
      scopeId: Facet,
    ) => Promise<DownloadClientRoutingEntry[]>;
    createLibrary: (input: { name: string; roots: import("@/lib/types/titles").RootFolderOption[]; settings?: LibrarySettingsDraft }) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
    updateLibrary: (libraryId: string, input: { name: string; roots: import("@/lib/types/titles").RootFolderOption[]; settings?: LibrarySettingsDraft }) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
    deleteLibrary: (libraryId: string) => Promise<boolean | void> | boolean | void;
    scanLibrary: (libraryId?: string) => Promise<void> | void;
    onOpenOverview: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
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
  };
}) {
  const t = useTranslate();
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
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    loadMoreCatalogTitles,
    titleCatalogSortKey,
    titleCatalogSortDirection,
    updateTitleCatalogSort,
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    monitoredTitles,
    titleQuickFilters,
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
    invalidRootLibraryIds,
    selectedLibraryIds,
    allLibrariesValue,
    setSelectedLibraryIds,
    scanLibrary,
    onOpenOverview,
    deleteCatalogTitle,
    isDeletingCatalogTitleById,
    isMobile,
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
  } = state;
  const [titleFilterInputValue, setTitleFilterInputValue] = React.useState(titleFilter);
  const deferredMonitoredTitles = React.useDeferredValue(monitoredTitles);

  React.useEffect(() => {
    setTitleFilterInputValue((current) => (
      current === titleFilter ? current : titleFilter
    ));
  }, [titleFilter]);
  const compactSelectedVisibleCount = React.useMemo(
    () => deferredMonitoredTitles.filter((title) => selectedTitleIds.has(title.id)).length,
    [deferredMonitoredTitles, selectedTitleIds],
  );
  const effectiveContentSettingsSection =
    canAccessMediaSettingsSection(
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
    activeQualityScopeId === "movie"
      ? t("search.facetMovie")
      : activeQualityScopeId === "series"
        ? t("search.facetSeries")
        : t("search.facetAnime");
  const effectiveViewMode: ContentViewMode = isMobile ? "poster" : viewMode;
  const explicitlySelectedLibraryIds = selectedLibraryIds.filter(
    (libraryId) => libraryId !== allLibrariesValue,
  );
  const selectedLibraryIdSet =
    explicitlySelectedLibraryIds.length > 0
      ? new Set(explicitlySelectedLibraryIds)
      : null;
  const relevantLibraries = selectedLibraryIdSet
    ? libraries.filter((library) => selectedLibraryIdSet.has(library.id))
    : libraries;
  const hasConfiguredRootFolders =
    !catalogInitialLoadComplete || librariesLoading
    ? null
    : relevantLibraries.some((library) =>
        library.roots.some((folder) => folder.path.trim().length > 0),
      );
  const hasInvalidConfiguredRootFolders =
    catalogInitialLoadComplete &&
    !librariesLoading &&
    relevantLibraries.some((library) => invalidRootLibraryIds.includes(library.id));
  const showInitialScanAction =
    canManageLibrarySettings &&
    catalogInitialLoadComplete &&
    monitoredTitles.length === 0 &&
    hasConfiguredRootFolders === true &&
    !hasInvalidConfiguredRootFolders;
  const showConfigureRootFoldersAction =
    canManageLibrarySettings &&
    catalogInitialLoadComplete &&
    monitoredTitles.length === 0 &&
    (hasConfiguredRootFolders === false || hasInvalidConfiguredRootFolders);
  const configureRootFoldersReason =
    hasInvalidConfiguredRootFolders ? "invalid" : "missing";
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
      setCategoryFillerPolicies((previous) => ({ ...previous, [activeQualityScopeId]: value }));
      saveSetting("system", activeQualityScopeId, "anime.filler_policy", value);
    },
    [activeQualityScopeId, setCategoryFillerPolicies, saveSetting],
  );

  const handleRecapPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRecapPolicies((previous) => ({ ...previous, [activeQualityScopeId]: value }));
      saveSetting("system", activeQualityScopeId, "anime.recap_policy", value);
    },
    [activeQualityScopeId, setCategoryRecapPolicies, saveSetting],
  );

  const handleMonitorSpecialsChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryMonitorSpecials((previous) => ({ ...previous, [activeQualityScopeId]: value }));
      saveSetting("system", activeQualityScopeId, "anime.monitor_specials", value);
    },
    [activeQualityScopeId, setCategoryMonitorSpecials, saveSetting],
  );

  const handleInterSeasonMoviesChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryInterSeasonMovies((previous) => ({ ...previous, [activeQualityScopeId]: value }));
      saveSetting("system", activeQualityScopeId, "anime.inter_season_movies", value);
    },
    [activeQualityScopeId, setCategoryInterSeasonMovies, saveSetting],
  );

  const handleMonitorFillerMoviesChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryMonitorFillerMovies((previous) => ({ ...previous, [activeQualityScopeId]: value }));
      saveSetting("system", activeQualityScopeId, "anime.monitor_filler_movies", value);
    },
    [activeQualityScopeId, setCategoryMonitorFillerMovies, saveSetting],
  );

  const handleNfoWriteChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      const key = activeQualityScopeId === "movie" ? "nfo.write_on_import.movie"
        : activeQualityScopeId === "anime" ? "nfo.write_on_import.anime"
        : "nfo.write_on_import.series";
      setNfoWriteOnImport((previous) => ({ ...previous, [activeQualityScopeId]: value }));
      saveSetting("system", undefined, key, value);
    },
    [activeQualityScopeId, setNfoWriteOnImport, saveSetting],
  );

  const handlePlexmatchWriteChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      const key = activeQualityScopeId === "anime" ? "plexmatch.write_on_import.anime" : "plexmatch.write_on_import.series";
      setPlexmatchWriteOnImport((previous) => ({ ...previous, [activeQualityScopeId]: value }));
      saveSetting("system", undefined, key, value);
    },
    [activeQualityScopeId, setPlexmatchWriteOnImport, saveSetting],
  );

  const handleImportModeChange = React.useCallback(
    (value: ImportMode) => {
      setImportMode((previous) => ({ ...previous, [activeQualityScopeId]: value }));
      saveSetting("system", activeQualityScopeId, "import.mode", value);
    },
    [activeQualityScopeId, saveSetting, setImportMode],
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

  const handleTitleFilterChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      const nextValue = event.target.value;
      setTitleFilterInputValue(nextValue);
      React.startTransition(() => {
        setTitleFilter(nextValue);
      });
    },
    [setTitleFilter],
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

  const handleLibraryScan = React.useCallback((libraryId?: string) => {
    void scanLibrary(libraryId);
  }, [scanLibrary]);

  const quickFilterView = view === "movies" ? "movies" : view === "series" ? "series" : "anime";
  const hasActiveTitleDisplayFilters =
    titleFilter.trim().length > 0
    || hasActiveTitleQuickFilters(titleQuickFilters, quickFilterView);
  const showEmptyStateActions = !hasActiveTitleDisplayFilters;

  const handleDeleteCatalogTitle = React.useCallback(
    (title: TitleRecord) => {
      deleteCatalogTitle(title);
    },
    [deleteCatalogTitle],
  );

  return (
    <div className="space-y-4">
      {effectiveContentSettingsSection === "quality" ? (
        <QualitySettingsPanel
          contentSettingsLabel={contentSettingsLabel}
          mediaSettingsLoading={mediaSettingsLoading}
          mediaSettingsSaving={mediaSettingsSaving}
          qualityProfiles={qualityProfiles}
          qualityProfileParseError={qualityProfileParseError}
          categoryQualityProfileOverrides={categoryQualityProfileOverrides}
          categoryRequiredAudioLanguages={categoryRequiredAudioLanguages}
          saveCategoryRequiredAudioLanguages={saveCategoryRequiredAudioLanguages}
          activeQualityScopeId={activeQualityScopeId}
          globalScoringPersona={globalScoringPersona}
          categoryPersonaSelections={categoryPersonaSelections}
          qualityProfileInheritValue={qualityProfileInheritValue}
          toProfileOptions={toProfileOptions}
          saveCategoryQualityProfileOverride={saveCategoryQualityProfileOverride}
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
          categoryRenameMissingMetadataPolicies={categoryRenameMissingMetadataPolicies}
          handleRenameMissingMetadataPolicyChange={handleRenameMissingMetadataPolicyChange}
          updateCategoryMediaProfileSettings={updateCategoryMediaProfileSettings}
        />
      ) : effectiveContentSettingsSection === "routing" ? (
        <div className="space-y-4">
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
            updateDownloadClientRoutingForScope={updateDownloadClientRoutingForScope}
            moveDownloadClientInScope={moveDownloadClientInScope}
          />
        </div>
      ) : effectiveContentSettingsSection === "library" ? (
        view === "movies" || view === "series" || view === "anime" ? (
          <MediaLibrarySettingsPanel
            facet={view === "movies" ? "movie" : view === "series" ? "series" : "anime"}
            settingsTitle={mediaLibrarySettingsTitle}
            libraries={libraries}
            librariesLoading={librariesLoading}
            rootValidationLibraries={rootValidationLibraries}
            rootValidationLibrariesLoading={rootValidationLibrariesLoading}
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
            loadFacetDownloadClientRouting={state.loadFacetDownloadClientRouting}
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
        />
      ) : (
        view === "movies" || view === "series" || view === "anime" ? (
          <Card id={`media-overview-${view}`}>
            <CardHeader>
              <CardTitle>{view === "movies" ? t("title.manageMovies") : view === "anime" ? t("nav.anime") : t("nav.series")}</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
                <Input
                  placeholder={t("title.filterPlaceholder")}
                  value={titleFilterInputValue}
                  onChange={handleTitleFilterChange}
                  className="w-full sm:flex-1"
                />
                <LibraryMultiSelect
                  libraries={libraries}
                  selectedLibraryIds={selectedLibraryIds}
                  onSelectedLibraryIdsChange={setSelectedLibraryIds}
                  disabled={librariesLoading || libraries.length === 0}
                  triggerClassName="w-full sm:w-[180px]"
                />
                {!isMobile ? (
                  <ToggleGroup
                    type="single"
                    value={viewMode}
                    onValueChange={(v) => {
                      if (
                        v === "compact" ||
                        v === "poster-table" ||
                        v === "poster"
                      ) {
                        setViewMode(v);
                      }
                    }}
                    size="sm"
                    aria-label={t("title.viewModeToggle")}
                  >
                    <ToggleGroupItem
                      id={titleOverviewViewModeId(view, "compact")}
                      value="compact"
                      size="sm"
                      aria-label={t("title.viewModeCompact")}
                      title={t("title.viewModeCompact")}
                      className="data-[state=on]:!border-purple-900/80 data-[state=on]:!shadow-[0_0_0_2px_rgba(88,28,135,0.55)]"
                    >
                      <CompactTableIcon />
                    </ToggleGroupItem>
                    <ToggleGroupItem
                      id={titleOverviewViewModeId(view, "poster-table")}
                      value="poster-table"
                      size="sm"
                      aria-label={t("title.viewModePosterTable")}
                      title={t("title.viewModePosterTable")}
                      className="data-[state=on]:!border-purple-900/80 data-[state=on]:!shadow-[0_0_0_2px_rgba(88,28,135,0.55)]"
                    >
                      <LayoutList className="h-4 w-4" />
                    </ToggleGroupItem>
                    <ToggleGroupItem
                      id={titleOverviewViewModeId(view, "poster")}
                      value="poster"
                      size="sm"
                      aria-label={t("title.viewModePoster")}
                      title={t("title.viewModePoster")}
                      className="data-[state=on]:!border-purple-900/80 data-[state=on]:!shadow-[0_0_0_2px_rgba(88,28,135,0.55)]"
                    >
                      <LayoutGrid className="h-4 w-4" />
                    </ToggleGroupItem>
                  </ToggleGroup>
                ) : null}
                <Button
                  id={`title-overview-refresh-${view === "movies" ? "movie" : view === "series" ? "series" : "anime"}`}
                  className="w-full sm:w-auto"
                  variant="primary"
                  onClick={handleRefreshTitles}
                  disabled={titleLoading}
                >
                  {t("label.refresh")}
                </Button>
              </div>
              <div className="mb-3">
                <TitleQuickFilterBar
                  view={view}
                  filters={titleQuickFilters}
                  onToggleMonitoring={toggleTitleQuickMonitoringFilter}
                  onToggleStatus={toggleTitleQuickStatusFilter}
                  onClear={clearTitleQuickFilters}
                  trailingContent={
                    effectiveViewMode === "compact" || effectiveViewMode === "poster-table" ? (
                      compactSelectedVisibleCount > 0 ? (
                        <div className="flex h-12 w-full items-center justify-end gap-2 rounded-xl border border-border/70 bg-muted/20 px-3 py-2 sm:w-[20rem]">
                          <span className="mr-1 text-sm text-muted-foreground whitespace-nowrap">
                            {t("title.bulkSelectionCount", { count: compactSelectedVisibleCount })}
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
                      ) : (
                        <div className="h-12 w-full sm:w-[20rem]" aria-hidden="true" />
                      )
                    ) : null
                  }
                />
              </div>
              {(() => {
                const isMovieView = view === "movies";
                const overviewTargetView = isMovieView ? "movies" as const : view === "anime" ? "anime" as const : "series" as const;
                const resolvedProfileName = (() => {
                  const overrideId = categoryQualityProfileOverrides[activeQualityScopeId];
                  const effectiveId = (!overrideId || overrideId === qualityProfileInheritValue)
                    ? globalQualityProfileId
                    : overrideId;
                  return qualityProfiles.find((p) => p.id === effectiveId)?.name
                    ?? formatQualityProfileFallback(effectiveId)
                    ?? qualityProfiles[0]?.name
                    ?? null;
                })();

                if (effectiveViewMode === "poster") {
                  return (
                    <PosterGrid
                      titles={deferredMonitoredTitles}
                      catalogInitialLoadComplete={catalogInitialLoadComplete}
                      isMovieView={isMovieView}
                      resolvedProfileName={resolvedProfileName}
                      qualityProfiles={qualityProfiles}
                      qualityProfilesLoading={mediaSettingsLoading}
                      onOpenOverview={onOpenOverview}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      isDeletingById={isDeletingCatalogTitleById}
                      overviewTargetView={overviewTargetView}
                      showScanLibraryAction={showEmptyStateActions && showInitialScanAction}
                      showConfigureRootsAction={
                        showEmptyStateActions && showConfigureRootFoldersAction
                      }
                      configureRootsReason={configureRootFoldersReason}
                      configureRootsHref={configureRootFoldersHref}
                      onScanLibrary={scanLibrary}
                      scanLibraryLoading={libraryScanLoading}
                      scanLibraryDisabled={libraryScanDisabled}
                      scanLibraryNotice={libraryScanNotice}
                    />
                  );
                }

                if (effectiveViewMode === "compact") {
                  return (
                    <CompactTitleTable
                      view={view}
                      titles={deferredMonitoredTitles}
                      titleLoading={titleLoading || catalogBootstrapLoading}
                      catalogHasMoreTitles={catalogHasMoreTitles}
                      catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                      onCatalogEndReached={loadMoreCatalogTitles}
                      sortKey={titleCatalogSortKey}
                      sortDirection={titleCatalogSortDirection}
                      onSortChange={updateTitleCatalogSort}
                      resolvedProfileName={resolvedProfileName}
                      qualityProfiles={qualityProfiles}
                      qualityProfilesLoading={mediaSettingsLoading}
                      onOpenOverview={onOpenOverview}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      onToggleMonitored={toggleTitleMonitored}
                      onInteractiveSearch={runInteractiveSearchForTitle}
                      onQueueFromInteractive={queueExistingFromRelease}
                      onQueueAdditionalFromInteractive={queueAdditionalFromRelease}
                      isDeletingById={isDeletingCatalogTitleById}
                      isTogglingMonitoredById={isTogglingTitleMonitoredById}
                      selectedTitleIds={selectedTitleIds}
                      onToggleSelected={toggleTitleSelection}
                      onToggleSelectAll={toggleAllVisibleTitles}
                      bulkActionBusy={bulkActionBusy}
                      showScanLibraryAction={showEmptyStateActions && showInitialScanAction}
                      showConfigureRootsAction={
                        showEmptyStateActions && showConfigureRootFoldersAction
                      }
                      configureRootsReason={configureRootFoldersReason}
                      configureRootsHref={configureRootFoldersHref}
                      onScanLibrary={scanLibrary}
                      scanLibraryLoading={libraryScanLoading}
                      scanLibraryDisabled={libraryScanDisabled}
                      scanLibraryNotice={libraryScanNotice}
                    />
                  );
                }

                return (
                  <TitleTable
                    view={view}
                    titles={deferredMonitoredTitles}
                    titleLoading={titleLoading || catalogBootstrapLoading}
                    catalogHasMoreTitles={catalogHasMoreTitles}
                    catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                    onCatalogEndReached={loadMoreCatalogTitles}
                    sortKey={titleCatalogSortKey}
                    sortDirection={titleCatalogSortDirection}
                    onSortChange={updateTitleCatalogSort}
                    onOpenOverview={onOpenOverview}
                    onDelete={handleDeleteCatalogTitle}
                    onAutoQueue={queueExisting}
                    onToggleMonitored={toggleTitleMonitored}
                    onInteractiveSearch={runInteractiveSearchForTitle}
                    onQueueFromInteractive={queueExistingFromRelease}
                    onQueueAdditionalFromInteractive={queueAdditionalFromRelease}
                    isDeletingById={isDeletingCatalogTitleById}
                    isTogglingMonitoredById={isTogglingTitleMonitoredById}
                    showScanLibraryAction={showEmptyStateActions && showInitialScanAction}
                    showConfigureRootsAction={
                      showEmptyStateActions && showConfigureRootFoldersAction
                    }
                    configureRootsReason={configureRootFoldersReason}
                    configureRootsHref={configureRootFoldersHref}
                    onScanLibrary={scanLibrary}
                    scanLibraryLoading={libraryScanLoading}
                    scanLibraryDisabled={libraryScanDisabled}
                    scanLibraryNotice={libraryScanNotice}
                  />
                );
              })()}
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
        )
      )}
    </div>
  );
}
