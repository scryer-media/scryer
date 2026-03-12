
import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { LayoutGrid, LayoutList } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import type { ViewId } from "@/components/root/types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type {
  DownloadClientRecord,
  IndexerCategoryRoutingSettings,
  IndexerRecord,
  LibraryScanSummary,
  NzbgetCategoryRoutingSettings,
  Release,
  TitleRecord,
} from "@/lib/types";
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
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { RuleSetRecord } from "@/lib/types/rule-sets";
import type { ParsedQualityProfileEntry, ScoringPersonaId } from "@/lib/types/quality-profiles";

type Facet = "movie" | "tv" | "anime";
type ContentSettingsSection = "overview" | "settings" | "general" | "quality" | "renaming" | "routing";

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

export function MediaContentView({
  state,
}: {
  state: {
    view: ViewId;
    contentSettingsSection: ContentSettingsSection;
    contentSettingsLabel: string;
    moviesPath: string;
    setMoviesPath: (value: string) => void;
    seriesPath: string;
    setSeriesPath: (value: string) => void;
    mediaSettingsLoading: boolean;
    qualityProfiles: ParsedQualityProfile[];
    qualityProfileEntries: ParsedQualityProfileEntry[];
    qualityProfilesText: string;
    qualityProfileParseError: string;
    globalQualityProfileId: string;
    categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
    activeQualityScopeId: ViewCategoryId;
    setCategoryQualityProfileOverrides: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameTemplates: Record<ViewCategoryId, string>;
    setCategoryRenameTemplates: React.Dispatch<
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
    nfoWriteOnImport: Record<ViewCategoryId, string>;
    setNfoWriteOnImport: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    plexmatchWriteOnImport: Record<ViewCategoryId, string>;
    setPlexmatchWriteOnImport: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    qualityProfileInheritValue: string;
    toProfileOptions: (profiles: ParsedQualityProfile[]) => QualityProfileOption[];
    handleFacetPersonaSave: (persona: ScoringPersonaId | null) => Promise<void>;
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
    monitorSpecialsForQueue: boolean;
    setMonitorSpecialsForQueue: (value: boolean) => void;
    interSeasonMoviesForQueue: boolean;
    setInterSeasonMoviesForQueue: (value: boolean) => void;
    minAvailabilityForQueue: string;
    setMinAvailabilityForQueue: (value: string) => void;
    selectedTvdb: TvdbSearchItem | null;
    tvdbCandidates: TvdbSearchItem[];
    selectedTvdbId: string | null;
    selectTvdbCandidate: (candidate: TvdbSearchItem) => void;
    searchNzbForSelectedTvdb: () => Promise<void>;
    searchResults: Release[];
    onAddSubmit: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
    addTvdbCandidateToCatalog: (candidate: TvdbSearchItem) => Promise<void> | void;
    queueFromSearch: (release: Release) => Promise<void> | void;
    titleFilter: string;
    setTitleFilter: (value: string) => void;
    refreshTitles: () => Promise<void> | void;
    titleLoading: boolean;
    titleStatus: string;
    monitoredTitles: TitleRecord[];
    queueExisting: (title: TitleRecord) => Promise<void> | void;
    toggleTitleMonitored: (title: TitleRecord, monitored: boolean) => Promise<void> | void;
    runInteractiveSearchForTitle: (title: TitleRecord) => Promise<Release[]> | Release[];
    queueExistingFromRelease: (title: TitleRecord, release: Release) => Promise<void> | void;
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
    libraryScanSummary: LibraryScanSummary | null;
    scanLibrary: () => Promise<void> | void;
    onOpenOverview: (targetView: ViewId, titleId: string) => void;
    deleteCatalogTitle: (title: TitleRecord) => void;
    isDeletingCatalogTitleById: Record<string, boolean>;
  };
}) {
  const t = useTranslate();
  const {
    view,
    contentSettingsSection,
    contentSettingsLabel,
    moviesPath,
    setMoviesPath,
    seriesPath,
    setSeriesPath,
    mediaSettingsLoading,
    qualityProfiles,
    qualityProfileEntries,
    qualityProfileParseError,
    globalQualityProfileId,
    categoryQualityProfileOverrides,
    activeQualityScopeId,
    setCategoryQualityProfileOverrides,
    categoryRenameTemplates,
    setCategoryRenameTemplates,
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
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
    qualityProfileInheritValue,
    toProfileOptions,
    handleFacetPersonaSave,
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
    monitorSpecialsForQueue,
    setMonitorSpecialsForQueue,
    interSeasonMoviesForQueue,
    setInterSeasonMoviesForQueue,
    minAvailabilityForQueue,
    setMinAvailabilityForQueue,
    selectedTvdb,
    tvdbCandidates,
    selectedTvdbId,
    selectTvdbCandidate,
    addTvdbCandidateToCatalog,
    searchNzbForSelectedTvdb,
    searchResults,
    onAddSubmit,
    queueFromSearch,
    titleFilter,
    setTitleFilter,
    refreshTitles,
    titleLoading,
    titleStatus,
    monitoredTitles,
    queueExisting,
    toggleTitleMonitored,
    runInteractiveSearchForTitle,
    queueExistingFromRelease,
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
    libraryScanSummary,
    scanLibrary,
    onOpenOverview,
    deleteCatalogTitle,
    isDeletingCatalogTitleById,
  } = state;

  const scopeLabel =
    activeQualityScopeId === "movie"
      ? t("search.facetMovie")
      : activeQualityScopeId === "series"
        ? t("search.facetTv")
        : t("search.facetAnime");
  type ContentViewMode = "table" | "poster";
  const [viewMode, setViewMode] = React.useState<ContentViewMode>(() => {
    try {
      const stored = localStorage.getItem("scryer:content-view-mode");
      return stored === "poster" ? "poster" : "table";
    } catch {
      return "table";
    }
  });
  React.useEffect(() => {
    try { localStorage.setItem("scryer:content-view-mode", viewMode); } catch { /* noop */ }
  }, [viewMode]);

  const handleMoviesPathChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setMoviesPath(event.target.value);
    },
    [setMoviesPath],
  );

  const handleSeriesPathChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setSeriesPath(event.target.value);
    },
    [setSeriesPath],
  );

  const mediaLibraryPathValue = view === "series" ? seriesPath : moviesPath;
  const mediaLibraryPathLabel =
    view === "series" ? t("settings.seriesPathLabel") : t("settings.moviesPathLabel");
  const mediaLibraryPathPlaceholder =
    view === "series" ? t("settings.seriesPathPlaceholder") : t("settings.moviesPathPlaceholder");
  const mediaLibraryPathHelp =
    view === "series" ? t("settings.seriesPathHelp") : t("settings.moviesPathHelp");
  const mediaLibraryPathChangeHandler =
    view === "series" ? handleSeriesPathChange : handleMoviesPathChange;
  const mediaLibrarySettingsTitle =
    view === "series" ? t("settings.seriesLibrarySettings") : t("settings.moviesLibrarySettings");

  const handleQualityProfileOverrideChange = React.useCallback(
    (value: string) => {
      setCategoryQualityProfileOverrides((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryQualityProfileOverrides],
  );

  const handleRenameTemplateChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategoryRenameTemplates((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameTemplates],
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
    },
    [activeQualityScopeId, setCategoryFillerPolicies],
  );

  const handleRecapPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRecapPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryRecapPolicies],
  );

  const handleMonitorSpecialsChange = React.useCallback(
    (checked: boolean) => {
      setCategoryMonitorSpecials((previous) => ({
        ...previous,
        [activeQualityScopeId]: checked ? "true" : "false",
      }));
    },
    [activeQualityScopeId, setCategoryMonitorSpecials],
  );

  const handleInterSeasonMoviesChange = React.useCallback(
    (checked: boolean) => {
      setCategoryInterSeasonMovies((previous) => ({
        ...previous,
        [activeQualityScopeId]: checked ? "true" : "false",
      }));
    },
    [activeQualityScopeId, setCategoryInterSeasonMovies],
  );

  const handleNfoWriteChange = React.useCallback(
    (checked: boolean) => {
      setNfoWriteOnImport((previous) => ({
        ...previous,
        [activeQualityScopeId]: checked ? "true" : "false",
      }));
    },
    [activeQualityScopeId, setNfoWriteOnImport],
  );

  const handlePlexmatchWriteChange = React.useCallback(
    (checked: boolean) => {
      setPlexmatchWriteOnImport((previous) => ({
        ...previous,
        [activeQualityScopeId]: checked ? "true" : "false",
      }));
    },
    [activeQualityScopeId, setPlexmatchWriteOnImport],
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
      setTitleFilter(event.target.value);
    },
    [setTitleFilter],
  );

  const handleRefreshTitles = React.useCallback(() => {
    void refreshTitles();
  }, [refreshTitles]);

  const handleLibraryScan = React.useCallback(() => {
    void scanLibrary();
  }, [scanLibrary]);

  const handleDeleteCatalogTitle = React.useCallback(
    (title: TitleRecord) => {
      deleteCatalogTitle(title);
    },
    [deleteCatalogTitle],
  );

  return (
    <div className="space-y-4">
      {contentSettingsSection === "quality" ? (
        <QualitySettingsPanel
          contentSettingsLabel={contentSettingsLabel}
          mediaSettingsLoading={mediaSettingsLoading}
          mediaSettingsSaving={mediaSettingsSaving}
          qualityProfiles={qualityProfiles}
          qualityProfileEntries={qualityProfileEntries}
          qualityProfileParseError={qualityProfileParseError}
          categoryQualityProfileOverrides={categoryQualityProfileOverrides}
          activeQualityScopeId={activeQualityScopeId}
          globalQualityProfileId={globalQualityProfileId}
          qualityProfileInheritValue={qualityProfileInheritValue}
          toProfileOptions={toProfileOptions}
          handleQualityProfileOverrideChange={handleQualityProfileOverrideChange}
          onFacetPersonaSave={handleFacetPersonaSave}
          updateCategoryMediaProfileSettings={updateCategoryMediaProfileSettings}
        />
      ) : contentSettingsSection === "renaming" ? (
        <RenameSettingsPanel
          activeQualityScopeId={activeQualityScopeId}
          mediaSettingsLoading={mediaSettingsLoading}
          mediaSettingsSaving={mediaSettingsSaving}
          categoryRenameTemplates={categoryRenameTemplates}
          handleRenameTemplateChange={handleRenameTemplateChange}
          categoryRenameCollisionPolicies={categoryRenameCollisionPolicies}
          handleRenameCollisionPolicyChange={handleRenameCollisionPolicyChange}
          categoryRenameMissingMetadataPolicies={categoryRenameMissingMetadataPolicies}
          handleRenameMissingMetadataPolicyChange={handleRenameMissingMetadataPolicyChange}
          updateCategoryMediaProfileSettings={updateCategoryMediaProfileSettings}
        />
      ) : contentSettingsSection === "routing" ? (
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
      ) : contentSettingsSection === "settings" || contentSettingsSection === "general" ? (
        <div className="space-y-4">
          {view === "movies" || view === "series" ? (
            <MediaLibrarySettingsPanel
              settingsTitle={mediaLibrarySettingsTitle}
              pathLabel={mediaLibraryPathLabel}
              pathValue={mediaLibraryPathValue}
              pathPlaceholder={mediaLibraryPathPlaceholder}
              pathHelp={mediaLibraryPathHelp}
              pathRequired={view === "movies" || view === "series"}
              onPathChange={mediaLibraryPathChangeHandler}
              loading={mediaSettingsLoading}
              scanLoading={libraryScanLoading}
              scanSummary={libraryScanSummary}
              onScan={handleLibraryScan}
            />
          ) : null}
          <GeneralSettingsPanel
            activeQualityScopeId={activeQualityScopeId}
            mediaSettingsLoading={mediaSettingsLoading}
            mediaSettingsSaving={mediaSettingsSaving}
            categoryFillerPolicies={categoryFillerPolicies}
            handleFillerPolicyChange={handleFillerPolicyChange}
            categoryRecapPolicies={categoryRecapPolicies}
            handleRecapPolicyChange={handleRecapPolicyChange}
            categoryMonitorSpecials={categoryMonitorSpecials}
            handleMonitorSpecialsChange={handleMonitorSpecialsChange}
            categoryInterSeasonMovies={categoryInterSeasonMovies}
            handleInterSeasonMoviesChange={handleInterSeasonMoviesChange}
            nfoWriteOnImport={nfoWriteOnImport}
            handleNfoWriteChange={handleNfoWriteChange}
            plexmatchWriteOnImport={plexmatchWriteOnImport}
            handlePlexmatchWriteChange={handlePlexmatchWriteChange}
            updateCategoryMediaProfileSettings={updateCategoryMediaProfileSettings}
          />
        </div>
      ) : (
        view === "movies" || view === "series" || view === "anime" ? (
          <Card>
            <CardHeader>
              <CardTitle>{view === "movies" ? t("title.manageMovies") : view === "anime" ? t("nav.anime") : t("nav.series")}</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="mb-3 flex items-center gap-2">
                <Input
                  placeholder={t("title.filterPlaceholder")}
                  value={titleFilter}
                  onChange={handleTitleFilterChange}
                  className="flex-1"
                />
                <ToggleGroup
                  type="single"
                  value={viewMode}
                  onValueChange={(v) => {
                    if (v === "table" || v === "poster") setViewMode(v);
                  }}
                  size="sm"
                  aria-label={t("title.viewModeToggle")}
                >
                  <ToggleGroupItem value="table" size="sm" aria-label={t("title.viewModeTable")}>
                    <LayoutList className="h-4 w-4" />
                  </ToggleGroupItem>
                  <ToggleGroupItem value="poster" size="sm" aria-label={t("title.viewModePoster")}>
                    <LayoutGrid className="h-4 w-4" />
                  </ToggleGroupItem>
                </ToggleGroup>
                <Button variant="secondary" onClick={handleRefreshTitles} disabled={titleLoading}>
                  {titleLoading ? t("label.refreshing") : t("label.refresh")}
                </Button>
              </div>
              <p className="mb-2 text-sm text-muted-foreground">{titleStatus}</p>
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

                if (viewMode === "poster") {
                  return (
                    <PosterGrid
                      titles={monitoredTitles}
                      isMovieView={isMovieView}
                      resolvedProfileName={resolvedProfileName}
                      qualityProfiles={qualityProfiles}
                      onOpenOverview={onOpenOverview}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      isDeletingById={isDeletingCatalogTitleById}
                      overviewTargetView={overviewTargetView}
                    />
                  );
                }

                return (
                  <TitleTable
                    view={view}
                    titles={monitoredTitles}
                    titleLoading={titleLoading}
                    resolvedProfileName={resolvedProfileName}
                    qualityProfiles={qualityProfiles}
                    onOpenOverview={onOpenOverview}
                    onDelete={handleDeleteCatalogTitle}
                    onAutoQueue={queueExisting}
                    onToggleMonitored={toggleTitleMonitored}
                    onInteractiveSearch={runInteractiveSearchForTitle}
                    onQueueFromInteractive={queueExistingFromRelease}
                    isDeletingById={isDeletingCatalogTitleById}
                    isTogglingMonitoredById={isTogglingTitleMonitoredById}
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
            monitorSpecialsForQueue={monitorSpecialsForQueue}
            setMonitorSpecialsForQueue={setMonitorSpecialsForQueue}
            interSeasonMoviesForQueue={interSeasonMoviesForQueue}
            setInterSeasonMoviesForQueue={setInterSeasonMoviesForQueue}
            minAvailabilityForQueue={minAvailabilityForQueue}
            setMinAvailabilityForQueue={setMinAvailabilityForQueue}
            onAddSubmit={onAddSubmit}
            tvdbCandidates={tvdbCandidates}
            selectedTvdbId={selectedTvdbId}
            selectTvdbCandidate={selectTvdbCandidate}
            addTvdbCandidateToCatalog={addTvdbCandidateToCatalog}
            searchNzbForSelectedTvdb={searchNzbForSelectedTvdb}
            selectedTvdb={selectedTvdb}
            searchResults={searchResults}
            queueFromSearch={queueFromSearch}
            titleFilter={titleFilter}
            onTitleFilterChange={handleTitleFilterChange}
            onRefreshTitles={handleRefreshTitles}
            titleLoading={titleLoading}
            titleStatus={titleStatus}
            monitoredTitles={monitoredTitles}
            onOpenOverview={onOpenOverview}
            queueExisting={queueExisting}
          />
        )
      )}
    </div>
  );
}
