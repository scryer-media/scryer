import { useCallback, useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useClient } from "urql";

import {
  adminSettingsQuery,
  indexerProviderTypesQuery,
} from "@/lib/graphql/queries";
import {
  saveAdminSettingsMutation,
  createDownloadClientMutation,
  testDownloadClientConnectionMutation,
  createIndexerMutation,
  testIndexerConnectionMutation,
  completeSetupMutation,
  previewExternalImportMutation,
  executeExternalImportMutation,
} from "@/lib/graphql/mutations";
import { QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY } from "@/lib/constants/settings";
import { DEFAULT_DOWNLOAD_CLIENT_DRAFT } from "@/lib/constants/download-clients";
import {
  buildDownloadClientBaseUrl,
  buildDownloadClientConfigJson,
} from "@/lib/utils/download-clients";
import {
  parseQualityProfileCatalogEntries,
  normalizeQualityProfilesForSave,
} from "@/lib/utils/quality-profiles";
import type { DownloadClientDraft } from "@/lib/types/download-clients";
import type { FacetQualityPrefs, ViewCategoryId } from "@/lib/types/quality-profiles";
import type { ExternalImportPreview, ExternalImportResult } from "@/lib/types/external-import";

import { SetupProgressBar } from "./setup-progress-bar";
import { SetupWelcomeView } from "./setup-welcome-view";
import { SetupPersonaView } from "./setup-persona-view";
import { SetupMediaPathsView } from "./setup-media-paths-view";
import { SetupDownloadClientView } from "./setup-download-client-view";
import { SetupIndexerView } from "./setup-indexer-view";
import { SetupSummaryView } from "./setup-summary-view";
import { SetupImportConnectView } from "./setup-import-connect-view";
import { SetupImportReviewView } from "./setup-import-review-view";

const FALLBACK_PROVIDER_OPTIONS = [
  { value: "nzbgeek", label: "NZBGeek Indexer" },
  { value: "newznab", label: "Newznab Indexer" },
];

interface SetupWizardContainerProps {
  t: (key: string) => string;
  isReentry?: boolean;
}

export function SetupWizardContainer({ t, isReentry }: SetupWizardContainerProps) {
  const client = useClient();
  const navigate = useNavigate();

  // ── Wizard path + step (URL-driven for browser back/forward) ──────
  const [searchParams, setSearchParams] = useSearchParams();
  const wizardPath: "fresh" | "import" = searchParams.get("path") === "import" ? "import" : "fresh";
  const currentStep = parseInt(searchParams.get("step") || "0", 10);

  const goToStep = useCallback(
    (step: number, path?: "fresh" | "import") => {
      const p = path ?? wizardPath;
      if (step === 0) {
        setSearchParams({});
      } else {
        setSearchParams({ path: p, step: String(step) });
      }
    },
    [wizardPath, setSearchParams],
  );

  // ── Step 1 (fresh) / Step 3 (import): Quality Preferences ─────────
  const [facetPrefs, setFacetPrefs] = useState<Record<ViewCategoryId, FacetQualityPrefs>>({
    movie:  { quality: "4k",    persona: "Balanced" },
    series: { quality: "4k",    persona: "Balanced" },
    anime:  { quality: "1080p", persona: "Balanced" },
  });
  const [personaSaving, setPersonaSaving] = useState(false);

  // ── Step 2 (fresh): Media Paths ─────────────────────────────────────
  const [moviesPath, setMoviesPath] = useState("/media/movies");
  const [seriesPath, setSeriesPath] = useState("/media/series");
  const [animePath, setAnimePath] = useState("");
  const [mediaPathsSaving, setMediaPathsSaving] = useState(false);
  const [mediaPathsError, setMediaPathsError] = useState<string | null>(null);

  // ── Step 3 (fresh): Download Client ─────────────────────────────────
  const [dcDraft, setDcDraft] = useState<DownloadClientDraft>({ ...DEFAULT_DOWNLOAD_CLIENT_DRAFT });
  const [dcTesting, setDcTesting] = useState(false);
  const [dcTestResult, setDcTestResult] = useState<"success" | "failed" | null>(null);
  const [dcSaving, setDcSaving] = useState(false);
  const [dcSaved, setDcSaved] = useState(false);
  const [dcError, setDcError] = useState<string | null>(null);

  // ── Step 4 (fresh): Indexer ─────────────────────────────────────────
  const [idxName, setIdxName] = useState("");
  const [idxProviderType, setIdxProviderType] = useState("");
  const [idxBaseUrl, setIdxBaseUrl] = useState("");
  const [idxApiKey, setIdxApiKey] = useState("");
  const [idxProviderOptions, setIdxProviderOptions] = useState<
    { value: string; label: string; defaultBaseUrl?: string }[]
  >([]);
  const [idxTesting, setIdxTesting] = useState(false);
  const [idxTestResult, setIdxTestResult] = useState<"success" | "failed" | null>(null);
  const [idxSaving, setIdxSaving] = useState(false);
  const [idxSaved, setIdxSaved] = useState(false);
  const [idxError, setIdxError] = useState<string | null>(null);

  // ── Import: Connect ─────────────────────────────────────────────────
  const [sonarrUrl, setSonarrUrl] = useState("");
  const [sonarrApiKey, setSonarrApiKey] = useState("");
  const [radarrUrl, setRadarrUrl] = useState("");
  const [radarrApiKey, setRadarrApiKey] = useState("");
  const [importConnecting, setImportConnecting] = useState(false);
  const [importConnectError, setImportConnectError] = useState<string | null>(null);

  // ── Import: Preview / Review ────────────────────────────────────────
  const [importPreview, setImportPreview] = useState<ExternalImportPreview | null>(null);
  const [selectedMoviesPath, setSelectedMoviesPath] = useState<string | null>(null);
  const [selectedSeriesPath, setSelectedSeriesPath] = useState<string | null>(null);
  const [selectedDcKeys, setSelectedDcKeys] = useState<Set<string>>(new Set());
  const [selectedIdxKeys, setSelectedIdxKeys] = useState<Set<string>>(new Set());
  const [selectedAnimePath, setSelectedAnimePath] = useState<string | null>(null);
  const [importExecuting, setImportExecuting] = useState(false);
  const [importExecuteError, setImportExecuteError] = useState<string | null>(null);
  const [importResult, setImportResult] = useState<ExternalImportResult | null>(null);

  // ── Summary / Finish ────────────────────────────────────────────────
  const [finishing, setFinishing] = useState(false);

  // ── Load indexer provider types on mount (for fresh path) ───────────
  useEffect(() => {
    (async () => {
      try {
        const { data } = await client.query(indexerProviderTypesQuery, {}).toPromise();
        if (data?.indexerProviderTypes?.length) {
          const options = data.indexerProviderTypes.map(
            (p: { providerType: string; name: string; defaultBaseUrl?: string }) => ({
              value: p.providerType,
              label: p.name,
              defaultBaseUrl: p.defaultBaseUrl || undefined,
            }),
          );
          setIdxProviderOptions(options);
          if (options.length > 0 && !idxProviderType) {
            setIdxProviderType(options[0].value);
          }
        } else {
          setIdxProviderOptions(FALLBACK_PROVIDER_OPTIONS);
          if (!idxProviderType) setIdxProviderType(FALLBACK_PROVIDER_OPTIONS[0].value);
        }
      } catch {
        setIdxProviderOptions(FALLBACK_PROVIDER_OPTIONS);
        if (!idxProviderType) setIdxProviderType(FALLBACK_PROVIDER_OPTIONS[0].value);
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ── Step labels per path ────────────────────────────────────────────
  const stepLabels =
    wizardPath === "import"
      ? [t("setup.stepConnect"), t("setup.stepReview"), t("setup.stepPersona"), t("setup.stepSummary")]
      : [t("setup.stepPersona"), t("setup.stepMediaPaths"), t("setup.stepDownloadClient"), t("setup.stepIndexer"), t("setup.stepSummary")];

  // ── Quality preferences save (per-facet) ────────────────────────────
  const saveFacetQualityPrefs = useCallback(
    async (nextStep: number) => {
      setPersonaSaving(true);
      try {
        // Load existing catalog to use as templates
        const { data } = await client.query(adminSettingsQuery, { scope: "system" }).toPromise();
        const catalogRecord = data?.adminSettings?.items?.find(
          (item: { keyName: string }) => item.keyName === QUALITY_PROFILE_CATALOG_KEY,
        );
        const rawCatalog =
          data?.adminSettings?.qualityProfiles ??
          (catalogRecord?.valueJson ?? catalogRecord?.effectiveValueJson ?? "[]");
        const existingProfiles = parseQualityProfileCatalogEntries(rawCatalog);

        // Build per-facet profiles from templates
        const WIZARD_FACETS: { facet: ViewCategoryId; name: string }[] = [
          { facet: "movie", name: "Movies" },
          { facet: "series", name: "Series" },
          { facet: "anime", name: "Anime" },
        ];
        const wizardProfileIds = WIZARD_FACETS.map((f) => `wizard-${f.facet}`);
        const builtinProfileIds = ["4k", "1080p"];
        const keptProfiles = existingProfiles.filter(
          (p) => !wizardProfileIds.includes(p.id) && !builtinProfileIds.includes(p.id),
        );

        for (const { facet, name } of WIZARD_FACETS) {
          const prefs = facetPrefs[facet];
          const template = existingProfiles.find((p) => p.id === prefs.quality);
          if (template) {
            const profileName = `${name} (${prefs.quality === "4k" ? "4K" : "1080P"})`;
            keptProfiles.push({
              id: `wizard-${facet}`,
              name: profileName,
              criteria: { ...template.criteria, scoring_persona: prefs.persona },
            });
          }
        }

        // Save updated catalog
        const catalogText = normalizeQualityProfilesForSave(JSON.stringify(keptProfiles));
        await client
          .mutation(saveAdminSettingsMutation, {
            input: {
              scope: "system",
              items: [{ keyName: QUALITY_PROFILE_CATALOG_KEY, value: catalogText }],
            },
          })
          .toPromise();

        // Set quality.profile_id per facet
        for (const { facet } of WIZARD_FACETS) {
          await client
            .mutation(saveAdminSettingsMutation, {
              input: {
                scope: "system",
                scopeId: facet,
                items: [{ keyName: QUALITY_PROFILE_ID_KEY, value: JSON.stringify(`wizard-${facet}`) }],
              },
            })
            .toPromise();
        }

        goToStep(nextStep);
      } catch (err) {
        console.warn("Failed to save quality preferences, continuing", err);
        goToStep(nextStep);
      } finally {
        setPersonaSaving(false);
      }
    },
    [client, facetPrefs, goToStep],
  );

  // ── Media paths save ────────────────────────────────────────────────
  const saveMediaPaths = useCallback(async () => {
    setMediaPathsSaving(true);
    setMediaPathsError(null);
    try {
      const items = [
        { keyName: "movies.path", value: JSON.stringify(moviesPath.trim()) },
        { keyName: "series.path", value: JSON.stringify(seriesPath.trim()) },
      ];
      const trimmedAnime = animePath.trim();
      if (trimmedAnime.length > 0) {
        items.push({ keyName: "anime.path", value: JSON.stringify(trimmedAnime) });
      }
      const { error } = await client
        .mutation(saveAdminSettingsMutation, {
          input: { scope: "media", items },
        })
        .toPromise();
      if (error) throw error;
      goToStep(3);
    } catch (err) {
      setMediaPathsError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setMediaPathsSaving(false);
    }
  }, [client, moviesPath, seriesPath, animePath, goToStep]);

  // ── Download client test ────────────────────────────────────────────
  const testDownloadClient = useCallback(async () => {
    setDcTesting(true);
    setDcTestResult(null);
    setDcError(null);
    try {
      const { data, error } = await client
        .mutation(testDownloadClientConnectionMutation, {
          input: {
            clientType: dcDraft.clientType,
            baseUrl: buildDownloadClientBaseUrl(dcDraft),
            configJson: buildDownloadClientConfigJson(dcDraft),
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testDownloadClientConnection) {
        setDcTestResult("success");
      } else {
        setDcTestResult("failed");
      }
    } catch {
      setDcTestResult("failed");
    } finally {
      setDcTesting(false);
    }
  }, [client, dcDraft]);

  // ── Download client save ────────────────────────────────────────────
  const saveDownloadClient = useCallback(async () => {
    setDcSaving(true);
    setDcError(null);
    try {
      const { error } = await client
        .mutation(createDownloadClientMutation, {
          input: {
            name: dcDraft.name.trim(),
            clientType: dcDraft.clientType,
            baseUrl: buildDownloadClientBaseUrl(dcDraft),
            configJson: buildDownloadClientConfigJson(dcDraft),
            isEnabled: true,
          },
        })
        .toPromise();
      if (error) throw error;
      setDcSaved(true);
    } catch (err) {
      setDcError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setDcSaving(false);
    }
  }, [client, dcDraft]);

  const handleDcTestAndSave = useCallback(async () => {
    setDcTesting(true);
    setDcTestResult(null);
    setDcError(null);
    try {
      const { data, error } = await client
        .mutation(testDownloadClientConnectionMutation, {
          input: {
            clientType: dcDraft.clientType,
            baseUrl: buildDownloadClientBaseUrl(dcDraft),
            configJson: buildDownloadClientConfigJson(dcDraft),
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testDownloadClientConnection) {
        setDcTestResult("success");
        setDcTesting(false);
        await saveDownloadClient();
      } else {
        setDcTestResult("failed");
        setDcTesting(false);
      }
    } catch {
      setDcTestResult("failed");
      setDcTesting(false);
    }
  }, [client, dcDraft, saveDownloadClient]);

  // ── Indexer test ────────────────────────────────────────────────────
  const testIndexer = useCallback(async () => {
    setIdxTesting(true);
    setIdxTestResult(null);
    setIdxError(null);
    const selectedProvider = idxProviderOptions.find((p) => p.value === idxProviderType);
    const effectiveBaseUrl = selectedProvider?.defaultBaseUrl || idxBaseUrl.trim();
    try {
      const { data, error } = await client
        .mutation(testIndexerConnectionMutation, {
          input: {
            name: idxName.trim(),
            providerType: idxProviderType,
            baseUrl: effectiveBaseUrl,
            apiKey: idxApiKey.trim() || undefined,
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testIndexerConnection) {
        setIdxTestResult("success");
      } else {
        setIdxTestResult("failed");
      }
    } catch {
      setIdxTestResult("failed");
    } finally {
      setIdxTesting(false);
    }
  }, [client, idxName, idxProviderType, idxBaseUrl, idxApiKey, idxProviderOptions]);

  // ── Indexer save ────────────────────────────────────────────────────
  const saveIndexer = useCallback(async () => {
    setIdxSaving(true);
    setIdxError(null);
    const selectedProvider = idxProviderOptions.find((p) => p.value === idxProviderType);
    const effectiveBaseUrl = selectedProvider?.defaultBaseUrl || idxBaseUrl.trim();
    try {
      const { error } = await client
        .mutation(createIndexerMutation, {
          input: {
            name: idxName.trim(),
            providerType: idxProviderType,
            baseUrl: effectiveBaseUrl,
            apiKey: idxApiKey.trim() || undefined,
            isEnabled: true,
            enableInteractiveSearch: true,
            enableAutoSearch: true,
          },
        })
        .toPromise();
      if (error) throw error;
      setIdxSaved(true);
    } catch (err) {
      setIdxError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setIdxSaving(false);
    }
  }, [client, idxName, idxProviderType, idxBaseUrl, idxApiKey, idxProviderOptions]);

  const handleIdxTestAndSave = useCallback(async () => {
    setIdxTesting(true);
    setIdxTestResult(null);
    setIdxError(null);
    const selectedProvider = idxProviderOptions.find((p) => p.value === idxProviderType);
    const effectiveBaseUrl = selectedProvider?.defaultBaseUrl || idxBaseUrl.trim();
    try {
      const { data, error } = await client
        .mutation(testIndexerConnectionMutation, {
          input: {
            name: idxName.trim(),
            providerType: idxProviderType,
            baseUrl: effectiveBaseUrl,
            apiKey: idxApiKey.trim() || undefined,
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testIndexerConnection) {
        setIdxTestResult("success");
        setIdxTesting(false);
        await saveIndexer();
      } else {
        setIdxTestResult("failed");
        setIdxTesting(false);
      }
    } catch {
      setIdxTestResult("failed");
      setIdxTesting(false);
    }
  }, [client, idxName, idxProviderType, idxBaseUrl, idxApiKey, idxProviderOptions, saveIndexer]);

  // ── Import: Connect & Scan ──────────────────────────────────────────
  const handleImportConnect = useCallback(async () => {
    setImportConnecting(true);
    setImportConnectError(null);
    try {
      const sonarr =
        sonarrUrl.trim() && sonarrApiKey.trim()
          ? { baseUrl: sonarrUrl.trim(), apiKey: sonarrApiKey.trim() }
          : undefined;
      const radarr =
        radarrUrl.trim() && radarrApiKey.trim()
          ? { baseUrl: radarrUrl.trim(), apiKey: radarrApiKey.trim() }
          : undefined;

      const { data, error } = await client
        .mutation(previewExternalImportMutation, {
          input: { sonarr: sonarr ?? null, radarr: radarr ?? null },
        })
        .toPromise();
      if (error) throw error;

      const preview: ExternalImportPreview = data.previewExternalImport;

      if (!preview.sonarrConnected && sonarr) {
        setImportConnectError("Could not connect to Sonarr. Check the URL and API key.");
        setImportConnecting(false);
        return;
      }
      if (!preview.radarrConnected && radarr) {
        setImportConnectError("Could not connect to Radarr. Check the URL and API key.");
        setImportConnecting(false);
        return;
      }

      setImportPreview(preview);

      // Auto-select all supported items
      const dcKeys = new Set<string>();
      for (const dc of preview.downloadClients) {
        if (dc.supported) dcKeys.add(dc.dedupKey);
      }
      setSelectedDcKeys(dcKeys);

      const idxKeys = new Set<string>();
      for (const idx of preview.indexers) {
        if (idx.supported) idxKeys.add(idx.dedupKey);
      }
      setSelectedIdxKeys(idxKeys);

      // Auto-select first root folder per facet
      const radarrFolders = preview.rootFolders.filter((f) => f.source === "radarr");
      if (radarrFolders.length > 0) setSelectedMoviesPath(radarrFolders[0].path);
      const sonarrFolders = preview.rootFolders.filter((f) => f.source === "sonarr");
      if (sonarrFolders.length > 0) setSelectedSeriesPath(sonarrFolders[0].path);

      // Auto-detect anime path if a Sonarr folder looks like anime
      if (sonarrFolders.length > 1) {
        const animeFolder = sonarrFolders.find((f) =>
          f.path.toLowerCase().includes("anime"),
        );
        if (animeFolder) setSelectedAnimePath(animeFolder.path);
      }

      goToStep(2);
    } catch (err) {
      setImportConnectError(err instanceof Error ? err.message : "Connection failed");
    } finally {
      setImportConnecting(false);
    }
  }, [client, sonarrUrl, sonarrApiKey, radarrUrl, radarrApiKey, goToStep]);

  // ── Import: Execute ─────────────────────────────────────────────────
  const handleImportExecute = useCallback(async () => {
    setImportExecuting(true);
    setImportExecuteError(null);
    try {
      const sonarr =
        sonarrUrl.trim() && sonarrApiKey.trim()
          ? { baseUrl: sonarrUrl.trim(), apiKey: sonarrApiKey.trim() }
          : undefined;
      const radarr =
        radarrUrl.trim() && radarrApiKey.trim()
          ? { baseUrl: radarrUrl.trim(), apiKey: radarrApiKey.trim() }
          : undefined;

      const { data, error } = await client
        .mutation(executeExternalImportMutation, {
          input: {
            sonarr: sonarr ?? null,
            radarr: radarr ?? null,
            selectedMoviesPath: selectedMoviesPath ?? null,
            selectedSeriesPath: selectedSeriesPath ?? null,
            selectedAnimePath: selectedAnimePath ?? null,
            selectedDownloadClientDedupKeys: [...selectedDcKeys],
            selectedIndexerDedupKeys: [...selectedIdxKeys],
          },
        })
        .toPromise();
      if (error) throw error;

      const result: ExternalImportResult = data.executeExternalImport;
      setImportResult(result);

      // Update paths for summary display
      if (selectedMoviesPath) setMoviesPath(selectedMoviesPath);
      if (selectedSeriesPath) setSeriesPath(selectedSeriesPath);

      if (result.errors.length > 0) {
        setImportExecuteError(result.errors.join("; "));
      }

      goToStep(3); // → persona
    } catch (err) {
      setImportExecuteError(err instanceof Error ? err.message : "Import failed");
    } finally {
      setImportExecuting(false);
    }
  }, [
    client,
    sonarrUrl,
    sonarrApiKey,
    radarrUrl,
    radarrApiKey,
    selectedMoviesPath,
    selectedSeriesPath,
    selectedAnimePath,
    selectedDcKeys,
    selectedIdxKeys,
    goToStep,
  ]);

  // ── Complete setup ──────────────────────────────────────────────────
  const finishSetup = useCallback(async () => {
    setFinishing(true);
    try {
      await client.mutation(completeSetupMutation, {}).toPromise();
      navigate(isReentry ? "/settings" : "/movies", { replace: true });
    } catch {
      navigate(isReentry ? "/settings" : "/movies", { replace: true });
    }
  }, [client, navigate, isReentry]);

  // ── Toggle helpers for import review ────────────────────────────────
  const toggleDcKey = useCallback((key: string) => {
    setSelectedDcKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const toggleIdxKey = useCallback((key: string) => {
    setSelectedIdxKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  // ── Render ──────────────────────────────────────────────────────────

  // Step mapping for progress bar (step 0 = welcome, not shown in bar)
  const progressStep = currentStep > 0 ? currentStep - 1 : -1;

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-2xl flex-col items-center justify-center px-4 py-10">
      {currentStep > 0 && (
        <div className="mb-8 w-full">
          <SetupProgressBar currentStep={progressStep} stepLabels={stepLabels} />
        </div>
      )}

      {/* ── Step 0: Welcome (shared) ─────────────────────────────────── */}
      {currentStep === 0 && (
        <SetupWelcomeView
          t={t}
          onFreshSetup={() => goToStep(1, "fresh")}
          onImportSetup={() => goToStep(1, "import")}
          onSkip={finishSetup}
          skipping={finishing}
        />
      )}

      {/* ════════════════════════════════════════════════════════════════ */}
      {/* FRESH PATH                                                      */}
      {/* ════════════════════════════════════════════════════════════════ */}

      {currentStep === 1 && wizardPath === "fresh" && (
        <SetupPersonaView
          t={t}
          facetPrefs={facetPrefs}
          onFacetPrefsChange={(facet, prefs) =>
            setFacetPrefs((prev) => ({ ...prev, [facet]: prefs }))
          }
          onNext={() => saveFacetQualityPrefs(2)}
          onBack={() => goToStep(0)}
          onSkip={() => goToStep(2)}
          saving={personaSaving}
        />
      )}

      {currentStep === 2 && wizardPath === "fresh" && (
        <SetupMediaPathsView
          t={t}
          moviesPath={moviesPath}
          seriesPath={seriesPath}
          animePath={animePath}
          onMoviesPathChange={setMoviesPath}
          onSeriesPathChange={setSeriesPath}
          onAnimePathChange={setAnimePath}
          onNext={saveMediaPaths}
          onBack={() => goToStep(1)}
          onSkip={() => goToStep(3)}
          saving={mediaPathsSaving}
          error={mediaPathsError}
        />
      )}

      {currentStep === 3 && wizardPath === "fresh" && (
        <SetupDownloadClientView
          t={t}
          draft={dcDraft}
          onDraftChange={(updates) =>
            setDcDraft((prev) => ({ ...prev, ...updates }))
          }
          onTestConnection={dcSaved ? testDownloadClient : handleDcTestAndSave}
          onNext={() => goToStep(4)}
          onBack={() => goToStep(2)}
          onSkip={() => goToStep(4)}
          testing={dcTesting}
          testResult={dcTestResult}
          saving={dcSaving}
          saved={dcSaved}
          error={dcError}
        />
      )}

      {currentStep === 4 && wizardPath === "fresh" && (
        <SetupIndexerView
          t={t}
          name={idxName}
          providerType={idxProviderType}
          baseUrl={idxBaseUrl}
          apiKey={idxApiKey}
          providerOptions={idxProviderOptions}
          onNameChange={setIdxName}
          onProviderTypeChange={setIdxProviderType}
          onBaseUrlChange={setIdxBaseUrl}
          onApiKeyChange={setIdxApiKey}
          onTestConnection={idxSaved ? testIndexer : handleIdxTestAndSave}
          onNext={() => goToStep(5)}
          onBack={() => goToStep(3)}
          onSkip={() => goToStep(5)}
          testing={idxTesting}
          testResult={idxTestResult}
          saving={idxSaving}
          saved={idxSaved}
          error={idxError}
        />
      )}

      {currentStep === 5 && wizardPath === "fresh" && (
        <SetupSummaryView
          t={t}
          facetPrefs={facetPrefs}
          moviesPath={moviesPath}
          seriesPath={seriesPath}
          animePath={animePath}
          downloadClientName={dcDraft.name || dcDraft.clientType}
          indexerName={idxName || idxProviderType}
          onFinish={finishSetup}
          onBack={() => goToStep(4)}
          finishing={finishing}
        />
      )}

      {/* ════════════════════════════════════════════════════════════════ */}
      {/* IMPORT PATH                                                     */}
      {/* ════════════════════════════════════════════════════════════════ */}

      {currentStep === 1 && wizardPath === "import" && (
        <SetupImportConnectView
          t={t}
          sonarrUrl={sonarrUrl}
          sonarrApiKey={sonarrApiKey}
          radarrUrl={radarrUrl}
          radarrApiKey={radarrApiKey}
          onSonarrUrlChange={setSonarrUrl}
          onSonarrApiKeyChange={setSonarrApiKey}
          onRadarrUrlChange={setRadarrUrl}
          onRadarrApiKeyChange={setRadarrApiKey}
          onConnect={handleImportConnect}
          onBack={() => goToStep(0)}
          connecting={importConnecting}
          error={importConnectError}
        />
      )}

      {currentStep === 2 && wizardPath === "import" && importPreview && (
        <SetupImportReviewView
          t={t}
          preview={importPreview}
          selectedMoviesPath={selectedMoviesPath}
          selectedSeriesPath={selectedSeriesPath}
          selectedAnimePath={selectedAnimePath}
          selectedDcKeys={selectedDcKeys}
          selectedIdxKeys={selectedIdxKeys}
          onSelectMoviesPath={setSelectedMoviesPath}
          onSelectSeriesPath={setSelectedSeriesPath}
          onSelectAnimePath={setSelectedAnimePath}
          onToggleDc={toggleDcKey}
          onToggleIdx={toggleIdxKey}
          onImport={handleImportExecute}
          onBack={() => goToStep(1)}
          importing={importExecuting}
          error={importExecuteError}
        />
      )}

      {currentStep === 3 && wizardPath === "import" && (
        <SetupPersonaView
          t={t}
          facetPrefs={facetPrefs}
          onFacetPrefsChange={(facet, prefs) =>
            setFacetPrefs((prev) => ({ ...prev, [facet]: prefs }))
          }
          onNext={() => saveFacetQualityPrefs(4)}
          onBack={() => goToStep(2)}
          saving={personaSaving}
        />
      )}

      {currentStep === 4 && wizardPath === "import" && (
        <SetupSummaryView
          t={t}
          facetPrefs={facetPrefs}
          moviesPath={moviesPath}
          seriesPath={seriesPath}
          animePath={selectedAnimePath ?? undefined}
          downloadClientName=""
          indexerName=""
          importedDcCount={importResult?.downloadClientsCreated}
          importedIdxCount={importResult?.indexersCreated}
          onFinish={finishSetup}
          onBack={() => goToStep(3)}
          finishing={finishing}
        />
      )}
    </div>
  );
}
