
import * as React from "react";
import { MediaContentView } from "@/components/views/media-content-view";
import {
  addTitleMutation,
  addTitleAndQueueMutation,
  queueExistingMutation,
  scanMovieLibraryMutation,
  deleteTitleMutation,
  updateRuleSetMutation,
} from "@/lib/graphql/mutations";
import { titlesQuery, ruleSetsQuery } from "@/lib/graphql/queries";
import {
  CATEGORY_SCOPE_MAP,
  QUALITY_PROFILE_INHERIT_VALUE,
  viewToFacet,
} from "@/lib/constants/settings";
import { useClient } from "urql";
import type { ContentSettingsSection, ViewId } from "@/components/root/types";
import { toProfileOptions } from "@/lib/utils/quality-profiles";
import { useDownloadClientRouting } from "@/lib/hooks/use-download-client-routing";
import { useIndexerRouting } from "@/lib/hooks/use-indexer-routing";
import { useMediaSettings } from "@/lib/hooks/use-media-settings";
import { useQueueFormState } from "@/lib/hooks/use-queue-form-state";
import { useTitleManagementState } from "@/lib/hooks/use-title-management-state";
import type {
  Release,
  Facet,
  TitleRecord,
  RuleSetRecord,
} from "@/lib/types";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Translate } from "@/components/root/types";

type NzbSearchOptions = {
  imdbId?: string | null;
  tvdbId?: string | null;
  limit?: number;
};
type MediaContentContainerProps = {
  t: Translate;
  view: ViewId;
  contentSettingsSection: ContentSettingsSection;
  setGlobalStatus: (status: string) => void;
  queueFacet: Facet;
  setQueueFacet: (value: Facet) => void;
  runTvdbSearch: (query: string) => Promise<MetadataTvdbSearchItem[]>;
  catalogChangeSignal?: number;
  runSearch: (
    query: string,
    category?: string | null,
    options?: NzbSearchOptions,
  ) => Promise<Release[]>;
  searchNzbForSelectedTvdb: () => Promise<void>;
  selectedTvdb: MetadataTvdbSearchItem | null;
  tvdbCandidates: MetadataTvdbSearchItem[];
  selectedTvdbId: string | null;
  selectTvdbCandidate: (candidate: MetadataTvdbSearchItem) => void;
  searchResults: Release[];
  onOpenOverview: (targetView: ViewId, titleId: string) => void;
};

export const MediaContentContainer = React.memo(function MediaContentContainer({
  t,
  view,
  contentSettingsSection,
  setGlobalStatus,
  queueFacet,
  setQueueFacet,
  runTvdbSearch,
  runSearch,
  searchNzbForSelectedTvdb,
  selectedTvdb,
  tvdbCandidates,
  selectedTvdbId,
  selectTvdbCandidate,
  searchResults,
  onOpenOverview,
  catalogChangeSignal,
}: MediaContentContainerProps) {
  const client = useClient();
  const activeFacet = viewToFacet[view as keyof typeof viewToFacet] ?? "movie";
  const activeQualityScopeId = CATEGORY_SCOPE_MAP[view as keyof typeof CATEGORY_SCOPE_MAP] ?? "movie";

  const {
    titleNameForQueue, setTitleNameForQueue,
    monitoredForQueue, setMonitoredForQueue,
    seasonFoldersForQueue, setSeasonFoldersForQueue,
    monitorSpecialsForQueue, setMonitorSpecialsForQueue,
    interSeasonMoviesForQueue, setInterSeasonMoviesForQueue,
    preferredSubGroupForQueue, setPreferredSubGroupForQueue,
    minAvailabilityForQueue, setMinAvailabilityForQueue,
  } = useQueueFormState();

  const {
    titleFilter, setTitleFilter,
    monitoredTitles, setMonitoredTitles,
    titleLoading, setTitleLoading,
    titleStatus, setTitleStatus,
    titleToDelete, setTitleToDelete,
    deleteFilesOnDisk, setDeleteFilesOnDisk,
    deleteTitleLoadingById, setDeleteTitleLoadingById,
    libraryScanLoading, setLibraryScanLoading,
    libraryScanSummary, setLibraryScanSummary,
  } = useTitleManagementState();

  const {
    moviesPath,
    setMoviesPath,
    seriesPath,
    setSeriesPath,
    mediaSettingsLoading,
    mediaSettingsSaving,
    qualityProfiles,
    qualityProfileParseError,
    globalQualityProfileId,
    categoryQualityProfileOverrides,
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
    categoryPreferredSubGroup,
    setCategoryPreferredSubGroup,
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
    updateCategoryMediaProfileSettings,
    refreshMediaSettings,
  } = useMediaSettings({
    activeQualityScopeId,
    setGlobalStatus,
    t,
    view,
  });

  const contentSettingsLabel =
    view === "movies"
      ? t("settings.moviesSettings")
      : view === "series"
        ? t("settings.seriesSettings")
        : t("settings.animeSettings");
  const {
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving,
    refreshDownloadClientRouting,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
    saveDownloadClientRouting,
  } = useDownloadClientRouting({
    activeQualityScopeId,
    setGlobalStatus,
    t,
  });
  const {
    indexers,
    activeScopeRouting: activeScopeIndexerRouting,
    activeScopeRoutingOrder: activeScopeIndexerRoutingOrder,
    indexerRoutingLoading,
    indexerRoutingSaving,
    refreshIndexerRouting,
    setIndexerEnabledForScope,
    updateIndexerRoutingForScope,
    moveIndexerInScope,
  } = useIndexerRouting({
    activeQualityScopeId,
    setGlobalStatus,
    t,
  });

  const [ruleSets, setRuleSets] = React.useState<RuleSetRecord[]>([]);
  const [rulesLoading, setRulesLoading] = React.useState(true);
  const [rulesSaving, setRulesSaving] = React.useState(false);

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
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setRulesSaving(false);
      }
    },
    [activeFacet, client, refreshRuleSets, ruleSets, setGlobalStatus, t],
  );

  React.useEffect(() => {
    setMonitorSpecialsForQueue(categoryMonitorSpecials.anime !== "false");
    setInterSeasonMoviesForQueue(categoryInterSeasonMovies.anime !== "false");
    setPreferredSubGroupForQueue(categoryPreferredSubGroup.anime);
  }, [categoryMonitorSpecials.anime, categoryInterSeasonMovies.anime, categoryPreferredSubGroup.anime]);

  const refreshTitles = React.useCallback(async () => {
    setTitleLoading(true);
    setTitleStatus(t("title.loading"));
    try {
      const { data, error } = await client.query(titlesQuery, {
        facet: activeFacet,
        query: titleFilter || undefined,
      }).toPromise();
      if (error) throw error;
      const titles = (data?.titles || []) as TitleRecord[];
      setMonitoredTitles(titles);
      setTitleStatus(t("title.statusTemplate", { count: titles.length }));
    } catch (error) {
      setTitleStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setTitleLoading(false);
    }
  }, [activeFacet, client, t, titleFilter]);

  React.useEffect(() => {
    if (!catalogChangeSignal) {
      return;
    }
    void refreshTitles();
  }, [catalogChangeSignal, refreshTitles]);

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

      const tvdbResults = await runTvdbSearch(titleNameForQueue.trim());
      if (!tvdbResults.length) {
        return;
      }
      setGlobalStatus(t("status.tvdbQueueTip"));
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

      const tvdbId = String(candidate.tvdb_id).trim();
      const imdbId = candidate.imdb_id?.trim();
      const externalIds = [
        { source: "tvdb", value: tvdbId },
        ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
      ];

      const monitorType = monitoredForQueue ? "allEpisodes" : "none";
      const tags = [
        `scryer:monitor-type:${monitorType}`,
        ...(queueFacet !== "movie"
          ? [`scryer:season-folder:${seasonFoldersForQueue ? "enabled" : "disabled"}`]
          : []),
        ...(queueFacet === "anime"
          ? [
              `scryer:monitor-specials:${monitorSpecialsForQueue ? "true" : "false"}`,
              `scryer:inter-season-movies:${interSeasonMoviesForQueue ? "true" : "false"}`,
              ...(preferredSubGroupForQueue.trim()
                ? [`scryer:preferred-sub-group:${preferredSubGroupForQueue.trim()}`]
                : []),
            ]
          : []),
      ];

      try {
        const { data, error } = await client.mutation(addTitleMutation, {
          input: {
            name,
            facet: queueFacet,
            monitored: monitoredForQueue,
            tags,
            externalIds,
            ...(queueFacet === "movie" ? { minAvailability: minAvailabilityForQueue } : {}),
          },
        }).toPromise();
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
        await refreshTitles();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [interSeasonMoviesForQueue, minAvailabilityForQueue, monitorSpecialsForQueue, monitoredForQueue, preferredSubGroupForQueue, queueFacet, refreshTitles, client, setGlobalStatus, t],
  );

  const queueFromSearch = React.useCallback(
    async (release: Release) => {
      if (
        release.qualityProfileDecision && release.qualityProfileDecision.allowed === false
      ) {
        const reason =
          release.qualityProfileDecision.blockCodes.join(", ") ||
          t("settings.qualityProfileUnknown", { id: t("label.default") });
        setGlobalStatus(t("status.qualityProfileBlocked", { reason }));
        return;
      }

      const queuedTitle = selectedTvdb?.name || release.title;
      if (!titleNameForQueue) {
        setTitleNameForQueue(queuedTitle);
      }
      const sourceHint = release.downloadUrl || release.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noReleaseSource"));
        return;
      }

      const queueMonitorType = monitoredForQueue ? "allEpisodes" : "none";
      const queueTags = [
        `scryer:monitor-type:${queueMonitorType}`,
        ...(queueFacet !== "movie"
          ? [`scryer:season-folder:${seasonFoldersForQueue ? "enabled" : "disabled"}`]
          : []),
        ...(queueFacet === "anime"
          ? [
              `scryer:monitor-specials:${monitorSpecialsForQueue ? "true" : "false"}`,
              `scryer:inter-season-movies:${interSeasonMoviesForQueue ? "true" : "false"}`,
              ...(preferredSubGroupForQueue.trim()
                ? [`scryer:preferred-sub-group:${preferredSubGroupForQueue.trim()}`]
                : []),
            ]
          : []),
      ];

      try {
        const { data, error } = await client.mutation(
          addTitleAndQueueMutation,
          {
            input: {
              name: queuedTitle,
              facet: queueFacet,
              monitored: monitoredForQueue,
              tags: queueTags,
              externalIds: [
                ...(selectedTvdb
                  ? [{ source: "tvdb", value: String(selectedTvdb.tvdb_id) }]
                  : []),
                ...(selectedTvdb?.imdb_id
                  ? [{ source: "imdb", value: selectedTvdb.imdb_id.trim() }]
                  : []),
              ],
              sourceHint,
              sourceTitle: release.title,
              ...(queueFacet === "movie" ? { minAvailability: minAvailabilityForQueue } : {}),
            },
          },
        ).toPromise();
        if (error) throw error;
        const queuedName = data.addTitleAndQueueDownload.title.name;
        const queuedMessage = t("status.queueSuccess", { name: queuedName });
        setGlobalStatus(queuedMessage);
        await refreshTitles();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [
      interSeasonMoviesForQueue,
      minAvailabilityForQueue,
      monitorSpecialsForQueue,
      monitoredForQueue,
      preferredSubGroupForQueue,
      queueFacet,
      refreshTitles,
      client,
      selectedTvdb,
      setGlobalStatus,
      titleNameForQueue,
      t,
    ],
  );

  const queueExisting = React.useCallback(
    async (title: TitleRecord) => {
      const imdbId =
        title.externalIds
          ?.find((externalId) => externalId.source.toLowerCase() === "imdb")
          ?.value?.trim() || null;
      const tvdbId =
        title.externalIds
          ?.find((externalId) => externalId.source.toLowerCase() === "tvdb")
          ?.value?.trim() || null;
      const payload = await runSearch(title.name, title.facet, {
        imdbId,
        tvdbId,
        limit: title.facet === "movie" ? 50 : 15,
      });
      const top = payload.find((result) => result.qualityProfileDecision?.allowed ?? true);
      if (!top) {
        setGlobalStatus(t("status.noReleaseForTitle", { name: title.name }));
        return;
      }
      const sourceHint = top.downloadUrl || top.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noSource", { name: title.name }));
        return;
      }

      try {
        const { error } = await client.mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            sourceHint,
            sourceTitle: top.title,
          },
        }).toPromise();
        if (error) throw error;
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [client, runSearch, setGlobalStatus, t],
  );

  const runInteractiveSearchForTitle = React.useCallback(
    async (title: TitleRecord) => {
      const imdbId =
        title.externalIds
          ?.find((externalId) => externalId.source.toLowerCase() === "imdb")
          ?.value?.trim() || null;
      const tvdbId =
        title.externalIds
          ?.find((externalId) => externalId.source.toLowerCase() === "tvdb")
          ?.value?.trim() || null;

      return runSearch(title.name, title.facet, {
        imdbId,
        tvdbId,
        limit: title.facet === "movie" ? 50 : 15,
      });
    },
    [runSearch],
  );

  const queueExistingFromRelease = React.useCallback(
    async (title: TitleRecord, release: Release) => {
      if (release.qualityProfileDecision && release.qualityProfileDecision.allowed === false) {
        const reason =
          release.qualityProfileDecision.blockCodes.join(", ") ||
          t("settings.qualityProfileUnknown", { id: t("label.default") });
        setGlobalStatus(t("status.qualityProfileBlocked", { reason }));
        return;
      }

      const sourceHint = release.downloadUrl || release.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noSource", { name: title.name }));
        return;
      }

      try {
        const { error } = await client.mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            sourceHint,
            sourceTitle: release.title,
          },
        }).toPromise();
        if (error) throw error;
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      }
    },
    [client, setGlobalStatus, t],
  );

  const requestDeleteTitle = React.useCallback((title: TitleRecord) => {
    setTitleToDelete(title);
    setDeleteFilesOnDisk(false);
  }, []);

  const closeDeleteTitleDialog = React.useCallback(() => {
    setTitleToDelete(null);
    setDeleteFilesOnDisk(false);
  }, []);

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
      const payload: { titleId: string; deleteFilesOnDisk?: boolean } = {
        titleId,
      };

      if (deleteFilesOnDisk) {
        payload.deleteFilesOnDisk = true;
      }

      const { error } = await client.mutation(deleteTitleMutation, {
        input: payload,
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.titleDeleted", { name: titleToDelete.name }));
      await refreshTitles();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setDeleteTitleLoadingById((previous) => {
        const next = { ...previous };
        delete next[titleId];
        return next;
      });
      closeDeleteTitleDialog();
    }
  }, [closeDeleteTitleDialog, deleteFilesOnDisk, refreshTitles, client, t, titleToDelete, setGlobalStatus]);

  const handleLibraryScan = React.useCallback(async () => {
    setLibraryScanLoading(true);
    setGlobalStatus(t("settings.libraryScanRunning"));
    try {
      const { data, error } = await client.mutation(scanMovieLibraryMutation, {}).toPromise();
      if (error) throw error;
      setLibraryScanSummary(data.scanMovieLibrary);
      setGlobalStatus(
        t("settings.libraryScanSuccess", {
          imported: data.scanMovieLibrary.imported,
          skipped: data.scanMovieLibrary.skipped,
          unmatched: data.scanMovieLibrary.unmatched,
        }),
      );
      await refreshTitles();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("settings.libraryScanFailed"));
    } finally {
      setLibraryScanLoading(false);
    }
  }, [refreshTitles, client, setGlobalStatus, t]);

  React.useEffect(() => {
    if (!titleStatus) {
      setTitleStatus(t("title.noManaged"));
    }
  }, [t, titleStatus]);

  React.useEffect(() => {
    if (view !== "movies" && view !== "series" && view !== "anime") {
      return;
    }

    void refreshTitles();
    if (contentSettingsSection === "settings") {
      void refreshMediaSettings();
      void refreshDownloadClientRouting();
      void refreshIndexerRouting();
      void refreshRuleSets();
    }
  }, [
    contentSettingsSection,
    refreshDownloadClientRouting,
    refreshIndexerRouting,
    refreshMediaSettings,
    refreshRuleSets,
    refreshTitles,
    view,
  ]);

  return (
    <>
      <MediaContentView
        state={{
          t,
          view,
          contentSettingsSection,
          contentSettingsLabel,
          moviesPath,
          setMoviesPath,
          seriesPath,
          setSeriesPath,
          mediaSettingsLoading,
          qualityProfiles: qualityProfiles,
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
          categoryPreferredSubGroup,
          setCategoryPreferredSubGroup,
          nfoWriteOnImport,
          setNfoWriteOnImport,
          plexmatchWriteOnImport,
          setPlexmatchWriteOnImport,
          qualityProfileInheritValue: QUALITY_PROFILE_INHERIT_VALUE,
          toProfileOptions,
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
          monitorSpecialsForQueue,
          setMonitorSpecialsForQueue,
          interSeasonMoviesForQueue,
          setInterSeasonMoviesForQueue,
          preferredSubGroupForQueue,
          setPreferredSubGroupForQueue,
          minAvailabilityForQueue,
          setMinAvailabilityForQueue,
          selectedTvdb,
          tvdbCandidates,
          selectedTvdbId,
          selectTvdbCandidate,
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
          runInteractiveSearchForTitle,
          queueExistingFromRelease,
          downloadClients,
          activeScopeRouting,
          activeScopeRoutingOrder,
          downloadClientRoutingLoading,
          downloadClientRoutingSaving,
          updateDownloadClientRoutingForScope,
          moveDownloadClientInScope,
          saveDownloadClientRouting,
          indexers,
          activeScopeIndexerRouting,
          activeScopeIndexerRoutingOrder,
          indexerRoutingLoading,
          indexerRoutingSaving,
          setIndexerEnabledForScope,
          updateIndexerRoutingForScope,
          moveIndexerInScope,
          ruleSets,
          rulesLoading,
          rulesSaving,
          onToggleRuleFacet,
          libraryScanLoading,
          libraryScanSummary,
          onOpenOverview,
          scanMovieLibrary: handleLibraryScan,
          deleteCatalogTitle: requestDeleteTitle,
          isDeletingCatalogTitleById: deleteTitleLoadingById,
        }}
      />
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
        isBusy={titleToDelete !== null ? !!deleteTitleLoadingById[titleToDelete.id] : false}
        onConfirm={confirmDeleteTitle}
        onCancel={closeDeleteTitleDialog}
      >
        <label className="flex items-center gap-2">
          <Checkbox
            checked={deleteFilesOnDisk}
            onCheckedChange={(checked) => setDeleteFilesOnDisk(checked === true)}
            disabled={titleToDelete !== null ? !!deleteTitleLoadingById[titleToDelete.id] : false}
          />
          <span className="text-xs text-card-foreground">{t("title.deleteFilesOnDisk")}</span>
        </label>
      </ConfirmDialog>
    </>
  );
});
