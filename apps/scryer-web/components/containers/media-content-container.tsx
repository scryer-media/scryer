import * as React from "react";
import { MediaContentView } from "@/components/views/media-content-view";
import {
  addTitleMutation,
  addTitleAndQueueMutation,
  queueExistingMutation,
  scanLibraryMutation,
  deleteTitleMutation,
  setTitleMonitoredMutation,
  updateRuleSetMutation,
  saveQualityProfileSettingsMutation,
} from "@/lib/graphql/mutations";
import {
  titlesQuery,
  titleListEntryQuery,
  deleteTitlePreviewQuery,
  ruleSetsQuery,
  routingPageInitQuery,
} from "@/lib/graphql/queries";
import {
  CATEGORY_SCOPE_MAP,
  QUALITY_PROFILE_INHERIT_VALUE,
  viewToFacet,
} from "@/lib/constants/settings";
import { useClient } from "urql";
import type { ContentSettingsSection, ViewId } from "@/components/root/types";
import {
  parseQualityProfileCatalogEntries,
  qualityProfileEntryToMutationInput,
  toProfileOptions,
} from "@/lib/utils/quality-profiles";
import { useDownloadClientRouting } from "@/lib/hooks/use-download-client-routing";
import { useIndexerRouting } from "@/lib/hooks/use-indexer-routing";
import { useMediaSettings } from "@/lib/hooks/use-media-settings";
import { useQueueFormState } from "@/lib/hooks/use-queue-form-state";
import { useTitleManagementState } from "@/lib/hooks/use-title-management-state";
import type { Release, TitleRecord, RuleSetRecord } from "@/lib/types";
import type { ScoringPersonaId } from "@/lib/types/quality-profiles";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useLibraryScanProgress } from "@/lib/context/library-scan-progress-context";
import { useSearchContext } from "@/lib/context/search-context";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import { useTitleListReactiveRefresh } from "@/lib/hooks/use-title-list-reactive-refresh";
import { toast } from "sonner";

type MediaContentContainerProps = {
  view: ViewId;
  contentSettingsSection: ContentSettingsSection;
  onOpenOverview: (targetView: ViewId, titleId: string) => void;
};

export const MediaContentContainer = React.memo(function MediaContentContainer({
  view,
  contentSettingsSection,
  onOpenOverview,
}: MediaContentContainerProps) {
  const searchState = useSearchContext();
  const {
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
    catalogChangeSignal,
  } = searchState;
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [titleDeleteTypedConfirmation, setTitleDeleteTypedConfirmation] =
    React.useState("");
  const activeFacet = viewToFacet[view as keyof typeof viewToFacet] ?? "movie";
  const { getActiveSession } = useLibraryScanProgress();
  const activeLibraryScanSession = getActiveSession(activeFacet);
  const activeQualityScopeId =
    CATEGORY_SCOPE_MAP[view as keyof typeof CATEGORY_SCOPE_MAP] ?? "movie";
  const isMediaView =
    view === "movies" || view === "series" || view === "anime";
  const shouldLoadCatalogTitles =
    isMediaView && contentSettingsSection === "overview";
  const shouldLoadMediaSettings = isMediaView;

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
  const titleDeletePreviewVariables = React.useMemo(
    () =>
      titleToDelete && deleteFilesOnDisk
        ? { input: { titleId: titleToDelete.id } }
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

  const {
    moviesPath,
    setMoviesPath,
    seriesPath,
    setSeriesPath,
    rootFolders,
    saveRootFolders,
    saveSetting,
    mediaSettingsLoading,
    mediaSettingsSaving,
    qualityProfiles,
    qualityProfileEntries,
    qualityProfilesText,
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
    categoryMonitorFillerMovies,
    setCategoryMonitorFillerMovies,
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
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
      : activeFacet === "tv"
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

  const refreshTitles = React.useCallback(async () => {
    setTitleLoading(true);
    setTitleStatus(t("title.loading"));
    try {
      const { data, error } = await client
        .query(titlesQuery, {
          facet: activeFacet,
        })
        .toPromise();
      if (error) throw error;
      const titles = (data?.titles || []) as TitleRecord[];
      setMonitoredTitles(titles);
      setTitleStatus(t("title.statusTemplate", { count: titles.length }));
    } catch (error) {
      setTitleStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    } finally {
      setTitleLoading(false);
    }
  }, [
    activeFacet,
    client,
    t,
    setMonitoredTitles,
    setTitleLoading,
    setTitleStatus,
  ]);

  const refreshTitleRecord = React.useCallback(
    async (titleId: string) => {
      const { data, error } = await client
        .query(
          titleListEntryQuery,
          { id: titleId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }

      const refreshedTitle = (data?.title ?? null) as TitleRecord | null;
      setMonitoredTitles((current) => {
        const existingIndex = current.findIndex((item) => item.id === titleId);
        if (!refreshedTitle) {
          if (existingIndex === -1) {
            return current;
          }
          return current.filter((item) => item.id !== titleId);
        }
        if (existingIndex === -1) {
          return [...current, refreshedTitle];
        }

        return current.map((item) =>
          item.id === titleId ? refreshedTitle : item,
        );
      });
    },
    [client, setMonitoredTitles],
  );

  React.useEffect(() => {
    if (!catalogChangeSignal || !shouldLoadCatalogTitles) {
      return;
    }
    void refreshTitles();
  }, [catalogChangeSignal, refreshTitles, shouldLoadCatalogTitles]);

  useTitleListReactiveRefresh({
    facet: activeFacet,
    pause: !shouldLoadCatalogTitles,
    onTitleUpdated: refreshTitleRecord,
  });

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
        await refreshTitles();
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
      refreshTitles,
      client,
      setGlobalStatus,
      t,
      seasonFoldersForQueue,
      setTitleNameForQueue,
    ],
  );

  const queueFromSearch = React.useCallback(
    async (release: Release) => {
      if (
        release.qualityProfileDecision &&
        release.qualityProfileDecision.allowed === false
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
      try {
        const { data, error } = await client
          .mutation(addTitleAndQueueMutation, {
            input: {
              name: queuedTitle,
              facet: queueFacet,
              monitored: monitoredForQueue,
              tags: [],
              options: {
                monitorType: queueMonitorType,
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
              externalIds: [
                ...(selectedTvdb
                  ? [{ source: "tvdb", value: String(selectedTvdb.tvdbId) }]
                  : []),
                ...(selectedTvdb?.imdbId
                  ? [{ source: "imdb", value: selectedTvdb.imdbId.trim() }]
                  : []),
              ],
              sourceHint,
              sourceKind: release.sourceKind ?? null,
              sourceTitle: release.title,
              ...(queueFacet === "movie"
                ? { minAvailability: minAvailabilityForQueue }
                : {}),
            },
          })
          .toPromise();
        if (error) throw error;
        const queuedName = data.addTitleAndQueueDownload.title.name;
        const queuedMessage = t("status.queueSuccess", { name: queuedName });
        setGlobalStatus(queuedMessage);
        await refreshTitles();
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
      refreshTitles,
      client,
      selectedTvdb,
      setGlobalStatus,
      titleNameForQueue,
      t,
      seasonFoldersForQueue,
      setTitleNameForQueue,
    ],
  );

  const queueExisting = React.useCallback(
    async (title: TitleRecord) => {
      const imdbId =
        title.imdbId?.trim() ||
        title.externalIds
          ?.find((externalId) => externalId.source.toLowerCase() === "imdb")
          ?.value?.trim() ||
        null;
      const tvdbId =
        title.externalIds
          ?.find((externalId) => externalId.source.toLowerCase() === "tvdb")
          ?.value?.trim() || null;
      const payload = await runSearch(title.name, title.facet, {
        imdbId,
        tvdbId,
        limit: title.facet === "movie" ? 50 : 15,
      });
      const top = payload.find(
        (result) => result.qualityProfileDecision?.allowed ?? true,
      );
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
        const { error } = await client
          .mutation(queueExistingMutation, {
            input: {
              titleId: title.id,
              release: {
                sourceHint,
                sourceKind: top.sourceKind ?? null,
                sourceTitle: top.title,
              },
            },
          })
          .toPromise();
        if (error) throw error;
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.queueFailed"),
        );
      }
    },
    [client, runSearch, setGlobalStatus, t],
  );

  const runInteractiveSearchForTitle = React.useCallback(
    async (title: TitleRecord) => {
      const imdbId =
        title.imdbId?.trim() ||
        title.externalIds
          ?.find((externalId) => externalId.source.toLowerCase() === "imdb")
          ?.value?.trim() ||
        null;
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
      if (
        release.qualityProfileDecision &&
        release.qualityProfileDecision.allowed === false
      ) {
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
        const { error } = await client
          .mutation(queueExistingMutation, {
            input: {
              titleId: title.id,
              release: {
                sourceHint,
                sourceKind: release.sourceKind ?? null,
                sourceTitle: release.title,
              },
            },
          })
          .toPromise();
        if (error) throw error;
        const queuedMessage = t("status.queuedLatest", { name: title.name });
        setGlobalStatus(queuedMessage);
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.queueFailed"),
        );
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
      const payload: {
        titleId: string;
        deleteFilesOnDisk?: boolean;
        previewFingerprint?: string;
        typedConfirmation?: string;
      } = {
        titleId,
      };

      if (deleteFilesOnDisk) {
        if (!titleDeletePreview) {
          throw new Error("Delete preview is not ready yet.");
        }
        payload.deleteFilesOnDisk = true;
        payload.previewFingerprint = titleDeletePreview.fingerprint;
        if (titleDeleteTypedConfirmation.trim()) {
          payload.typedConfirmation = titleDeleteTypedConfirmation.trim();
        }
      }

      const { error } = await client
        .mutation(deleteTitleMutation, {
          input: payload,
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.titleDeleted", { name: titleToDelete.name }));
      await refreshTitles();
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
    refreshTitles,
    client,
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

  const handleLibraryScan = React.useCallback(async () => {
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
    try {
      const { data, error } = await client
        .mutation(scanLibraryMutation, { facet: activeFacet })
        .toPromise();
      if (error) throw error;
      setLibraryScanSummary(data.scanLibrary);
      await refreshTitles();
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
    activeFacet,
    refreshTitles,
    client,
    setLibraryScanLoading,
    setLibraryScanNotice,
    setLibraryScanSummary,
    setGlobalStatus,
    t,
  ]);

  React.useEffect(() => {
    if (!titleStatus) {
      setTitleStatus(t("title.noManaged"));
    }
  }, [t, titleStatus, setTitleStatus]);

  const handleFacetPersonaSave = React.useCallback(
    async (persona: ScoringPersonaId | null) => {
      const entries = parseQualityProfileCatalogEntries(qualityProfilesText);
      const overrideId = categoryQualityProfileOverrides[activeQualityScopeId];
      const effectiveProfileId =
        !overrideId || overrideId === QUALITY_PROFILE_INHERIT_VALUE
          ? globalQualityProfileId
          : overrideId;
      const entry = entries.find((e) => e.id === effectiveProfileId);
      if (!entry) return;
      const nextOverrides = { ...entry.criteria.facet_persona_overrides };
      if (persona === null) {
        delete nextOverrides[activeQualityScopeId];
      } else {
        nextOverrides[activeQualityScopeId] = persona;
      }
      const updatedEntry = {
        ...entry,
        criteria: { ...entry.criteria, facet_persona_overrides: nextOverrides },
      };
      const nextEntries = entries.map((candidate) =>
        candidate.id === updatedEntry.id ? updatedEntry : candidate,
      );
      await client
        .mutation(saveQualityProfileSettingsMutation, {
          input: {
            profiles: nextEntries.map(qualityProfileEntryToMutationInput),
            globalProfileId: null,
            categorySelections: [],
            replaceExisting: true,
          },
        })
        .toPromise();
      await refreshMediaSettings();
    },
    [
      qualityProfilesText,
      categoryQualityProfileOverrides,
      activeQualityScopeId,
      globalQualityProfileId,
      client,
      refreshMediaSettings,
    ],
  );

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
      contentSettingsSection === "settings" ||
      contentSettingsSection === "general";
    const isRoutingSection = contentSettingsSection === "routing";

    if (shouldLoadCatalogTitles) {
      void refreshTitles();
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
    if (isGeneralSettingsSection) {
      void refreshRuleSets();
    }
  }, [
    activeQualityScopeId,
    client,
    contentSettingsSection,
    hydrateDownloadClientRouting,
    hydrateIndexerRouting,
    isMediaView,
    refreshRuleSets,
    refreshTitles,
    setGlobalStatus,
    shouldLoadCatalogTitles,
    t,
    view,
  ]);

  return (
    <>
      <MediaContentView
        state={{
          view,
          contentSettingsSection,
          contentSettingsLabel,
          moviesPath,
          setMoviesPath,
          seriesPath,
          setSeriesPath,
          rootFolders,
          saveRootFolders,
          saveSetting,
          mediaSettingsLoading,
          qualityProfiles: qualityProfiles,
          qualityProfileEntries,
          qualityProfilesText,
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
          categoryMonitorFillerMovies,
          setCategoryMonitorFillerMovies,
          nfoWriteOnImport,
          setNfoWriteOnImport,
          plexmatchWriteOnImport,
          setPlexmatchWriteOnImport,
          qualityProfileInheritValue: QUALITY_PROFILE_INHERIT_VALUE,
          toProfileOptions,
          handleFacetPersonaSave,
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
          monitoredTitles: titleFilter
            ? monitoredTitles.filter((t) =>
                t.name.toLowerCase().includes(titleFilter.toLowerCase()),
              )
            : monitoredTitles,
          queueExisting,
          toggleTitleMonitored,
          runInteractiveSearchForTitle,
          queueExistingFromRelease,
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
          libraryScanLoading:
            libraryScanLoading || Boolean(activeLibraryScanSession),
          libraryScanDisabled:
            libraryScanLoading || Boolean(activeLibraryScanSession),
          libraryScanNotice,
          libraryScanSummary,
          onOpenOverview,
          scanLibrary: handleLibraryScan,
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
    </>
  );
});
